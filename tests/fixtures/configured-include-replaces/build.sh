#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
# A documented footgun: `include` replaces the defaults, so naming `docs/**`
# silently stops linting CLAUDE.md — the tool's own wedge.
write Makefile <<'M'
demo:
	echo demo
M
write CLAUDE.md <<'D'
Run `make demo` to start.
D
write docs/guide.md <<'D'
Run `make demo` to start.
D
write stilltrue.toml <<'C'
include = ["docs/**"]
C
commit "add the demo target"
write Makefile <<'M'
seed:
	echo seed
M
commit "drop the demo target"
