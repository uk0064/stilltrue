#!/usr/bin/env bash
# ADR-0025: a nested repository is not this repository. An agent's worktree kept inside
# the checkout, at the commit before a rename, still defines `fold_events`. Walked as
# part of this repository, that definition entered the index and silenced the rot the
# rename caused in CLAUDE.md — and the worktree's own documents were linted as this
# repository's. The rot must be reported, and nothing under the worktree must be read.
source "$(dirname "$0")/../_common.sh"
init "$1"
write src/app.py <<'P'
def fold_events(events):
    return list(events)
P
write CLAUDE.md <<'D'
Events are folded by `fold_events()`.
D
commit "add fold_events"
write src/app.py <<'P'
def reduce_events(events):
    return list(events)
P
commit "rename fold_events to reduce_events"
git worktree add -q .claude/worktrees/agent HEAD~1
