#!/usr/bin/env bash
# kind: near-miss: compatible pin
# scope: in
# expect: silent
# check: grep -q '^22.9' .node-version
source "$(dirname "$0")/../../harbor.sh" "$1"
write .node-version <<'V'
22.9.0
V
commit "Node 22.9"
