#!/usr/bin/env bash
# kind: renamed path
# scope: in
# expect: stilltrue/path/rot src/pkg/core.py
# check: [ ! -e src/pkg/core.py ] && [ -e src/pkg/engine.py ]
source "$(dirname "$0")/../../workbench.sh" "$1"
git mv src/pkg/core.py src/pkg/engine.py
commit "rename core to engine"
