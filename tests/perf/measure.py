#!/usr/bin/env python3
"""Measure stilltrue against the benchmark tree, and hold it to the warm-run budget.

    tests/perf/measure.py [--runs 10] [--budget-ms 1000] [--out RESULTS.json]

Cold runs pass --no-cache, so every history search is paid for; warm runs read the
cache a previous run wrote. Both are repeated and reported as p50 and p95, with the git
processes each spawned, the size of the tree and its history, and the machine — a
number without its hardware is not a budget. Exits 1 when the warm p50 is over budget.

End-to-end Action duration, which adds the download and the history fetch, cannot be
measured here; the published-smoke workflow records it per platform once a release
exists.
"""

import argparse
import json
import os
import platform
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def percentile(values, share):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(share * (len(ordered) - 1))))]


def run(binary, repo, report, cold, env):
    args = [str(binary), "--report-file", str(report)] + (["--no-cache"] if cold else [])
    started = time.perf_counter()
    done = subprocess.run(args, cwd=repo, capture_output=True, env=env)
    elapsed = (time.perf_counter() - started) * 1000
    if done.returncode >= 2:
        sys.exit(f"stilltrue failed (exit {done.returncode}): {done.stderr.decode()[:500]}")
    data = json.loads(report.read_text())
    return elapsed, data


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--budget-ms", type=float, default=1000.0)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/stilltrue")
    args = parser.parse_args()
    if not args.binary.is_file():
        sys.exit(f"no binary at {args.binary}: cargo build --release")

    with tempfile.TemporaryDirectory() as scratch:
        scratch = Path(scratch)
        repo = scratch / "tree"
        subprocess.run([sys.executable, str(HERE / "generate.py"), str(repo)], check=True, capture_output=True)
        env = dict(os.environ, HOME=str(scratch / "home"), XDG_CACHE_HOME=str(scratch / "home/.cache"))
        report = scratch / "report.json"
        cold, warm = [], []
        for _ in range(args.runs):
            elapsed, data = run(args.binary, repo, report, True, env)
            cold.append((elapsed, data))
        run(args.binary, repo, report, False, env)  # prime the cache
        for _ in range(args.runs):
            elapsed, data = run(args.binary, repo, report, False, env)
            warm.append((elapsed, data))
        last = warm[-1][1]
        files = sum(1 for p in repo.rglob("*") if p.is_file() and ".git" not in p.parts)
        commits = int(subprocess.run(["git", "rev-list", "--count", "HEAD"], cwd=repo, capture_output=True, text=True).stdout)

    summary = lambda runs: {
        "p50Ms": round(statistics.median(t for t, _ in runs)),
        "p95Ms": round(percentile([t for t, _ in runs], 0.95)),
        "gitProcesses": runs[-1][1]["history"]["gitProcesses"],
        "timings": runs[-1][1]["run"]["timings"],
    }
    result = {
        "schema": "stilltrue/perf-run",
        "version": 1,
        "status": last["status"],
        "tool": {
            "version": subprocess.run([str(args.binary), "--version"], capture_output=True, text=True).stdout.split()[-1],
            "revision": subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip(),
        },
        "machine": {"platform": platform.platform(), "machine": platform.machine(), "processors": os.cpu_count()},
        "tree": {
            "files": files,
            "documents": last["coverage"]["documents"]["read"],
            "claims": last["coverage"]["claims"]["extracted"],
            "findings": len(last["findings"]),
            "commits": commits,
        },
        "runs": args.runs,
        "cold": summary(cold),
        "warm": summary(warm),
        "budget": {"warmP50Ms": args.budget_ms},
    }
    result["budget"]["met"] = result["warm"]["p50Ms"] < args.budget_ms
    text = json.dumps(result, indent=2)
    if args.out:
        args.out.write_text(text + "\n")
    print(text)
    if result["status"] != "complete":
        sys.exit(f"the benchmark run was {result['status']}, so its times describe an incomplete scan")
    if not result["budget"]["met"]:
        sys.exit(f"warm p50 {result['warm']['p50Ms']} ms is over the {args.budget_ms:.0f} ms budget")


if __name__ == "__main__":
    main()
