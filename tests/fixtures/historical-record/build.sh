#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
# The same rotted path in two documents. A changelog entry naming a file that has since
# moved is the genre working correctly, not a defect (ADR-0012).
write src/engine.py <<'P'
def run():
    pass
P
write docs/guide.md <<'D'
The engine lives in `src/engine.py`.
D
write docs/changelog.md <<'D'
## 0.2.0

Moved the engine out of `src/engine.py`.
D
commit "add the engine"
git mv src/engine.py src/core.py
commit "rename engine to core"
