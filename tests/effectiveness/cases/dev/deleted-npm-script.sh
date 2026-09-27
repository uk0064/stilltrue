#!/usr/bin/env bash
# kind: deleted command target
# scope: in
# expect: stilltrue/command/rot npm run build
# check: ! grep -q '"build"' package.json
source "$(dirname "$0")/../../workbench.sh" "$1"
write package.json <<'J'
{"name": "workbench", "scripts": {"compile": "tsc -p .", "test": "vitest"}}
J
commit "rename build to compile"
