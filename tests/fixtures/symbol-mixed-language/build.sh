#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write app.py <<'P'
def other():
    pass
P
write main.go <<'G'
package main
G
write util.go <<'G'
package main
G
write CLAUDE.md <<'D'
Call `fold_events()` to combine them.
D
commit "mixed language repository"
