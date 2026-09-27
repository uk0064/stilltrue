#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
origin="$1.origin"
init "$origin"
write Makefile <<'M'
demo:
	echo demo
M
write CLAUDE.md <<'D'
Run `make demo`.
D
commit "add demo target"
write Makefile <<'M'
seed:
	echo seed
M
commit "drop the demo target"
git clone -q --depth 1 "file://$origin" "$1"
