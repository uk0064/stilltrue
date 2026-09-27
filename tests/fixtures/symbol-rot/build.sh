#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
# Wholly Python, so symbols are speakable at all (ADR-0004).
write app.py <<'P'
def fold_events(events):
    return events
P
write helpers.py <<'P'
def other():
    pass
P
write CLAUDE.md <<'D'
The entry point is `fold_events()`.
D
commit "add fold_events"
write app.py <<'P'
def collapse_events(events):
    return events
P
commit "rename fold_events to collapse_events"
