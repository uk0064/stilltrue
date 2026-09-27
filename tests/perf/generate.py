#!/usr/bin/env python3
"""Build the performance benchmark tree: 500 documents across a 5,000-file repository,
with real history.

    tests/perf/generate.py DIR [--documents 500] [--files 5000] [--commits 40]

Documents make every kind of claim — paths, make targets, npm scripts, environment
variables, Python symbols, pinned versions and links with anchors — and a fixed share
of them are broken by a later commit, so a run exercises the history stage as well as
extraction and resolution. The tree is deterministic: the same arguments build the
same repository, so measurements from different days describe the same input.
"""

import argparse
import os
import subprocess
from pathlib import Path

ENV = dict(
    os.environ,
    GIT_AUTHOR_NAME="perf",
    GIT_AUTHOR_EMAIL="perf@example.com",
    GIT_COMMITTER_NAME="perf",
    GIT_COMMITTER_EMAIL="perf@example.com",
)


def git(root, *args):
    subprocess.run(["git", *args], cwd=root, env=ENV, check=True, capture_output=True)


def commit(root, message, stamp):
    git(root, "add", "-A")
    env = dict(ENV, GIT_AUTHOR_DATE=f"@{stamp} +0000", GIT_COMMITTER_DATE=f"@{stamp} +0000")
    subprocess.run(["git", "commit", "-q", "-m", message], cwd=root, env=env, check=True)


def build(root, documents, files, commits):
    root.mkdir(parents=True)
    git(root, "init", "-q", "-b", "main")
    # Forty commits in a few seconds trigger git's automatic gc, which detaches and holds
    # a lock the next commit then fails on — intermittently, which is the worst way.
    git(root, "config", "gc.auto", "0")
    git(root, "config", "maintenance.auto", "false")
    # Five files at the root: Makefile, package.json, .nvmrc, README.md and CHANGES.md.
    sources = files - documents - 5
    modules = max(1, -(-sources // 50))
    targets = [f"task{i:03d}" for i in range(60)]
    (root / "Makefile").write_text("".join(f"{t}:\n\t@echo {t}\n\n" for t in targets))
    (root / "package.json").write_text(
        '{"name": "perf", "scripts": {' + ", ".join(f'"script{i}": "echo {i}"' for i in range(40)) + "}}\n"
    )
    (root / ".nvmrc").write_text("20.11.0\n")
    (root / "README.md").write_text("# Perf\n\nSee [the docs](docs/section00/page000.md).\n")
    written = 0
    for m in range(modules):
        package = root / "src" / f"mod{m:03d}"
        package.mkdir(parents=True)
        for f in range(50):
            if written >= sources:
                break
            (package / f"file{f:02d}.py").write_text(
                "import os\n\n"
                f"SETTING = os.environ.get('PERF_VAR_{m:03d}_{f:02d}')\n\n\n"
                f"def func_{m:03d}_{f:02d}(value):\n    return value\n\n\n"
                f"class Model{m:03d}{f:02d}:\n    def run(self):\n        return 1\n"
            )
            written += 1
    for d in range(documents):
        section = root / "docs" / f"section{d // 50:02d}"
        section.mkdir(parents=True, exist_ok=True)
        m, f = d % modules, d % 50
        other = (d + 1) % documents
        (section / f"page{d % 50:03d}.md").write_text(
            f"# Page {d}\n\n## Setup\n\n"
            f"Run `make {targets[d % len(targets)]}` and `npm run script{d % 40}`.\n\n"
            f"The code is in `src/mod{m:03d}/file{f:02d}.py`, and `func_{m:03d}_{f:02d}()` does the work.\n\n"
            f"Set `PERF_VAR_{m:03d}_{f:02d}` first. Node 20 is required.\n\n"
            f"Next: [page {other}](../section{other // 50:02d}/page{other % 50:03d}.md#setup).\n"
        )
    stamp = 1_700_000_000
    commit(root, "initial tree", stamp)
    # Break a fixed share of claims in one commit, so rot exists and has history.
    kept = [t for i, t in enumerate(targets) if i % 10]
    (root / "Makefile").write_text("".join(f"{t}:\n\t@echo {t}\n\n" for t in kept))
    for m in range(0, modules, 10):
        path = root / "src" / f"mod{m:03d}" / "file00.py"
        if path.exists():
            path.write_text(path.read_text().replace(f"def func_{m:03d}_00", f"def renamed_{m:03d}_00"))
    commit(root, "drop every tenth target and rename some functions", stamp + 100)
    # History depth: commits that touch nothing any claim names.
    for c in range(commits):
        (root / "CHANGES.md").write_text(f"change {c}\n")
        commit(root, f"housekeeping {c}", stamp + 200 + c * 100)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("dir", type=Path)
    parser.add_argument("--documents", type=int, default=500)
    parser.add_argument("--files", type=int, default=5000)
    parser.add_argument("--commits", type=int, default=40)
    args = parser.parse_args()
    build(args.dir, args.documents, args.files, args.commits)
    count = sum(1 for p in args.dir.rglob("*") if p.is_file() and ".git" not in p.parts)
    print(f"{args.dir}: {count} files, {args.documents} documents")


if __name__ == "__main__":
    main()
