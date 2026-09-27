#!/usr/bin/env python3
"""Render a pinned corpus run as Markdown, with every rate beside its denominator.

    tests/corpus/summarise.py RESULTS.json

Precision is computed over reviewed findings only: an unreviewed finding is neither a
true nor a false positive, and is counted on its own line. A failed or incomplete
repository is listed as such, never among the quiet ones.
"""

import json
import sys


def ratio(part, whole):
    if whole == 0:
        return "no reviewed findings"
    return f"{part}/{whole} ({100 * part / whole:.0f}%)"


def summarise(result):
    repos = result["repositories"]
    findings = [f for r in repos for f in r.get("findings", [])]
    tp = sum(f["label"] == "true-positive" for f in findings)
    fp = sum(f["label"] == "false-positive" for f in findings)
    unreviewed = sum(f["label"] == "unreviewed" for f in findings)
    failed = [r["name"] for r in repos if r["status"] == "failed"]
    incomplete = [r["name"] for r in repos if r["status"] in ("incomplete", "no-input")]
    quiet = [r["name"] for r in repos if r["status"] == "complete" and not r.get("findings")]
    lines = [
        f"Pinned corpus run: **{result['status']}** — stilltrue {result['tool']['version']} "
        f"({result['tool']['revision'][:12]}), {result['machine']['platform']}, "
        f"{result['startedAt']}.",
        "",
        "| Repository | Commit | Status | History | Documents | Abstentions | Findings | Cold | Warm |",
        "|---|---|---|---|---:|---:|---:|---:|---:|",
    ]
    for r in repos:
        lines.append(
            "| {name} | {commit} | {status} | {history} | {docs} | {abst} | {n} | {cold} | {warm} |".format(
                name=r["name"],
                commit=r["commit"][:12],
                status=r["status"] + (f" ({r['error']})" if r.get("error") else ""),
                history=r.get("history", "—"),
                docs=r.get("documents", "—"),
                abst=r.get("abstentions", "—"),
                n=len(r.get("findings", [])),
                cold=f"{r['coldMs']} ms" if "coldMs" in r else "—",
                warm=f"{r['warmMs']} ms" if "warmMs" in r else "—",
            )
        )
    warm = [r["warmMs"] for r in repos if "warmMs" in r]
    cold = [r["coldMs"] for r in repos if "coldMs" in r]
    lines += [
        "",
        f"- Findings: {len(findings)} — {tp} true positive, {fp} false positive, "
        f"{unreviewed} unreviewed.",
        f"- Precision over reviewed findings: {ratio(tp, tp + fp)}.",
        f"- Repositories complete and quiet: {len(quiet)} of {len(repos)}"
        + (f"; incomplete: {', '.join(incomplete)}" if incomplete else "; none incomplete")
        + (f"; failed: {', '.join(failed)}" if failed else "; none failed")
        + ".",
        f"- Abstentions (skipped or ambiguous claims): {sum(r.get('abstentions', 0) for r in repos)}.",
        f"- Time: cold {sum(cold)} ms in total (slowest {max(cold, default=0)} ms), "
        f"warm {sum(warm)} ms in total (slowest {max(warm, default=0)} ms).",
    ]
    for f in findings:
        lines.append(f"  - {f['label']}: `{f['claim']}` in {f['file']}:{f['line']} — {f['note'] or 'not yet reviewed'}")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} RESULTS.json")
    sys.stdout.write(summarise(json.loads(open(sys.argv[1]).read())))
