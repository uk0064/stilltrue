# The development family: a Python package with a Makefile, npm scripts, a pinned Node,
# an environment variable, and a guide with headings. Sourced by each dev case, which
# then makes one edit and commits it, so history holds both states.
source "$(dirname "${BASH_SOURCE[0]}")/../fixtures/_common.sh"
init "$1"
write Makefile <<'M'
demo:
	python -m pkg.demo

seed:
	python -m pkg.seed

lint:
	ruff check src
M
write package.json <<'J'
{"name": "workbench", "scripts": {"build": "tsc -p .", "test": "vitest"}}
J
write .nvmrc <<'V'
20.11.0
V
write src/pkg/__init__.py <<'P'
P
write src/pkg/core.py <<'P'
import os


def fold_events(events):
    """Fold a stream of events into a list of states."""
    return [event for event in events]


class Widget:
    def render(self):
        return "widget"


API_KEY = os.environ.get("WORKBENCH_API_KEY")
P
write docs/guide.md <<'G'
# Guide

## Install

Install it with pip.

## Usage

Call it.
G
write CLAUDE.md <<'D'
# Working on workbench

- Run `make demo` to see it working, and `npm run build` before a release.
- The core lives in `src/pkg/core.py`; events are folded by `fold_events()`.
- Set `WORKBENCH_API_KEY` before calling the service.
- Node 20 is required for the front end.
- Installation is covered in [the guide](docs/guide.md#install).
D
commit "workbench"
