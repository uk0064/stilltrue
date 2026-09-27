#!/usr/bin/env bash
# pyapp, after a commit renamed the `test` target to `unit` and left CLAUDE.md behind:
# the instruction is stale before any session starts.
HERE="$(cd "$(dirname "$0")" && pwd)"
bash "$HERE/../build.sh" "$1"
source "$HERE/../../_bench.sh"
cd "$1"

write Makefile <<'M'
.PHONY: unit lint

unit:
	PYTHONPATH=src python3 -m unittest discover -s tests -q

lint:
	python3 -m compileall -q src
M
commit "Rename the test target to unit"
