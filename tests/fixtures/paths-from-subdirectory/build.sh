#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write docs/guide/README.md <<'D'
See `docs/never-existed.md` for detail.
D
commit "add a guide"
