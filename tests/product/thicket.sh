#!/usr/bin/env bash
# Builds "thicket": a monorepo shaped to exercise the parts of the tool the orchard
# fixture never reaches — nested manifests, Tier C abstentions, version pins, in-document
# anchors, the default file set, and non-ASCII columns.
set -euo pipefail
ROOT="$1"
rm -rf "$ROOT"; mkdir -p "$ROOT"; cd "$ROOT"
git init -q -b main
git config user.email dev@thicket.example
git config user.name "Thicket Dev"

commit() { # commit <months-ago> <subject>
  local when
  when=$(python3 -c "
import datetime
d = datetime.datetime(2026,9,24) - datetime.timedelta(days=int($1*30))
print(d.strftime('%Y-%m-%dT12:00:00'))")
  GIT_AUTHOR_DATE="$when" GIT_COMMITTER_DATE="$when" git commit -q -m "$2"
}

mkdir -p packages/api docs .cursor/rules skills/deploy config

# ---------- commit 1: a monorepo where every claim is true ----------
cat > package.json <<'EOF'
{ "name": "thicket", "scripts": { "build": "tsc -b", "test": "vitest" } }
EOF
cat > packages/api/package.json <<'EOF'
{ "name": "@thicket/api", "scripts": { "serve": "node server.js", "migrate": "node migrate.js" } }
EOF
echo "22.4.0" > .nvmrc
echo "flask" > config/requirements.txt

cat > packages/api/README.md <<'EOF'
# @thicket/api

Run `pnpm run serve` to start the API, and `pnpm run migrate` before first boot.
EOF

cat > README.md <<'EOF'
# thicket

Build with `pnpm run build`. Test with `pnpm run test`.

Requires Node 22.4.0.

## Installation

Install the dependencies, then read the [configuration](#configuration) section.

## Configuration

Copy `requirements.txt` into your own project and edit it.
The tracked one is `config/requirements.txt`.

See [the deploy guide](./docs/deploy) for the rest.

Naïve café — the ‹accented› column must land on the claim `docs/unicode-check.md`.
EOF

cat > docs/deploy.md <<'EOF'
# Deploy

Run `pnpm run build` first.
EOF

cat > CHANGELOG.md <<'EOF'
# Changelog

## 0.2.0
- Removed `packages/legacy/index.ts` and dropped `pnpm run bundle`.
EOF

cat > .cursor/rules/style.md <<'EOF'
# Style

Always run `pnpm run test` before committing.
EOF

cat > skills/deploy/SKILL.md <<'EOF'
# Deploy skill

Invoke `pnpm run build`, then `pnpm run serve` from `packages/api`.
EOF

git add -A; commit 10 "Initial thicket monorepo"

# ---------- commit 2: the nested script is renamed (nested anchoring rots) ----------
python3 - <<'EOF'
import pathlib, json
p = pathlib.Path("packages/api/package.json")
d = json.loads(p.read_text())
d["scripts"]["start"] = d["scripts"].pop("serve")
p.write_text(json.dumps(d, indent=2) + "\n")
EOF
git add -A; commit 3 "Rename the api serve script to start"

# ---------- commit 3: the Node pin moves (the version claim rots) ----------
echo "24.2.0" > .nvmrc
git add -A; commit 2 "Move to Node 24"

# ---------- commit 4: a heading is renamed (the in-document anchor rots) ----------
python3 - <<'EOF'
import pathlib
p = pathlib.Path("README.md")
p.write_text(p.read_text().replace("## Configuration", "## Configuring thicket"))
EOF
git add -A; commit 1 "Rename the Configuration heading"
