# Contributing

Thanks for your interest in NoFlex! This page covers how changes land. For building and running from source, start with [`docs/GETTING_STARTED.md`](docs/GETTING_STARTED.md); for code conventions, read the `AGENTS.md` in the root and in each directory you touch — they're written for people as much as for tools.

## Workflow

- Work in the Nix dev shell (`nix develop`). It is the only supported toolchain. The default shell has everything, including Flutter and several gigabytes of Android SDK. If you don't touch the app, use `nix develop .#no-flutter`; if you only touch the app, `nix develop .#flutter-only`. With direnv, set `export NOFLEX_SHELL=no-flutter` in `.envrc.local`. CI uses two smaller shells that leave out the emulator and its system images: `flutter-check` for analysis and tests, and `flutter-android` for building APKs.
- Keep commits small and focused: `area: imperative summary` (`scanner: keep year-like numbers in movie titles`), then a body explaining **why**.
- Before opening a pull request, run the gates CI runs:

  ```sh
  just check all
  just check nix
  ```

- Schema changes include the regenerated GraphQL snapshot; see [`docs/GETTING_STARTED.md`](docs/GETTING_STARTED.md#check-your-work).
- Architecture changes — anything that bends an invariant in [`backend/AGENTS.md`](backend/AGENTS.md) — need a discussion in the pull request description first.

## Storage backends

A storage backend is a crate implementing the `Storage` trait from `noflex-storage`. Postgres or MySQL support is a contribution seam, not a fork. A backend is accepted once it passes the shared conformance suite:

```rust
// backend/crates/noflex-storage-<engine>/tests/conformance.rs
#[tokio::test]
async fn conformance() {
    let store = MyStore::open_temp().await.unwrap();
    noflex_storage::test_suite::run_all(&store).await;
}
```

The full recipe is in [`backend/AGENTS.md`](backend/AGENTS.md#adding-a-storage-backend).

## Plugin API

The plugin ABI is the WIT world in `backend/crates/noflex-plugin-wit/`; each provider trait in `noflex-core/src/providers/` mirrors one hook.

- **Before 1.0**, changes to the WIT, manifest schema, and traits land cleanly as one co-versioned change: the WIT, host runtime, SDK, every first-party plugin, and the test fixture guests move together, and the API version is bumped. No compatibility shims.
- **From 1.0**, the trait surface is semver-stable. New methods get default implementations; a changed signature ships as a new trait (`MetadataProvider2`), and the old one is deprecated and removed after two minor versions.

User-visible plugin API changes update [`docs/PLUGINS.md`](docs/PLUGINS.md).

## Licensing and the CLA

NoFlex is [AGPL-3.0-only](LICENSE), and so is everything contributed to it. **Before a pull request can merge, everyone who wrote its commits signs the [Contributor License Agreement](CLA.md).** You sign once, and that covers every contribution after it.

### What the CLA does

- **You keep your copyright.** The CLA grants the maintainer a license to your contribution. It doesn't transfer ownership.
- **It lets the maintainer license NoFlex, your contribution included, under terms besides the AGPL.** App stores are the first reason. Apple's terms put restrictions on users that the AGPL forbids, so AGPL code can only ship on the App Store if every one of its authors allows it. The same permission covers commercial licenses: the maintainer may sell NoFlex under proprietary terms, or build a commercial edition on it, and may change its license later.
- **Your contribution stays free software.** Whatever else it's licensed under, your contribution itself is always also licensed under the license NoFlex had on the day you submitted it: AGPL-3.0-only today. See section 4 of the CLA.
- **It includes a patent license**, as Apache-2.0 does.

### How to sign

1. Open your pull request. The `cla` check names everyone on it who hasn't signed yet.
2. Each of them comments on the pull request:

   ```
   I have read the NoFlex CLA and I agree to its terms.
   ```

3. The check records the signature and passes. Comment `recheck` to run it again.

When the CLA changes, its version number goes up, and the check asks everyone to sign the new version.

### Who signs

- **Every author of every commit in the pull request.** The check finds people by the email on each commit, so that email must belong to your GitHub account (Settings → Emails). The check fails on a commit it can't match to an account.
- **Co-authors too.** The check doesn't read `Co-authored-by:` trailers. Only credit a co-author who has signed, and ask them to comment on the pull request if they haven't.
- **If you write code as part of your job**, your employer may own it. Get your employer's permission before you sign, because the CLA has you confirm that you have it.
- Bots, such as Dependabot, don't sign.

### What you can contribute

- **Your own work.** That includes what you write with tools such as AI assistants: you're responsible for being entitled to contribute it.
- **Someone else's code only with its license.** Put it in a commit of its own, keep its copyright and license notice, and say in the pull request where it came from and what its license is. Its license must allow distribution as part of AGPL-3.0-only software.

### Signed commits

`main` only accepts signed commits. Sign with an SSH, GPG or S/MIME key that's added to your GitHub account, so your commits show as Verified. For an SSH key:

```sh
git config --global gpg.format ssh
git config --global user.signingkey ~/.ssh/id_ed25519.pub
git config --global commit.gpgsign true
```

### File headers

Every source file carries an SPDX header:

```rust
// SPDX-License-Identifier: AGPL-3.0-only
```

Plugins are independent works and may use any license; see [`docs/PLUGIN_LICENSING.md`](docs/PLUGIN_LICENSING.md).
