#!/usr/bin/env bash
# docsite: a documentation-heavy repository — relative links with anchors, script
# paths, and a changelog that records what used to be true.
source "$(dirname "$0")/../_bench.sh"
init "$1"

write README.md <<'D'
# lantern

Static documentation for the lantern tool. Start with
[the requirements](docs/install.md#requirements), then read
[the usage guide](docs/usage.md).

Preview the site locally by running `scripts/serve.py`.
D
write docs/install.md <<'D'
# Installing

## Requirements

Python 3.11 or newer.

## Steps

Copy the `docs/` directory into your site root.
D
write docs/usage.md <<'D'
# Usage

Write pages in Markdown under `docs/` and link them relatively.
D
write scripts/serve.py <<'P'
"""Serve the documentation directory on localhost for previewing."""

import functools
import http.server

if __name__ == "__main__":
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory="docs")
    http.server.test(HandlerClass=handler, port=8000)
P
write scripts/legacy_build.py <<'P'
"""Deprecated: the old static build. Kept until 0.4."""
P
write CHANGELOG.md <<'D'
# Changelog

## 0.3.0

- Deprecated `scripts/legacy_build.py`; it will be removed in 0.4.
D
commit "Start the lantern documentation site"

write docs/usage.md <<'D'
# Usage

Write pages in Markdown under `docs/` and link them relatively. Preview them before
publishing.
D
commit "Mention previewing in the usage guide"
