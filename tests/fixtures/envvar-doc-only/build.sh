#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
write app.py <<'P'
import os

KEY = os.environ["GEMINI_API_KEY"]
P
write CLAUDE.md <<'D'
Set `GEMINI_API_KEY` before running.
D
write docs/setup.md <<'D'
# Setup

Remember GEMINI_API_KEY.
D
commit "read the key"
write app.py <<'P'
import os

KEY = os.environ["GOOGLE_API_KEY"]
P
commit "rename the key"
