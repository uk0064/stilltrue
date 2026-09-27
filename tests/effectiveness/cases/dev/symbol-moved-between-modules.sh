#!/usr/bin/env bash
# kind: near-miss: symbol still defined
# scope: in
# expect: silent
# check: ! grep -q fold_events src/pkg/core.py && grep -q 'def fold_events' src/pkg/events.py
source "$(dirname "$0")/../../workbench.sh" "$1"
write src/pkg/events.py <<'P'
def fold_events(events):
    return [event for event in events]
P
sed '/def fold_events/,/return \[event/d' src/pkg/core.py > core.tmp && mv core.tmp src/pkg/core.py
commit "move fold_events into its own module"
