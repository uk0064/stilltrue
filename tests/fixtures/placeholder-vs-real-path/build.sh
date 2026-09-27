#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
# Both paths sit in the same document and share a prefix, so the placeholder rule
# has to discriminate on the angle brackets alone.
write packages/core/index.ts <<'T'
export const core = true;
T
write CLAUDE.md <<'D'
Every package follows the same layout: `packages/<name>/index.ts`.

The entry point is `packages/core/index.ts`.
D
commit "add the core package"
git rm -q packages/core/index.ts
commit "move core out of packages"
