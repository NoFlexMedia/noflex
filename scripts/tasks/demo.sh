#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# A throwaway NoFlex with a real library: Blender's open movies, on one server
# or several linked ones. Everything lives under .cache, metadata comes from the
# real providers, and search embeds with the default model, as a release bundle
# would. Run it through `just demo`.

# GraphQL variables look like shell ones.
# shellcheck disable=SC2016

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
# Worktrees share the films and the embedding model but run their own servers.
common=$(dirname "$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)")
films=$common/.cache/demo-films
model=$common/.cache/demo-model
state=$root/.cache/demo
config=$state/config
seeded=$state/seeded

username='admin'
password='noflex-demo-pass'

# What `up` was last asked for, from $config.
servers=0 port=28793 bind=127.0.0.1 advertise=127.0.0.1 release=''

# kind | title folder | path inside it | source. Zips hold the one film.
catalog='movies|Sintel (2010)|Sintel (2010).mkv|https://download.blender.org/demo/movies/Sintel.2010.720p.mkv.zip
movies|Big Buck Bunny (2008)|Big Buck Bunny (2008).mp4|https://download.blender.org/demo/movies/BBB/bbb_sunflower_1080p_30fps_normal.mp4.zip
movies|Tears of Steel (2012)|Tears of Steel (2012).mov|https://download.blender.org/demo/movies/ToS/tears_of_steel_720p.mov
movies|Elephants Dream (2006)|Elephants Dream (2006).mp4|https://archive.org/download/ElephantsDream/ed_1024_512kb.mp4
movies|Sprite Fright (2021)|Sprite Fright (2021).webm|https://commons.wikimedia.org/wiki/Special:FilePath/Sprite%20Fright%20-%20Blender%20Open%20Movie-full%20movie.webm
movies|Charge (2022)|Charge (2022).webm|https://commons.wikimedia.org/wiki/Special:FilePath/Charge%20-%20Blender%20Open%20Movie-full%20movie.webm
shows|Caminandes (2013)|Season 01/Caminandes - S01E01 - Llama Drama.ogv|https://commons.wikimedia.org/wiki/Special:FilePath/Caminandes-%20Llama%20Drama%20-%20Short%20Movie.ogv
shows|Caminandes (2013)|Season 01/Caminandes - S01E02 - Gran Dillama.mp4|https://download.blender.org/demo/movies/caminandes_gran_dillama.mp4.zip
shows|Caminandes (2013)|Season 01/Caminandes - S01E03 - Llamigos.webm|https://commons.wikimedia.org/wiki/Special:FilePath/Caminandes%203%20-%20Llamigos%20-%20Blender%20Animated%20Short.webm'

die() {
  echo "demo: $*" >&2
  exit 1
}

home_of() { echo "$state/servers/$1"; }

# Probes and API calls need an address that connects, which 0.0.0.0 isn't.
local_host() {
  if [ "$bind" = 0.0.0.0 ]; then echo 127.0.0.1; else echo "$bind"; fi
}

load_config() {
  servers=0
  if [ -f "$config" ]; then
    # shellcheck disable=SC1090
    source "$config"
  fi
}

fetch_one() {
  local dest=$1 url=$2 work part
  work=$(mktemp -d "$films/.fetch.XXXXXX")
  # Resumable across runs: blender.org drops long downloads now and then.
  part=$films/.$(basename "$url").part
  curl -fsSL -A 'noflex-demo/0.1' -C - --retry 8 --retry-all-errors \
    --speed-time 30 --speed-limit 1000 -o "$part" "$url"
  if [[ $url == *.zip ]]; then
    unzip -qjo "$part" -x '__MACOSX/*' -d "$work"
    mv "$work"/* "$dest"
    rm -f "$part"
  else
    mv "$part" "$dest"
  fi
  rm -rf "$work"
  echo "  fetched $(basename "$dest")"
}

fetch() {
  local kind title file url dest pids=() failed=0
  mkdir -p "$films"
  while IFS='|' read -r kind title file url; do
    dest=$films/$kind/$title/$file
    [ -f "$dest" ] && continue
    [ ${#pids[@]} -gt 0 ] || echo "Downloading the sample films (about 2 GB)..."
    mkdir -p "$(dirname "$dest")"
    fetch_one "$dest" "$url" &
    pids+=("$!")
  done <<<"$catalog"
  for pid in ${pids[@]+"${pids[@]}"}; do
    wait "$pid" || failed=1
  done
  [ "$failed" = 0 ] || die "a download failed; run this again to resume"
}

build() {
  echo "Building..."
  local web=$root/web had_build=1
  [ -f "$web/build/index.html" ] || had_build=0
  if [ "$had_build" = 0 ] || [ -n "$(find "$web/src" "$web/package.json" -newer "$web/build/index.html" -print -quit)" ]; then
    [ -d "$web/node_modules" ] || (cd "$web" && npm install)
    (cd "$web" && npm run build)
    # With no build to watch, the server's build script watches this instead.
    [ "$had_build" = 1 ] || touch "$web/package.json"
  fi
  (
    cd "$root/backend"
    cargo xtask plugin-build tmdb tvdb embeddings
    # Checks the pinned hashes, so only the first run downloads (about 280 MB).
    cargo xtask model-fetch "$model"
    cargo build -p noflex-server ${release:+--release}
  )
}

# The films everyone gets one copy of, except Sintel, which is on every server so
# the same title can be seen arriving from several. Hard links, as the scanner
# doesn't follow symlinks.
lay_out_library() {
  local kind title file url last='' index=0 target
  while IFS='|' read -r kind title file url; do
    [ "$title" != "$last" ] || continue
    last=$title
    for n in $(seq 1 "$servers"); do
      if [ "$title" != "Sintel (2010)" ]; then
        target=$((index % servers + 1))
        [ "$n" = "$target" ] || continue
      fi
      mkdir -p "$(home_of "$n")/library/$kind"
      cp -al "$films/$kind/$title" "$(home_of "$n")/library/$kind/" 2>/dev/null ||
        cp -a "$films/$kind/$title" "$(home_of "$n")/library/$kind/"
    done
    [ "$title" = "Sintel (2010)" ] || index=$((index + 1))
  done <<<"$catalog"
}

pid_of() {
  local file pid
  file=$(home_of "$1")/pid
  [ -f "$file" ] || return 1
  pid=$(cat "$file")
  [[ "$(ps -p "$pid" -o comm= 2>/dev/null)" == noflex-server* ]] && echo "$pid"
}

stop_all() {
  local dir n pid
  [ -d "$state/servers" ] || return 0
  for dir in "$state"/servers/*; do
    n=$(basename "$dir")
    pid=$(pid_of "$n") || continue
    kill "$pid"
    for _ in $(seq 1 100); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -9 "$pid" 2>/dev/null || true
  done
}

start() {
  local n=$1 home server_port binary plugin
  home=$(home_of "$n")
  server_port=$((port + n - 1))
  binary=${CARGO_TARGET_DIR:-$root/backend/target}/$([ -n "$release" ] && echo release || echo debug)/noflex-server
  if (exec 3<>"/dev/tcp/$(local_host)/$server_port") 2>/dev/null; then
    die "port $server_port is taken; stop what's using it or pass --port"
  fi
  mkdir -p "$home/data" "$home/cache" "$home/config/noflex/plugins"
  for plugin in metadata-tmdb metadata-tvdb embeddings; do
    cp "$root/backend/plugins/noflex-$plugin/target/wasm32-wasip2/release/noflex_${plugin//-/_}.wasm" \
      "$home/config/noflex/plugins/${plugin#metadata-}.wasm"
  done
  (
    # Whatever this shell has set is for the real server, not these.
    for name in $(env | sed -n 's/^\(NOFLEX_[A-Z0-9_]*\)=.*/\1/p'); do unset "$name"; done
    export NOFLEX_BIND=$bind:$server_port
    export NOFLEX_DATA_DIR=$home/data NOFLEX_CACHE_DIR=$home/cache
    export NOFLEX_CROSS_ORIGIN=true NOFLEX_LOG_LEVEL=info XDG_CONFIG_HOME=$home/config
    export NOFLEX_PLUGIN_ASSETS_DIR=$model
    cd "$home"
    nohup "$binary" >>"$home/server.log" 2>&1 </dev/null &
    echo "$!" >"$home/pid"
  )
}

gql() { # port token query [jq variable arguments...]
  local url token=$2 query=$3 body response
  url=http://$(local_host):$1/graphql
  shift 3
  body=$(jq -cn --arg gql_query "$query" "$@" '{query: $gql_query, variables: ($ARGS.named | del(.gql_query))}')
  response=$(curl -sS -m 120 -H 'content-type: application/json' ${token:+-H "authorization: Bearer $token"} -d "$body" "$url")
  if [ "$(jq -r '.errors // empty | length' <<<"$response")" != "" ]; then
    die "GraphQL error from :$port: $(jq -c .errors <<<"$response")"
  fi
  jq -c .data <<<"$response"
}

await_ready() {
  local n=$1 server_port
  server_port=$((port + n - 1))
  for _ in $(seq 1 240); do
    if curl -fsS -m 2 -o /dev/null -H 'content-type: application/json' -d '{"query":"{serverInfo{name}}"}' \
      "http://$(local_host):$server_port/graphql" 2>/dev/null; then
      return 0
    fi
    pid_of "$n" >/dev/null || break
    sleep 0.5
  done
  tail -n 20 "$(home_of "$n")/server.log" >&2
  die "server $n didn't start; its log is in $(home_of "$n")/server.log"
}

scan_all() { # port token library ids...
  local server_port=$1 token=$2 id scan
  shift 2
  for id in "$@"; do
    gql "$server_port" "$token" 'mutation($id: ID!) { triggerLibraryScan(id: $id) { state } }' --arg id "$id" >/dev/null
  done
  for id in "$@"; do
    while :; do
      scan=$(gql "$server_port" "$token" 'query($id: ID!) { libraryScanStatus(id: $id) { state error } }' --arg id "$id")
      [ "$(jq -r .libraryScanStatus.state <<<"$scan")" = RUNNING ] || break
      sleep 1
    done
    [ "$(jq -r .libraryScanStatus.state <<<"$scan")" != FAILED ] || die "a scan failed: $(jq -r .libraryScanStatus.error <<<"$scan")"
  done
}

seed() {
  local n server_port kind name enum library ids tokens=() a b key
  echo "Setting up accounts and scanning..."
  for n in $(seq 1 "$servers"); do
    server_port=$((port + n - 1))
    tokens[n]=$(gql "$server_port" '' 'mutation($u: String!, $p: String!) { completeSetupWizard(username: $u, password: $p) { accessToken } }' \
      --arg u "$username" --arg p "$password" | jq -r .completeSetupWizard.accessToken)
    ids=()
    for kind in movies shows; do
      library=$(home_of "$n")/library/$kind
      [ -d "$library" ] || continue
      name=$(tr '[:lower:]' '[:upper:]' <<<"${kind:0:1}")${kind:1}
      enum=$(tr '[:lower:]' '[:upper:]' <<<"$kind")
      ids+=("$(gql "$server_port" "${tokens[n]}" "mutation(\$p: [String!]!) { createLibrary(name: \"$name\", kind: $enum, paths: \$p) { id } }" \
        --argjson p "$(jq -cn --arg path "$library" '[$path]')" | jq -r .createLibrary.id)")
    done
    scan_all "$server_port" "${tokens[n]}" "${ids[@]}"
  done

  # Something to continue watching, on the first server.
  local works position=120000 work
  works=$(gql "$port" "${tokens[1]}" '{ recentlyAdded(first: 20) { nodes { id kind } } }' |
    jq -r '[.recentlyAdded.nodes[] | select(.kind == "MOVIE") | .id][:2][]')
  for work in $works; do
    gql "$port" "${tokens[1]}" 'mutation($id: ID!, $at: Int!) { reportProgress(target: { kind: WORK, id: $id }, positionMs: $at, state: STOPPED) { positionMs } }' \
      --arg id "$work" --argjson at "$position" >/dev/null
    position=$((position + 80000))
  done

  # Every server links every other, with a key of its own.
  for a in $(seq 1 "$servers"); do
    for b in $(seq 1 "$servers"); do
      [ "$a" != "$b" ] || continue
      key=$(gql "$((port + b - 1))" "${tokens[b]}" 'mutation($name: String!) { createApiKey(name: $name) { key } }' \
        --arg name "Demo $a" | jq -r .createApiKey.key)
      gql "$((port + a - 1))" "${tokens[a]}" 'mutation($origin: String!, $name: String!, $key: String!) { addLinkedServer(origin: $origin, name: $name, apiKey: $key) { id } }' \
        --arg origin "http://$advertise:$((port + b - 1))" --arg name "Demo $b" --arg key "$key" >/dev/null
    done
  done
}

status() {
  load_config
  [ "$servers" != 0 ] || {
    echo "No demo yet; start one with: just demo up"
    return 0
  }
  local n running titles title
  for n in $(seq 1 "$servers"); do
    running=stopped
    pid_of "$n" >/dev/null && running=running
    titles=''
    for title in "$(home_of "$n")"/library/*/*; do
      titles="$titles${titles:+, }$(basename "$title")"
    done
    printf '  %s  http://%s:%s  %s  %s\n' "$n" "$advertise" "$((port + n - 1))" "$running" "$titles"
  done
  echo "  Sign in as $username / $password"
}

up() {
  local want_servers=1 want_port=28793 want_bind=127.0.0.1 want_advertise=''
  while [ $# -gt 0 ]; do
    case $1 in
    --servers | --port | --bind | --advertise)
      [ $# -ge 2 ] || die "$1 needs a value"
      case $1 in
      --servers) want_servers=$2 ;;
      --port) want_port=$2 ;;
      --bind) want_bind=$2 ;;
      --advertise) want_advertise=$2 ;;
      esac
      shift 2
      ;;
    --release)
      release=1
      shift
      ;;
    *) die "unknown option $1; see just demo" ;;
    esac
  done
  [[ $want_servers =~ ^[1-9]$ ]] || die "--servers takes 1 to 9"
  [[ $want_port =~ ^[0-9]+$ ]] || die "--port takes a port number"
  if [ -z "$want_advertise" ]; then
    [ "$want_bind" != 0.0.0.0 ] || die "--bind 0.0.0.0 needs --advertise, the address clients reach this machine at"
    want_advertise=$want_bind
  fi

  fetch
  build

  stop_all
  local wanted="servers=$want_servers
port=$want_port
bind=$want_bind
advertise=$want_advertise"
  local fresh=''
  if [ ! -f "$seeded" ] || [ "$wanted" != "$(cat "$config" 2>/dev/null)" ]; then
    fresh=1
    [ ! -d "$state" ] || echo "Starting afresh: the options changed or the last setup didn't finish."
    rm -rf "$state"
    mkdir -p "$state"
    echo "$wanted" >"$config"
  fi
  load_config

  [ -z "$fresh" ] || lay_out_library
  local n
  for n in $(seq 1 "$servers"); do
    start "$n"
  done
  for n in $(seq 1 "$servers"); do
    await_ready "$n"
  done
  if [ -n "$fresh" ]; then
    seed
    touch "$seeded"
  fi
  echo
  status
  echo
  echo "Logs: just demo logs [server]    Stop: just demo down"
}

command=${1:-}
[ $# -eq 0 ] || shift
case $command in
up) up "$@" ;;
down)
  stop_all
  ;;
status) status ;;
logs) tail -F "$(home_of "${1:-1}")/server.log" ;;
fetch) fetch ;;
reset)
  stop_all
  rm -rf "$state"
  ;;
clean)
  stop_all
  rm -rf "$state" "$films" "$model"
  ;;
*) die "unknown command: $command" ;;
esac
