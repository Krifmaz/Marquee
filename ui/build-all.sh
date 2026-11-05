#!/usr/bin/env bash
# Builds the forked Sonarr, Radarr and Seerr web apps from the commits pinned in UPSTREAM.lock (the Krifmaz fork
# first, else upstream: both hold the same commit), with Marquee's patches, into <out>/sonarr, <out>/radarr and
# <out>/seerr. Every text file also gets its brotli and gzip copy next to it, like the web app at /.
#
#   ui/build-all.sh <UPSTREAM.lock> <tokens.css> <work> <out>
set -euo pipefail
lock=${1:?UPSTREAM.lock}; tokens=${2:?tokens.css}; work=${3:?work folder}; out=${4:?out}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$work" "$out"

# one value of one [app] table of UPSTREAM.lock
pin() { python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))[sys.argv[2]][sys.argv[3]])' "$lock" "$1" "$2"; }

for app in sonarr radarr seerr; do
  commit=$(pin "$app" commit); tag=$(pin "$app" tag)
  src="$work/$app"
  rm -rf "$src"; git init -q "$src"
  if ! git -C "$src" fetch -q --depth 1 "$(pin "$app" fork)" "$commit" 2>/dev/null; then
    echo "The fork of $app has no commit $commit yet; fetching it from upstream."
    git -C "$src" fetch -q --depth 1 "$(pin "$app" upstream)" "$commit"
  fi
  git -C "$src" checkout -q FETCH_HEAD
  "$here/build.sh" "$app" "$src" "$tokens" "$out/$app"
  echo "${tag#v}" > "$out/$app/VERSION"
done

find "$out" -type f \( -name '*.js' -o -name '*.css' -o -name '*.html' -o -name '*.svg' -o -name '*.json' \
  -o -name '*.webmanifest' -o -name '*.txt' -o -name '*.xml' \) -exec brotli -f -k -q 11 {} + -exec gzip -f -k -n -9 {} +
echo "Built the forked web apps into $out"
