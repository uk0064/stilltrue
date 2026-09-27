#!/usr/bin/env bash
# kind: near-miss: document updated
# scope: in
# expect: silent
# check: [ ! -e src/pkg/core.py ] && grep -q src/pkg/engine.py CLAUDE.md
source "$(dirname "$0")/../../workbench.sh" "$1"
git mv src/pkg/core.py src/pkg/engine.py
sed 's#src/pkg/core.py#src/pkg/engine.py#' CLAUDE.md > c.tmp && mv c.tmp CLAUDE.md
commit "rename core to engine, and say so"
