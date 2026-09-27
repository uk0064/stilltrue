#!/usr/bin/env bash
# kind: near-miss: symbol still defined
# scope: in
# expect: silent
# check: grep -q 'function parseConfig' src/parse.ts && ! grep -q 'function parseConfig' src/config.ts
source "$(dirname "$0")/../../harbor.sh" "$1"
write src/parse.ts <<'T'
export function parseConfig(text: string): Record<string, string> {
  return Object.fromEntries(text.split("\n").map((line) => line.split("=")));
}
T
write src/config.ts <<'T'
export { parseConfig } from "./parse";

export const token = process.env.HARBOR_TOKEN;
T
commit "move parseConfig into parse.ts"
