#!/usr/bin/env bash
# Every claim type `--fix` can rewrite, in one repository, each with exactly one
# candidate. `--fix` was only ever tested on commands, and every other type wrote
# something that did not resolve: the tool name dropped from a version, a heading slug
# put where a whole link destination belonged, and a repository-relative path written
# into a link that resolves from its own directory.
source "$(dirname "$0")/../_common.sh"
init "$1"

write .nvmrc <<'V'
18
V
write docs/guide.md <<'G'
## Install

How to install it.
G
write docs/old/x.md <<'X'
# X
X
write docs/a.md <<'A'
See [x](old/x.md).
A
write README.md <<'R'
Needs Node 18 to build.

See [the guide](docs/guide.md#install).
R
commit "initial"

# One commit moves all three, so every finding has history and lands as rot.
write .nvmrc <<'V'
24
V
write docs/guide.md <<'G'
## Setup

How to install it.
G
mkdir -p docs/new
mv docs/old/x.md docs/new/x.md
commit "upgrade node, retitle the heading, move the page"
