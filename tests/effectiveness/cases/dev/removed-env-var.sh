#!/usr/bin/env bash
# kind: removed env var
# scope: in
# expect: stilltrue/env/rot WORKBENCH_API_KEY
# check: ! grep -rq WORKBENCH_API_KEY src
source "$(dirname "$0")/../../workbench.sh" "$1"
sed 's/os.environ.get("WORKBENCH_API_KEY")/None/' src/pkg/core.py > core.tmp && mv core.tmp src/pkg/core.py
commit "stop reading the API key"
