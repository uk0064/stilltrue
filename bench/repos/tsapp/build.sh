#!/usr/bin/env bash
# tsapp: a TypeScript package whose workflow lives in package.json scripts. Clean at
# HEAD. Nothing here needs node to build: the benchmark's checks read the files.
source "$(dirname "$0")/../_bench.sh"
init "$1"

write package.json <<'J'
{
  "name": "tsapp",
  "version": "1.0.0",
  "private": true,
  "scripts": {
    "build": "tsc -p .",
    "test": "node --test dist/",
    "typecheck": "tsc --noEmit -p ."
  },
  "devDependencies": {
    "typescript": "5.4.5"
  }
}
J
write tsconfig.json <<'J'
{
  "compilerOptions": {
    "target": "es2022",
    "module": "commonjs",
    "outDir": "dist",
    "strict": true
  },
  "include": ["src"]
}
J
write src/format.ts <<'T'
export function formatDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}
T
write src/index.ts <<'T'
export { formatDate } from "./format";
T
write AGENTS.md <<'D'
# tsapp

Run `npm run typecheck` and `npm run test` before you finish. Compiled JavaScript
is written to the dist directory.

Dates are formatted by `formatDate()`, which returns an ISO day such as 2024-01-31.
D
commit "Start tsapp with a date formatter"

write src/format.ts <<'T'
/** The calendar day of `date`, in UTC. */
export function formatDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}
T
commit "Document that formatDate works in UTC"
