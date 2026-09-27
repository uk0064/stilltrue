#!/usr/bin/env bash
# Builds "orchard": a realistic Python+TypeScript repo whose documentation has rotted
# in five different ways, each with real git history proving it was once true.
set -euo pipefail
ROOT="$1"
rm -rf "$ROOT"; mkdir -p "$ROOT"; cd "$ROOT"
git init -q -b main
git config user.email dev@orchard.example
git config user.name "Orchard Dev"

commit() { # commit <months-ago> <subject>
  local when
  when=$(python3 -c "
import datetime
d = datetime.datetime(2026,9,24) - datetime.timedelta(days=int($1*30))
print(d.strftime('%Y-%m-%dT12:00:00'))")
  GIT_AUTHOR_DATE="$when" GIT_COMMITTER_DATE="$when" git commit -q -m "$2"
}

# ---------- commit 1: everything the documents claim is true ----------
mkdir -p src/orchard web/src docs
cat > Makefile <<'EOF'
demo:
	python -m orchard.pipeline --demo

lint:
	ruff check src
EOF
cat > package.json <<'EOF'
{
  "name": "orchard-web",
  "scripts": { "build": "tsc", "dev": "vite" }
}
EOF
cat > src/orchard/pipeline.py <<'EOF'
import os

API_KEY = os.environ["ORCHARD_API_KEY"]


def fold_events(events):
    """Collapse an event stream into daily buckets."""
    return {}


def load_config(path):
    return {}


class Harvest:
    def run(self):
        return fold_events([])
EOF
cat > src/orchard/__init__.py <<'EOF'
from .pipeline import fold_events, load_config
EOF
cat > web/src/client.ts <<'EOF'
export function fetchHarvest(id: string): Promise<unknown> {
  return fetch(`/api/harvest/${id}`).then((r) => r.json());
}

export const DEFAULT_TIMEOUT = 30;
EOF
cat > web/src/index.ts <<'EOF'
export { fetchHarvest, DEFAULT_TIMEOUT } from "./client";
EOF
echo "3.11" > .python-version
cat > docs/setup.md <<'EOF'
# Setup

Install, then run the pipeline.
EOF
cat > README.md <<'EOF'
# orchard

Run `make demo` to see the pipeline end to end.

See [the setup guide](./docs/setup.md) for installation.

Set `ORCHARD_API_KEY` before starting. The core entry point is `fold_events()`,
and the web client exposes `fetchHarvest()`.

Requires Python 3.11.
EOF
cat > CLAUDE.md <<'EOF'
# orchard — agent instructions

Build with `make demo`. Lint with `make lint`.

The pipeline entry point is `fold_events()` in `src/orchard/pipeline.py`.
Configuration is read by `load_config()`.

Never commit `ORCHARD_API_KEY`.
EOF
git add -A; commit 14 "Initial orchard pipeline"

# ---------- commit 2: the Makefile target is split (make demo rots) ----------
cat > Makefile <<'EOF'
seed:
	python -m orchard.pipeline --seed

serve:
	python -m orchard.pipeline --serve

lint:
	ruff check src
EOF
git add -A; commit 4 "Split demo into seed and serve"

# ---------- commit 3: the Python symbol is renamed (fold_events rots) ----------
python3 - <<'EOF'
import pathlib
p = pathlib.Path("src/orchard/pipeline.py")
p.write_text(p.read_text().replace("fold_events", "reduce_events"))
p = pathlib.Path("src/orchard/__init__.py")
p.write_text(p.read_text().replace("fold_events", "reduce_events"))
EOF
git add -A; commit 3 "Rename fold_events to reduce_events"

# ---------- commit 4: the setup guide moves (the link rots) ----------
git mv docs/setup.md docs/getting-started.md
git add -A; commit 2 "Move the setup guide to getting-started"

# ---------- commit 5: the env var is dropped (ORCHARD_API_KEY rots) ----------
python3 - <<'EOF'
import pathlib
p = pathlib.Path("src/orchard/pipeline.py")
t = p.read_text().replace('import os\n\nAPI_KEY = os.environ["ORCHARD_API_KEY"]\n\n', 'import os\n\n')
p.write_text(t)
EOF
git add -A; commit 1 "Read credentials from the config file instead"
