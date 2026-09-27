#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write .nvmrc <<'V'
18
V
write package.json <<'P'
{"engines": {"node": ">=18"}}
P
write CLAUDE.md <<'D'
This project needs Node 18 to build.
D
commit "pin node 18"
write .nvmrc <<'V'
24
V
write package.json <<'P'
{"engines": {"node": ">=20"}}
P
commit "upgrade to node 24"
