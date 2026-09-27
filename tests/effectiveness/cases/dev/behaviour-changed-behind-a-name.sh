#!/usr/bin/env bash
# kind: semantic error
# scope: out
# expect: silent
# check: grep -q enumerate src/pkg/core.py && grep -q 'returns a list' CLAUDE.md
source "$(dirname "$0")/../../workbench.sh" "$1"
sed 's/return \[event for event in events\]/return {i: e for i, e in enumerate(events)}/' src/pkg/core.py > core.tmp && mv core.tmp src/pkg/core.py
write CLAUDE.md.append <<'D'
- `fold_events()` returns a list.
D
cat CLAUDE.md.append >> CLAUDE.md && rm CLAUDE.md.append
commit "fold into a dict; the document still says list"
