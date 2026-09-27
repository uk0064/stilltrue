#!/usr/bin/env bash
# docsite, after a commit renamed the install guide's "Requirements" heading to
# "Prerequisites": the README's anchored link is stale before any session starts.
HERE="$(cd "$(dirname "$0")" && pwd)"
bash "$HERE/../build.sh" "$1"
source "$HERE/../../_bench.sh"
cd "$1"

write docs/install.md <<'D'
# Installing

## Prerequisites

Python 3.11 or newer.

## Steps

Copy the `docs/` directory into your site root.
D
commit "Call the requirements prerequisites"
