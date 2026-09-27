#!/usr/bin/env bash
# The default `docs/**` glob matches whatever is under docs/, and being matched is not
# the same as being prose. Only a document is extracted from; a script that happens to
# contain a backtick is code, and reading it would invent claims nobody made.
source "$(dirname "$0")/../_common.sh"
init "$1"
write Makefile <<'M'
demo:
	echo demo
M
write docs/guide.md <<'D'
Run `make demo` to start.
D
write docs/guide.rst <<'D'
Guide
=====

Run ``make demo`` to start.
D
write docs/helper.py <<'P'
"""Run `make demo` to build."""


def run():
    pass
P
write docs/setup.sh <<'S'
# Run `make demo` to build
set -e
S
write docs/meta.json <<'J'
{"note": "Run `make demo`"}
J
commit "add the demo target"
write Makefile <<'M'
seed:
	echo seed
M
commit "drop the demo target"
