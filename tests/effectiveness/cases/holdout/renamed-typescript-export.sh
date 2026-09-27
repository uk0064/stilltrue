#!/usr/bin/env bash
# kind: removed symbol
# scope: in
# expect: stilltrue/symbol/rot parseConfig()
# check: ! grep -rq parseConfig src
source "$(dirname "$0")/../../harbor.sh" "$1"
sed 's/parseConfig/loadConfig/' src/config.ts > t.tmp && mv t.tmp src/config.ts
sed 's/parseConfig/loadConfig/' src/index.ts > t.tmp && mv t.tmp src/index.ts
commit "rename parseConfig to loadConfig"
