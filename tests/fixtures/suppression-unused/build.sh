#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write Makefile <<'M'
demo:
	echo demo
M
write CLAUDE.md <<'D'
<!-- stilltrue:ignore the claim under this one was fixed -->

Nothing here makes a claim any more.

<!-- stilltrue:ignore this one still covers something -->

Run `make ghost` to stop.
D
commit "add instructions"
