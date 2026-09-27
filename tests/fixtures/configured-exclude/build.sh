#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
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
write docs/legacy/old.md <<'D'
Run `make demo` to start.
D
write docs/changelog.md <<'D'
## 0.1.0 — `make demo` was added.
D
write stilltrue.toml <<'C'
exclude = ["docs/legacy/**"]
C
commit "add the demo target"
write Makefile <<'M'
seed:
	echo seed
M
commit "drop the demo target"
