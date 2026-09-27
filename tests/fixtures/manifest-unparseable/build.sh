#!/usr/bin/env bash
# Near-miss for command rot (ADR-0019): a manifest that stops parsing has an unknown
# target list, not an empty one. Read as empty, `build` looked missing, and `"build":`
# is in the manifest's own history, so a trailing comma was reported as rot.
source "$(dirname "$0")/../_common.sh"
init "$1"
write package.json <<'J'
{"scripts": {"build": "tsc"}}
J
write README.md <<'D'
Build it with `npm run build`.
D
commit "add build script"
write package.json <<'J'
{"scripts": {"build": "tsc", "lint": "eslint .",}}
J
commit "add a lint script, with a trailing comma"
