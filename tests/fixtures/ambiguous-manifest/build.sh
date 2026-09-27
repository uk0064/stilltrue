#!/usr/bin/env bash
source "$(dirname "$0")/../_common.sh"
init "$1"
# Both once existed and neither does now. The qualified one is a claim about this
# repository; the bare one is the shape pip-tools' README makes, and could be about
# anyone's (ADR-0011).
write requirements.txt <<'R'
click
R
write config/requirements.txt <<'R'
click
R
write README.md <<'D'
Install from `requirements.txt`, or from `config/requirements.txt` on CI.
D
commit "add the requirements files"
git rm -q requirements.txt config/requirements.txt
commit "move to pyproject"
