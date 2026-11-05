#!/usr/bin/env bash
# Builds one upstream web app with Marquee's patches, from a checkout of the commit pinned in UPSTREAM.lock.
#
#   ui/build.sh sonarr|radarr|seerr <checkout> <tokens.css> <out>
#
# <tokens.css> is the kit's colour tokens (web/static/css/tokens.css); <out> receives the built files that the
# server serves at /sonarr, /radarr or /requests (see server/src/upstream_ui.rs).
set -euo pipefail
app=${1:?app}; src=${2:?checkout}; tokens=${3:?tokens.css}; out=${4:?out}
here=$(cd "$(dirname "$0")" && pwd)
src=$(cd "$src" && pwd)
tokens=$(cd "$(dirname "$tokens")" && pwd)/$(basename "$tokens")
mkdir -p "$out"; out=$(cd "$out" && pwd)
case "$app" in
  sonarr|radarr)
    node "$here/arr/patch.mjs" "$src"
    cd "$src"
    yarn install --frozen-lockfile --network-timeout 600000
    yarn build --env production
    cp "$tokens" _output/UI/Content/encore-tokens.css
    find _output/UI -name '*.map' -delete
    cp -a _output/UI/. "$out/"
    ;;
  seerr)
    node "$here/seerr/patch.mjs" "$src" "$tokens"
    cd "$src"
    corepack enable >/dev/null 2>&1 || true
    CYPRESS_INSTALL_BINARY=0 pnpm install --frozen-lockfile --ignore-scripts
    pnpm exec next telemetry disable >/dev/null 2>&1 || true
    pnpm exec next build
    cp -a out/. "$out/"
    ;;
  *) echo "ui/build.sh: unknown app $app" >&2; exit 2 ;;
esac
echo "Built $app into $out"
