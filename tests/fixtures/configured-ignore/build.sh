#!/usr/bin/env bash
# The `[ignore]` table, named the way it is documented: by the symbol's name, not by
# the spelling a document happened to use. A bare `LegacyThing` is not a symbol claim
# at all (ADR-0017), so a config that only matched raw claim text could never match
# the documented example.
source "$(dirname "$0")/../_common.sh"
init "$1"
write src/app.py <<'P'
def fold_events(events):
    return {}


def load_config(path):
    return {}


def render_report(rows):
    return ""


class Context:
    def params(self):
        return fold_events([])
P
write CLAUDE.md <<'D'
The entry point is `fold_events()`, configured by `load_config()`.

Sphinx writes the attribute as `Context.params`.

The reporter is `render_report()`, which is not ignored by anything.

Set `ORCHARD_KEY` and `ORCHARD_SECRET` before starting.
D
write stilltrue.toml <<'C'
[ignore]
symbols = ["fold_events", "params"]
env = ["ORCHARD_KEY"]
C
write .env.example <<'E'
ORCHARD_KEY=
ORCHARD_SECRET=
E
commit "the symbols and the variables are all real"
write src/app.py <<'P'
def reduce_events(events):
    return {}


def load_config(path):
    return {}


def write_report(rows):
    return ""


class Context:
    def parameters(self):
        return reduce_events([])
P
write .env.example <<'E'
E
commit "rename the entry point, the attribute, the reporter and drop the variables"
