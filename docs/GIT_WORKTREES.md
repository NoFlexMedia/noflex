# Git Worktrees

This guide shows you **why** you'd run NoFlex as a set of git worktrees instead of a single checkout (or several clones), and **how** to set it up from scratch — including a shared compilation cache that makes builds fast across worktrees, and a hook that keeps your local config in sync everywhere.

If you've ever wished you could have several branches open and building at the same time without paying for it in disk, compile time, or duplicated local config, this is the setup for you.

---

## Why worktrees (instead of one checkout, or many clones)

The two ways people usually juggle multiple branches both hurt:

- **One checkout, constant branch-switching.** You stash, switch, rebuild, and lose your editor state and the server you had running. Switching back is just as costly, and Cargo rebuilds everything that changed in between.
- **Several full clones.** Each clone is its own git database (disk), its own build cache (a cold Wasmtime-sized build every time), and its own local config to maintain by hand.

**Linked worktrees** give you the best of both. You get multiple working directories, each on its own branch, but they all share **one git object store** (adding a worktree is instant and nearly free on disk) and — with the cache setup below — **one compilation cache** and **one set of local config**. You can have `main`, `feature-a`, and `feature-b` open side by side, each building and running independently, while:

- a third-party crate compiled in one worktree is a **cache hit** in the others, and
- your local config is **already in place** the moment you create a new worktree.

---

## The pattern you're building

Everything lives under one parent directory. One bare repo holds the git database; each worktree is a sibling directory that points back into it:

```
noflex/
├── .bare/                 # the bare repo: git objects, config, hooks (no working tree)
│   └── hooks/
│       └── post-checkout  # the shared sync hook (see "Keep local config in sync")
├── .git                   # a FILE (not a dir): "gitdir: noflex/.bare"
├── .cache/                # shared, gitignored build cache (sccache)
├── main/                  # worktree — the "source of truth" slot
├── feature-a/             # worktree
└── feature-b/             # worktree
```

Two things make this work, and both are non-obvious:

1. **`.bare/` is a bare clone** — no working tree, just the git database. All worktrees share it.
2. **The parent's `.git` is a _file_, not a directory.** Its contents are `gitdir: noflex/.bare`. This makes the parent behave like a git repo, so `git worktree add …` run from there lands new worktrees as siblings.

Each worktree then has its own `.git` _file_ pointing into the bare repo:

```
noflex/main/.git       → gitdir: noflex/.bare/worktrees/main
noflex/feature-a/.git  → gitdir: noflex/.bare/worktrees/feature-a
```

**A nice side effect:** with `core.hooksPath` unset, git resolves hooks against the shared bare dir for **every** worktree. One hook in `.bare/hooks/` fires for all of them.

---

## Set it up

Run this from the directory that will hold the bare repo and all the worktrees:

```sh
mkdir noflex && cd noflex

# 1. Bare clone the repo into .bare (no working tree, just the git database)
git clone --bare <repo-url> .bare

# 2. Make the parent behave like a git repo so `git worktree` works from here.
echo "gitdir: $PWD/.bare" > .git

# 3. Create the "main" worktree. This is the source-of-truth slot the sync
#    hook copies local config *from*, so set it up first.
git worktree add main main
```

That's the core: a bare repo plus one `main` worktree. Add as many more as you like.

---

## Day-to-day: adding and removing worktrees

All of these run from the parent (`noflex/`):

```sh
git worktree add feature-a             # new branch "feature-a"
git worktree add feature-b origin/x    # existing branch / remote ref
git worktree list                      # see all worktrees
cd feature-a                           # work in it like any checkout
```

Each worktree is a normal working directory — open it in your editor, run `noflex-server`, build, etc., independently of the others. Run `npm install` in a new worktree's `web/` before building the client; `node_modules/` is per worktree.

To remove a worktree and delete its branch in one command, add this alias to your **global** git config (`~/.config/git/config`) — a personal convenience, not part of the repo:

```gitconfig
[alias]
    kill-tree = "!f() { if git show-ref --verify --quiet refs/heads/\"$1\"; then git worktree remove -f \"$1\" && git branch -D \"$1\"; else echo \"Branch $1 not found\"; fi; }; f"
```

```sh
git kill-tree feature-a
```

---

## Share dependency builds across worktrees (sccache)

**Every worktree shares one `sccache` cache and one server**, so a third-party crate compiled in `main` is a cache hit in `feature-a`. Nothing to do by hand — the Nix dev shell configures it, so `direnv` / `nix develop` sets it up whenever you enter a worktree.

The shell hook derives a **project root that's the worktree _parent_** (not any single worktree) and points the cache at it:

```sh
# PROJECT_ROOT = parent of the bare repo = noflex/  (shared by ALL worktrees)
export PROJECT_ROOT="$(git rev-parse --git-common-dir | xargs dirname | xargs realpath)"
mkdir -p "$PROJECT_ROOT/.cache"

export RUSTC_WRAPPER=sccache
export SCCACHE_DIR="${SCCACHE_DIR:-$PROJECT_ROOT/.cache/sccache}"   # shared on disk
export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-8G}"
export SCCACHE_SERVER_PORT=42225                                    # dedicated server
```

In a plain clone (no bare repo), `PROJECT_ROOT` is simply the checkout, and linked worktrees you add from it share that checkout's cache.

| Var                         | Why it matters for worktrees                                                                                                                                   |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PROJECT_ROOT`              | The worktree _parent_, identical from every worktree, so everything below keys off a stable location.                                                          |
| `RUSTC_WRAPPER=sccache`     | Cargo routes all `rustc` invocations through sccache.                                                                                                          |
| `SCCACHE_DIR`               | On-disk cache at the shared parent → every worktree reads and writes the **same** cache. (Overridable via env.)                                                |
| `SCCACHE_SERVER_PORT=42225` | Pins a dedicated port: NoFlex gets its own sccache daemon (distinct from other projects on your machine) and all worktrees converge on the **same** daemon. |
| `SCCACHE_CACHE_SIZE=8G`     | Roomier than the default. (Overridable.)                                                                                                                       |

### What the cache does and doesn't share

- **Registry dependencies hit across worktrees.** Their sources live under `~/.cargo/registry`, the same path from every worktree, so wasmtime, async-graphql and the rest compile once.
- **Workspace crates never hit across worktrees.** sccache hashes a Rust compile's working directory, which is the worktree itself, and its 0.15 `SCCACHE_BASEDIRS` path stripping does not apply to Rust. Dev builds of workspace crates are also incremental, which sccache never caches. Expect the `noflex-*` crates, test binaries and links to rebuild in every worktree.
- That is why `sccache --show-stats` reports a low hit rate and a large "non-cacheable" count: those are the workspace and incremental compiles, not a misconfiguration. Growing the cache won't change it.

### Disk notes

- `.cache/` is **gitignored**, so it never leaks into commits. Because it lives at the shared parent, it survives adding and removing worktrees.
- Each worktree has its own `backend/target/`, and it only grows: every edit leaves new test executables and incremental session directories behind. Run `cargo clean` in `backend/` when it gets large, and `git worktree remove` deletes a finished worktree's `target/` with it.
- The plugin-host fixture guests build into `backend/crates/noflex-plugin-host/tests/fixtures/target/`, and each first-party plugin into its own `backend/plugins/<name>/target/`. Neither is touched by `cargo clean` in `backend/`.
- CI restores the same `.cache/sccache` directory between runs with `actions/cache` and prints `sccache --show-stats` at the end.

---

## Keep local config in sync across worktrees

You don't want to re-create local, untracked config by hand in every worktree. A single `post-checkout` hook handles it: it treats **`main` as the source-of-truth slot** and copies its local config into whichever worktree you just created or switched into.

It runs on every `git checkout`, `git switch`, `git worktree add` (and `git clone`) inside any non-`main` worktree, and syncs:

1. **Top-level local files** — `.envrc.local`, `.mcp.json`, `.claude/settings.local.json`.
2. **Every `.env.local`** anywhere under `main` (skipping `node_modules/` and `target/`), mirrored to the matching relative path in the current worktree.
3. Then runs `direnv allow` so the worktree's `.envrc` is approved automatically.

Both copies are **copy-if-absent**: a file is copied only when it doesn't already exist in the destination (as a real file _or_ a symlink). Existing files are never overwritten, so a worktree where you've customized a config won't get clobbered. The hook exits early if you're already in `main` or the parent directory.

### Install the hook

Drop this at `noflex/.bare/hooks/post-checkout` and make it executable (`chmod +x .bare/hooks/post-checkout`):

```sh
#!/bin/sh

# 1. Define your "Source of Truth" slot name
SOURCE_SLOT_NAME="main"

# 2. Get absolute paths
BARE_DIR=$(git rev-parse --git-common-dir)
TOP_LEVEL=$(realpath "$BARE_DIR/..")
SOURCE_PATH="$TOP_LEVEL/$SOURCE_SLOT_NAME"
CURRENT_WORKTREE=$(pwd)

# 3. Safety Check: Don't run if in the source slot or outside a worktree
if [ "$CURRENT_WORKTREE" = "$SOURCE_PATH" ] || [ "$CURRENT_WORKTREE" = "$TOP_LEVEL" ]; then
    exit 0
fi

# 4. Sync top-level local files (copied, not linked; skip if already present)
for file in .envrc.local .mcp.json .claude/settings.local.json; do
    if [ -f "$SOURCE_PATH/$file" ] && [ ! -e "$CURRENT_WORKTREE/$file" ] && [ ! -L "$CURRENT_WORKTREE/$file" ]; then
        mkdir -p "$(dirname "$CURRENT_WORKTREE/$file")"
        cp "$SOURCE_PATH/$file" "$CURRENT_WORKTREE/$file"
    fi
done

# 5. Find and copy all .env.local files in the repo
find "$SOURCE_PATH" -name ".env.local" -not -path "*/node_modules/*" -not -path "*/target/*" | while read -r src_file; do

    # Relative path from the source slot root, e.g. web/.env.local
    rel_path="${src_file#$SOURCE_PATH/}"
    dest_file="$CURRENT_WORKTREE/$rel_path"

    # Skip if the destination already exists (file or symlink)
    if [ -e "$dest_file" ] || [ -L "$dest_file" ]; then
        continue
    fi

    mkdir -p "$(dirname "$dest_file")"
    cp "$src_file" "$dest_file"
done

# 6. Silent direnv allow
if command -v direnv > /dev/null 2>&1; then
    direnv allow
fi

echo "✅ Local configs synced from $SOURCE_SLOT_NAME"
```

### direnv / Nix ties it together

The committed `.envrc` is just:

```sh
use flake
source_env_if_exists .envrc.local
```

`direnv` loads the flake's dev shell, which exports the `sccache` / `PROJECT_ROOT` environment described above. The hook runs `direnv allow` for you, so a brand-new worktree is fully configured — env loaded, cache wired, config in place — the moment you enter it.

### Things to know before you adopt this

- **`main` is the source-of-truth slot**, hardcoded in the hook (`SOURCE_SLOT_NAME="main"`). Set up `main` first and keep your canonical local config there. Rename the slot and you must edit the hook.
- **Copy-if-absent never overwrites.** That's intentional — you can customize a config in one worktree without it being clobbered — but the hook won't _update_ an out-of-date copy either. Delete the stale file and the next checkout re-syncs it from `main`.
- **The hook lives outside git.** `.bare/hooks/` isn't version-controlled, so a fresh clone won't have it. Treat the snippet above as the source of truth and reinstall it (with `chmod +x`) when you bootstrap a new machine.
- **Server state is shared by default.** `noflex-server` keeps its database, plugin directory, and config under your XDG directories, not the checkout, so every worktree talks to the same dev database. To give a worktree its own, export `NOFLEX_DATA_DIR` and `NOFLEX_CACHE_DIR` (and `NOFLEX_BIND` to run two servers at once) from that worktree's `.envrc.local`.
- **The cache lives at the project root's location.** If you move the `noflex/` directory itself, the on-disk cache path changes (unless you set `SCCACHE_DIR` explicitly).
