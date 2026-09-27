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
# A partial clone has every commit but fetches blobs on demand, so it is not shallow —
# and a pickaxe over it reads an incomplete tree.
git config uploadpack.allowFilter true
git clone -q --filter=blob:none "file://$origin" "$1"
