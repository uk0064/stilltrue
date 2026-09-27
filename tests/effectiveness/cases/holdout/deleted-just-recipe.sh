#!/usr/bin/env bash
# kind: deleted command target
# scope: in
# expect: stilltrue/command/rot just serve
# check: ! grep -q '^serve:' justfile
source "$(dirname "$0")/../../harbor.sh" "$1"
write justfile <<'J'
start:
    node dist/server.js

check:
    tsc --noEmit
J
commit "rename serve to start"
