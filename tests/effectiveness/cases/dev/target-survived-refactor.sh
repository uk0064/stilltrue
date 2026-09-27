#!/usr/bin/env bash
# kind: near-miss: target kept
# scope: in
# expect: silent
# check: grep -q '^demo:' Makefile && ! git diff --quiet HEAD~1 -- Makefile
source "$(dirname "$0")/../../workbench.sh" "$1"
write Makefile <<'M'
# Everyday targets.
lint:
	ruff check src

seed:
	python -m pkg.seed

demo: seed
	python -m pkg.demo
M
commit "reorder the Makefile"
