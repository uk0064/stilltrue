#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write package.json <<'P'
{"scripts": {"root-only": "echo"}}
P
write packages/a/package.json <<'P'
{"scripts": {"build": "tsc"}}
P
write packages/a/README.md <<'D'
Run `pnpm run build` in this package.
D
write README.md <<'D'
Run `pnpm run build` from the root.
D
commit "set up the workspace"
