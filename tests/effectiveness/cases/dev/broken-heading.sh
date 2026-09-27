#!/usr/bin/env bash
# kind: broken heading
# scope: in
# expect: stilltrue/link/rot docs/guide.md#install
# check: ! grep -q '^## Install' docs/guide.md
source "$(dirname "$0")/../../workbench.sh" "$1"
sed 's/^## Install$/## Setup/' docs/guide.md > guide.tmp && mv guide.tmp docs/guide.md
commit "retitle Install as Setup"
