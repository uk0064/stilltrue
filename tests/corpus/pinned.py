#!/usr/bin/env python3
"""The pinned corpus.

tests/corpus/run.sh lints whatever the fifteen repositories look like today, so its
counts drift. This lints each one at the exact commit in pinned.tsv, and records per
repository the tool version and revision, the operational status and history state
from the run report, its diagnostics and coverage, every finding with its review label
from labels.tsv, and cold and warm run times. A repository that could not be cloned,
checked out or scanned is recorded as failed, never as quiet.

    tests/corpus/pinned.py [RESULTS.json]
    tests/corpus/summarise.py RESULTS.json

Clones are full (Tier A needs history) and shared with run.sh under
$STILLTRUE_CORPUS, default $TMPDIR/stilltrue-corpus. The binary is
target/release/stilltrue unless $STILLTRUE_BIN says otherwise.
"""

import json
import os
import platform
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def table(path):
    rows = []
    for line in path.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            rows.append(line.split("\t"))
    return rows


def labels():
    out = {}
    for repo, fingerprint, label, note in table(HERE / "labels.tsv"):
        assert label in ("true-positive", "false-positive"), label
        out[(repo, fingerprint)] = (label, note)
    return out


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)


def scan(binary, repo, report, cold):
    args = [str(binary), "--report-file", str(report)]
    if cold:
        # --no-cache ignores the stored history results: every search is paid for.
        args.append("--no-cache")
    started = time.perf_counter()
    done = subprocess.run(args, cwd=repo, capture_output=True, text=True)
    return done, round((time.perf_counter() - started) * 1000)


def run_one(binary, work, name, url, commit, known):
    entry = {"name": name, "url": url, "commit": commit, "status": "failed"}
    repo = work / name
    if not (repo / ".git").exists():
        cloned = git("clone", "--quiet", url, str(repo))
        if cloned.returncode != 0:
            entry["error"] = f"clone failed: {cloned.stderr.strip()[:300]}"
            return entry
    if git("cat-file", "-e", f"{commit}^{{commit}}", cwd=repo).returncode != 0:
        git("fetch", "--quiet", "origin", cwd=repo)
    checkout = git("checkout", "--quiet", "--detach", commit, cwd=repo)
    if checkout.returncode != 0:
        entry["error"] = f"checkout failed: {checkout.stderr.strip()[:300]}"
        return entry
    with tempfile.TemporaryDirectory() as scratch:
        report = Path(scratch) / "report.json"
        _, cold_ms = scan(binary, repo, report, cold=True)
        done, warm_ms = scan(binary, repo, report, cold=False)
        entry.update(coldMs=cold_ms, warmMs=warm_ms, exitCode=done.returncode)
        if done.returncode >= 2 or not report.exists():
            entry["error"] = f"exit {done.returncode}: {done.stderr.strip()[:300]}"
            return entry
        data = json.loads(report.read_text())
    claims = data["coverage"]["claims"]
    entry.update(
        status=data["status"],
        history=data["history"]["state"],
        diagnostics=[d["code"] for d in data["diagnostics"]],
        documents=data["coverage"]["documents"]["read"],
        claims=claims,
        abstentions=claims["skipped"] + claims["ambiguous"],
        findings=[
            {
                "fingerprint": f["fingerprint"],
                "ruleId": f["ruleId"],
                "file": f["file"],
                "line": f["line"],
                "claim": f["claim"],
                "label": known.get((name, f["fingerprint"]), ("unreviewed", ""))[0],
                "note": known.get((name, f["fingerprint"]), ("", ""))[1],
            }
            for f in data["findings"]
        ],
    )
    return entry


def main():
    binary = Path(os.environ.get("STILLTRUE_BIN", ROOT / "target/release/stilltrue"))
    if not binary.is_file():
        sys.exit(f"no binary at {binary}: cargo build --release")
    work = Path(os.environ.get("STILLTRUE_CORPUS", Path(tempfile.gettempdir()) / "stilltrue-corpus"))
    work.mkdir(parents=True, exist_ok=True)
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else work / "pinned-results.json"
    version = subprocess.run([str(binary), "--version"], capture_output=True, text=True).stdout.split()
    known = labels()
    started = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    repositories = []
    for name, url, commit in table(HERE / "pinned.tsv"):
        print(f"{name} @ {commit[:12]}", file=sys.stderr)
        repositories.append(run_one(binary, work, name, url, commit, known))
    everything_ran = all(r["status"] != "failed" for r in repositories)
    all_complete = all(r["status"] == "complete" for r in repositories)
    result = {
        "schema": "stilltrue/corpus-run",
        "version": 1,
        # The suite's own operational status, beside the product's: a corpus run in
        # which anything failed or ran incomplete is not evidence of precision.
        "status": "complete" if everything_ran and all_complete else "incomplete",
        "tool": {
            "version": version[1] if len(version) > 1 else None,
            "revision": git("rev-parse", "HEAD", cwd=ROOT).stdout.strip(),
        },
        "machine": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "processors": os.cpu_count(),
        },
        "startedAt": started,
        "finishedAt": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "repositories": repositories,
    }
    out.write_text(json.dumps(result, indent=2) + "\n")
    print(out)


if __name__ == "__main__":
    main()
