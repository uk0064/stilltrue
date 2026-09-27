#!/usr/bin/env bash
# kind: deleted command target
# scope: in
# expect: stilltrue/command/rot make demo
# check: ! grep -q '^demo:' Makefile
source "$(dirname "$0")/../../workbench.sh" "$1"
write Makefile <<'M'
seed:
	python -m pkg.seed

lint:
	ruff check src
M
commit "drop the demo target"
