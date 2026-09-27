#!/usr/bin/env bash
# kind: broken heading
# scope: in
# expect: stilltrue/link/rot docs/usage.md#configuration
# check: ! grep -q '^## Configuration' docs/usage.md
source "$(dirname "$0")/../../harbor.sh" "$1"
sed 's/^## Configuration$/## Settings/' docs/usage.md > u.tmp && mv u.tmp docs/usage.md
commit "retitle Configuration as Settings"
