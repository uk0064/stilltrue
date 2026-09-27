#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write CLAUDE.md <<'D'
See `docs/never-existed.md` for detail.
D
commit "add instructions"
