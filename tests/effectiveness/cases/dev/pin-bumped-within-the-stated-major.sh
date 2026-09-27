#!/usr/bin/env bash
# kind: near-miss: compatible pin
# scope: in
# expect: silent
# check: grep -q '^20.12' .nvmrc
source "$(dirname "$0")/../../workbench.sh" "$1"
write .nvmrc <<'V'
20.12.1
V
commit "Node 20.12"
