#!/usr/bin/env bash
# kind: changed pin
# scope: in
# expect: stilltrue/version/rot Node 20
# check: grep -q '^22' .nvmrc
source "$(dirname "$0")/../../workbench.sh" "$1"
write .nvmrc <<'V'
22.3.0
V
commit "move to Node 22"
