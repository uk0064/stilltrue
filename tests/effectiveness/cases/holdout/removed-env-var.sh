#!/usr/bin/env bash
# kind: removed env var
# scope: in
# expect: stilltrue/env/rot HARBOR_TOKEN
# check: ! grep -rq HARBOR_TOKEN src
source "$(dirname "$0")/../../harbor.sh" "$1"
sed 's/process.env.HARBOR_TOKEN/undefined/' src/config.ts > t.tmp && mv t.tmp src/config.ts
commit "stop reading the token"
