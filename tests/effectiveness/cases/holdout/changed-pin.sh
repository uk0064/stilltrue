#!/usr/bin/env bash
# kind: changed pin
# scope: in
# expect: stilltrue/version/rot Node 22
# check: grep -q '^24' .node-version
source "$(dirname "$0")/../../harbor.sh" "$1"
write .node-version <<'V'
24.0.0
V
commit "move to Node 24"
