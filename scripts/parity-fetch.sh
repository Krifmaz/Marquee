#!/usr/bin/env sh
# Fetches Sonarr, Radarr and Seerr at the commits UPSTREAM.lock pins, into <folder>/sonarr, /radarr and /seerr.
# Each tree comes from the Krifmaz fork when it has the commit, else from upstream (same commit in both).
#
# Usage, from the repository root: scripts/parity-fetch.sh /tmp/upstream
set -eu

dest=${1:?"Say where to put the checkouts, for example: scripts/parity-fetch.sh /tmp/upstream"}
lock=UPSTREAM.lock

# Reads one field of one app from UPSTREAM.lock.
field() {
	awk -v app="[$1]" -v key="$2" '
		/^\[/ { inside = ($0 == app) }
		inside && $1 == key { gsub(/"/, "", $3); print $3; exit }
	' "$lock"
}

for app in sonarr radarr seerr; do
	commit=$(field "$app" commit)
	dir="$dest/$app"
	if [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null || true)" = "$commit" ]; then
		echo "$app: already at $commit"
		continue
	fi
	rm -rf "$dir"
	git init -q "$dir"
	if GIT_LFS_SKIP_SMUDGE=1 git -C "$dir" fetch -q --depth 1 "$(field "$app" fork)" "$commit" 2>/dev/null; then
		from=fork
	else
		GIT_LFS_SKIP_SMUDGE=1 git -C "$dir" fetch -q --depth 1 "$(field "$app" upstream)" "$commit"
		from=upstream
	fi
	git -C "$dir" -c advice.detachedHead=false checkout -q FETCH_HEAD
	echo "$app: $commit (from the $from)"
done
