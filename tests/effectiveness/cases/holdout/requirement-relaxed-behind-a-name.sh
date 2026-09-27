#!/usr/bin/env bash
# kind: semantic error
# scope: out
# expect: silent
# check: grep -q anonymous src/config.ts
source "$(dirname "$0")/../../harbor.sh" "$1"
sed 's/process.env.HARBOR_TOKEN;/process.env.HARBOR_TOKEN ?? "anonymous";/' src/config.ts > t.tmp && mv t.tmp src/config.ts
commit "make the token optional; the document still says it must be set"
