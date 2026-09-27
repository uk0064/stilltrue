#!/usr/bin/env bash
# kind: valid example
# scope: in
# expect: silent
# check: [ -e src/pkg/extra.py ]
source "$(dirname "$0")/../../workbench.sh" "$1"
write src/pkg/extra.py <<'P'
VALUE = 1
P
commit "add an unrelated module"
