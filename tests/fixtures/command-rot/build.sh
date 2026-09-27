#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write Makefile <<'M'
demo:
	echo demo
M
write CLAUDE.md <<'D'
Run `make demo` to see it.
D
commit "add demo target"
write Makefile <<'M'
seed:
	echo seed

serve:
	echo serve
M
commit "split demo into seed+serve"
