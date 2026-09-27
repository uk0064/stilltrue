#!/usr/bin/env bash
# kind: near-miss: same slug
# scope: in
# expect: silent
# check: grep -q '^## INSTALL$' docs/guide.md
source "$(dirname "$0")/../../workbench.sh" "$1"
sed 's/^## Install$/## INSTALL/' docs/guide.md > guide.tmp && mv guide.tmp docs/guide.md
commit "shout the heading"
