#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write build/Makefile <<'M'
demo:
	echo demo
M
write CLAUDE.md <<'D'
Run `make demo`.
D
commit "add demo target in build/"
git mv build/Makefile Makefile
write Makefile <<'M'
seed:
	echo seed
M
commit "move the Makefile to the root"
