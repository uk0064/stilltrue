#!/usr/bin/env bash
# kind: near-miss: target kept
# scope: in
# expect: silent
# check: grep -q '^serve: check' justfile
source "$(dirname "$0")/../../harbor.sh" "$1"
write justfile <<'J'
serve: check
    node --enable-source-maps dist/server.js

check:
    tsc --noEmit
J
commit "serve with source maps"
