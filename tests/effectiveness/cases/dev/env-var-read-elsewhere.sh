#!/usr/bin/env bash
# kind: near-miss: variable still read
# scope: in
# expect: silent
# check: ! grep -q WORKBENCH_API_KEY src/pkg/core.py && grep -q WORKBENCH_API_KEY src/pkg/settings.py
source "$(dirname "$0")/../../workbench.sh" "$1"
sed 's/os.environ.get("WORKBENCH_API_KEY")/None/' src/pkg/core.py > core.tmp && mv core.tmp src/pkg/core.py
write src/pkg/settings.py <<'P'
import os

API_KEY = os.environ["WORKBENCH_API_KEY"]
P
commit "read the API key in settings"
