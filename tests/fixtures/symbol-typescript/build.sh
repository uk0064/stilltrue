#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write src/engine.ts <<'T'
export function foldEvents(events: string[]) {
  return events;
}
T
write src/index.ts <<'T'
export { foldEvents } from "./engine";
T
write CLAUDE.md <<'D'
The entry point is `foldEvents()`.
D
commit "add foldEvents"
write src/engine.ts <<'T'
export function collapseEvents(events: string[]) {
  return events;
}
T
commit "rename foldEvents to collapseEvents"
