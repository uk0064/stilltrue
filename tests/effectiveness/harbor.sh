# The held-out family: a TypeScript package run with just, a pinned Node via
# .node-version, an environment variable read through process.env, and a usage page.
# Nothing here was used to tune the rules the dev family exercises.
source "$(dirname "${BASH_SOURCE[0]}")/../fixtures/_common.sh"
init "$1"
write justfile <<'J'
serve:
    node dist/server.js

check:
    tsc --noEmit
J
write .node-version <<'V'
22.4.1
V
write src/config.ts <<'T'
export function parseConfig(text: string): Record<string, string> {
  return Object.fromEntries(text.split("\n").map((line) => line.split("=")));
}

export const token = process.env.HARBOR_TOKEN;
T
write src/index.ts <<'T'
export { parseConfig } from "./config";
T
write docs/usage.md <<'U'
# Usage

## Configuration

Write a config file.
U
write AGENTS.md <<'D'
# Harbor

- Start it with `just serve`; `just check` type-checks.
- Configuration is parsed by `parseConfig()` in `src/config.ts`.
- `HARBOR_TOKEN` must be set.
- Node 22 is required.
- See [usage](docs/usage.md#configuration).
D
commit "harbor"
