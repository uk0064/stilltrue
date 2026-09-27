#!/usr/bin/env bash
# pyapp with unrelated rot already present: README.md documents a `docs` target that a
# later commit removed. Nothing a task here asks for touches it.
HERE="$(cd "$(dirname "$0")" && pwd)"
bash "$HERE/../build.sh" "$1"
source "$HERE/../../_bench.sh"
cd "$1"

write Makefile <<'M'
.PHONY: test lint docs

test:
	PYTHONPATH=src python3 -m unittest discover -s tests -q

lint:
	python3 -m compileall -q src

docs:
	python3 -m pydoc -w pyapp
M
write README.md <<'D'
# pyapp

A small configuration library. Contributors: see `CLAUDE.md`.

Build the API reference with `make docs`.
D
commit "Add a docs target"

write Makefile <<'M'
.PHONY: test lint

test:
	PYTHONPATH=src python3 -m unittest discover -s tests -q

lint:
	python3 -m compileall -q src
M
commit "Drop the docs target; the reference moved to the wiki"
