#!/usr/bin/env bash
# kind: near-miss: quiet recipe
# scope: in
# expect: silent
# check: grep -q '^@serve:' justfile
source "$(dirname "$0")/../../harbor.sh" "$1"
write justfile <<'J'
@serve:
    node dist/server.js

check:
    tsc --noEmit
J
commit "make serve quiet"
