#!/usr/bin/env bash
# kind: renamed path
# scope: in
# expect: stilltrue/path/rot src/config.ts
# check: [ ! -e src/config.ts ] && grep -q ./settings src/index.ts
source "$(dirname "$0")/../../harbor.sh" "$1"
git mv src/config.ts src/settings.ts
sed 's#./config#./settings#' src/index.ts > t.tmp && mv t.tmp src/index.ts
commit "rename config.ts to settings.ts"
