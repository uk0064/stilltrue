#!/usr/bin/env bash
# kind: removed symbol
# scope: in
# expect: stilltrue/symbol/rot fold_events()
# check: ! grep -rq fold_events src
source "$(dirname "$0")/../../workbench.sh" "$1"
sed 's/fold_events/reduce_events/' src/pkg/core.py > core.tmp && mv core.tmp src/pkg/core.py
commit "rename fold_events to reduce_events"
