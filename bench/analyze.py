"""Analysis of one benchmark run set.

Reads `manifest.json` and `outcomes.jsonl` from a run directory and writes
`report.json` and `summary.md` beside them. Every host is analysed on its own; no
pooled cross-host result is computed.

- Success rates per host and arm, with denominators and exclusions.
- Paired per-task differences: D−A (primary), C−A and B−A (mechanism). Each task's
  repeats are averaged first and travel together; 95% intervals come from a
  repository-clustered bootstrap that resamples families, then tasks within them.
- Unnecessary-block frequency per eligible completion boundary, with an upper 95%
  bound, and the fraction of blocking decisions judged unnecessary.
- The promotion gates, per host, against the thresholds frozen in bench/frozen.toml;
  "inconclusive" when the data are below its minimums.

    python3 bench/analyze.py OUT_DIR [--replicates 2000] [--seed N]
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import random
import re
import statistics
import sys
from pathlib import Path

COUNTED = ("completed", "task-failure", "budget-exhausted")
EXCLUDED = ("infrastructure-failure", "interrupted")
PAIRS = (("D", "A", "primary"), ("C", "A", "mechanism"), ("B", "A", "mechanism"))
ARM_IDS = ("A", "B", "C", "D")
FALSE_BLOCK_REGRESSION = ("clean-control", "near-miss", "preexisting-rot")
FINDING = re.compile(r"^- (?P<file>.+?):(?P<line>\d+) \S+: ")
DEFAULT_FROZEN = {
    "gates": {
        "min_improvement": 0.10,
        "confidence": 0.95,
        "max_unnecessary_block_upper": 0.05,
        "max_clean_control_overhead": 0.10,
    },
    "conclusive": {"min_families": 4, "min_tasks": 12, "min_repeats": 3},
}


# -- small statistics ----------------------------------------------------------------


def rate(k: int, n: int) -> float | None:
    return k / n if n else None


def percentile(values: list[float], q: float) -> float | None:
    """Linear-interpolated percentile, q in [0, 100]."""
    if not values:
        return None
    s = sorted(values)
    pos = (len(s) - 1) * q / 100
    lo, hi = math.floor(pos), math.ceil(pos)
    return s[lo] + (s[hi] - s[lo]) * (pos - lo)


def nearest_rank(values: list[float], q: float) -> float | None:
    """The p-th percentile by nearest rank: an observed value, as for latency."""
    if not values:
        return None
    s = sorted(values)
    return s[max(0, math.ceil(q / 100 * len(s)) - 1)]


def _log_binom_pmf(i: int, n: int, p: float) -> float:
    if p <= 0:
        return 0.0 if i == 0 else -math.inf
    if p >= 1:
        return 0.0 if i == n else -math.inf
    return (
        math.lgamma(n + 1) - math.lgamma(i + 1) - math.lgamma(n - i + 1)
        + i * math.log(p) + (n - i) * math.log1p(-p)
    )


def binom_cdf(k: int, n: int, p: float) -> float:
    return min(1.0, sum(math.exp(_log_binom_pmf(i, n, p)) for i in range(k + 1)))


def clopper_pearson_upper(x: int, n: int, confidence: float = 0.95) -> float | None:
    """The upper limit of the exact two-sided interval for x events in n trials."""
    if n == 0:
        return None
    if x >= n:
        return 1.0
    alpha = (1 - confidence) / 2
    lo, hi = x / n, 1.0
    for _ in range(100):
        mid = (lo + hi) / 2
        if binom_cdf(x, n, mid) > alpha:
            lo = mid
        else:
            hi = mid
    return hi


def resample(rng: random.Random, families: dict[str, list[str]]) -> list[str]:
    """One two-stage cluster resample: families with replacement, then tasks with
    replacement within each chosen family. Returns task ids, with multiplicity; a task
    carries all of its repeats, so repeats are never split or resampled on their own."""
    names = sorted(families)
    out: list[str] = []
    for family in (rng.choice(names) for _ in names):
        tasks = sorted(families[family])
        out.extend(rng.choice(tasks) for _ in tasks)
    return out


def cluster_bootstrap(values: dict[str, float], family_of: dict[str, str], *, seed, replicates: int,
                      confidence: float = 0.95) -> tuple[float | None, float | None]:
    """Percentile interval for the mean of per-task `values`."""
    if not values:
        return None, None
    families: dict[str, list[str]] = {}
    for task in values:
        families.setdefault(family_of[task], []).append(task)
    rng = random.Random(seed)
    means = []
    for _ in range(replicates):
        picked = resample(rng, families)
        means.append(statistics.fmean(values[t] for t in picked))
    tail = (1 - confidence) / 2 * 100
    return percentile(means, tail), percentile(means, 100 - tail)


# -- reading outcomes ----------------------------------------------------------------


def load(out_dir: Path) -> tuple[dict, list[dict]]:
    manifest = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))
    path = out_dir / manifest.get("outcomes_file", "outcomes.jsonl")
    outcomes = [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines() if l.strip()]
    return manifest, outcomes


def counted(outcomes: list[dict], host: str, arm: str | None = None) -> list[dict]:
    return [
        o for o in outcomes
        if o["host"] == host and o["status"] in COUNTED and (arm is None or o["arm"] == arm)
    ]


def _summary(runs: list[dict]) -> dict:
    n = len(runs)
    succ = sum(1 for o in runs if o.get("success"))
    code = sum(1 for o in runs if (o.get("grading") or {}).get("code_success"))
    docs = sum(1 for o in runs if (o.get("grading") or {}).get("docs_correct"))
    viol = sum(1 for o in runs if (o.get("grading") or {}).get("violations"))
    return {
        "denominator": n,
        "successes": succ,
        "success_rate": rate(succ, n),
        "code_successes": code,
        "code_success_rate": rate(code, n),
        "docs_correct": docs,
        "docs_correct_rate": rate(docs, n),
        "runs_with_violations": viol,
    }


def success_table(outcomes: list[dict], host: str) -> dict:
    table = {}
    for arm in ARM_IDS:
        every = [o for o in outcomes if o["host"] == host and o["arm"] == arm]
        if not every:
            continue
        runs = counted(outcomes, host, arm)
        excluded: dict[str, int] = {}
        for o in every:
            if o["status"] in EXCLUDED:
                excluded[o["status"]] = excluded.get(o["status"], 0) + 1
        row = {"runs": len(every), "excluded": excluded, **_summary(runs)}
        row["in_scope"] = _summary([o for o in runs if o["in_scope"]])
        row["out_of_scope"] = _summary([o for o in runs if not o["in_scope"]])
        row["task_failures"] = sum(1 for o in runs if o["status"] == "task-failure")
        row["budget_exhausted"] = sum(1 for o in runs if o["status"] == "budget-exhausted")
        table[arm] = row
    return table


def per_task(outcomes: list[dict], host: str, arm: str, subset=None) -> dict[str, dict]:
    tasks: dict[str, dict] = {}
    for o in counted(outcomes, host, arm):
        if subset is not None and not subset(o):
            continue
        entry = tasks.setdefault(o["task"], {"family": o["family"], "values": []})
        entry["values"].append(1.0 if o.get("success") else 0.0)
    return tasks


def paired(outcomes: list[dict], host: str, x: str, a: str, *, role: str, seed, replicates: int,
           frozen: dict, subset=None) -> dict:
    tx, ta = per_task(outcomes, host, x, subset), per_task(outcomes, host, a, subset)
    common = sorted(set(tx) & set(ta))
    result = {
        "comparison": f"{x}-{a}",
        "role": role,
        "tasks": len(common),
        "unpaired_tasks": sorted(set(tx) ^ set(ta)),
    }
    if not common:
        result.update(estimate=None, ci95=None, families=0, min_repeats=0, conclusive=False,
                      inconclusive_reasons=["no task has counted runs in both arms"], per_task=[])
        return result
    diffs = {t: statistics.fmean(tx[t]["values"]) - statistics.fmean(ta[t]["values"]) for t in common}
    family_of = {t: tx[t]["family"] for t in common}
    lo, hi = cluster_bootstrap(diffs, family_of, seed=seed, replicates=replicates,
                               confidence=frozen["gates"]["confidence"])
    families = len(set(family_of.values()))
    min_repeats = min(min(len(tx[t]["values"]), len(ta[t]["values"])) for t in common)
    need = frozen["conclusive"]
    reasons = []
    if families < need["min_families"]:
        reasons.append(f"{families} families < {need['min_families']}")
    if len(common) < need["min_tasks"]:
        reasons.append(f"{len(common)} tasks < {need['min_tasks']}")
    if min_repeats < need["min_repeats"]:
        reasons.append(f"{min_repeats} repeats < {need['min_repeats']}")
    result.update(
        estimate=statistics.fmean(diffs.values()),
        ci95=[lo, hi],
        families=families,
        min_repeats=min_repeats,
        conclusive=not reasons,
        inconclusive_reasons=reasons,
        per_task=[
            {"task": t, "family": family_of[t], x: statistics.fmean(tx[t]["values"]),
             a: statistics.fmean(ta[t]["values"]), "difference": diffs[t],
             "repeats": [len(tx[t]["values"]), len(ta[t]["values"])]}
            for t in common
        ],
    )
    return result


# -- blocking ------------------------------------------------------------------------


def block_files(reason: str | None) -> set[str]:
    return {m.group("file") for m in map(FINDING.match, (reason or "").splitlines()) if m}


def boundaries(outcome: dict) -> tuple[list[dict], list[dict]]:
    """Eligible completion boundaries of one run, and blocks outside any of them.

    Eligibility comes from the benchmark's own edit trace — a user turn in which the
    repository changed — never from what the adapter decided. The boundary is that
    turn's first completion check; continuations after a block are repair verification
    and are not boundaries, but a block they issue counts against the same boundary.
    A block is necessary only when a document it names had an unmet obligation at that
    moment; anything else (backlog, a false finding, a stale scan, unavailable
    verification) is unnecessary."""
    eligible, outside = [], []
    hooked = bool(outcome.get("arm_install", {}).get("hooks", {}).get("Stop"))
    for turn in outcome.get("trace", []):
        stops = turn.get("stops", [])
        edits = turn.get("edits") or any(s.get("edits_before") for s in stops[:1])
        judged = []
        for i, stop in enumerate(stops):
            if stop.get("decision") != "block":
                continue
            named = block_files(stop.get("block_reason"))
            unmet = set(stop.get("unmet_files") or [])
            necessary = bool(named & unmet)
            later = stops[i + 1] if i + 1 < len(stops) else None
            unresolved = necessary and (later is None or bool(named & set(later.get("unmet_files") or [])))
            judged.append({"necessary": necessary, "unresolved": unresolved, "files": sorted(named)})
        if not edits:
            outside.extend(judged)
            continue
        first = stops[0] if stops else None
        eligible.append({
            "run_id": outcome["run_id"],
            "task": outcome["task"],
            "family": outcome["family"],
            "turn": turn.get("index"),
            "missed_hook": hooked and (first is None or bool(first.get("hook_failed"))),
            "blocks": judged,
            "unnecessary": sum(1 for b in judged if not b["necessary"]),
        })
    return eligible, outside


def block_stats(outcomes: list[dict], host: str, arm: str, *, seed, replicates: int, frozen: dict) -> dict:
    runs = counted(outcomes, host, arm)
    eligible, outside = [], []
    for o in runs:
        e, x = boundaries(o)
        eligible.extend(e)
        outside.extend(x)
    n = len(eligible)
    x = sum(1 for b in eligible if b["unnecessary"])
    blocks = [b for bd in eligible for b in bd["blocks"]] + outside
    unnecessary_events = sum(1 for b in blocks if not b["necessary"])
    confidence = frozen["gates"]["confidence"]
    cp = clopper_pearson_upper(x, n, confidence)
    boot = None
    if n:
        by_task: dict[str, list[int]] = {}
        family_of: dict[str, str] = {}
        for b in eligible:
            by_task.setdefault(b["task"], [0, 0])
            by_task[b["task"]][0] += 1 if b["unnecessary"] else 0
            by_task[b["task"]][1] += 1
            family_of[b["task"]] = b["family"]
        families: dict[str, list[str]] = {}
        for t in by_task:
            families.setdefault(family_of[t], []).append(t)
        rng = random.Random(seed)
        rates = []
        for _ in range(replicates):
            picked = resample(rng, families)
            num = sum(by_task[t][0] for t in picked)
            den = sum(by_task[t][1] for t in picked)
            rates.append(num / den if den else 0.0)
        boot = percentile(rates, 100 - (1 - confidence) / 2 * 100)
    return {
        "eligible_boundaries": n,
        "boundaries_with_unnecessary_block": x,
        "unnecessary_block_rate": rate(x, n),
        # The larger of an exact binomial bound, which ignores clustering, and a
        # family-clustered bootstrap bound, which is degenerate with zero events.
        "upper_bound": max(v for v in (cp, boot) if v is not None) if n else None,
        "upper_bound_exact": cp,
        "upper_bound_bootstrap": boot,
        "block_events": len(blocks),
        "unnecessary_block_events": unnecessary_events,
        "necessary_block_events": len(blocks) - unnecessary_events,
        "fraction_of_blocks_unnecessary": rate(unnecessary_events, len(blocks)),
        "unresolved_valid_blocks": sum(1 for b in blocks if b["necessary"] and b["unresolved"]),
        "blocks_outside_eligible_boundaries": len(outside),
        "missed_hook_invocations": sum(1 for b in eligible if b["missed_hook"]),
    }


# -- latency, cost, overhead ---------------------------------------------------------


def _tokens(o: dict) -> float | None:
    t = o.get("tokens") or {}
    parts = [t.get("input"), t.get("output")]
    return None if any(p is None for p in parts) else float(sum(parts))


def latency_table(outcomes: list[dict], host: str) -> dict:
    table = {}
    for arm in ARM_IDS:
        runs = counted(outcomes, host, arm)
        if not runs:
            continue
        durations = [o["durations"]["total_seconds"] for o in runs]
        tokens = [_tokens(o) for o in runs]
        costs = [o.get("cost_usd") for o in runs]
        table[arm] = {
            "runs": len(runs),
            "duration_mean": statistics.fmean(durations),
            "duration_p50": nearest_rank(durations, 50),
            "duration_p95": nearest_rank(durations, 95),
            "checker_seconds_mean": statistics.fmean(o.get("checker_seconds") or 0.0 for o in runs),
            "hook_invocations_mean": statistics.fmean(o.get("hook_invocations", 0) for o in runs),
            "continuations": sum(o.get("continuations_requested", 0) for o in runs),
            "tokens_mean": None if any(t is None for t in tokens) else statistics.fmean(tokens),
            "cost_usd_total": None if any(c is None for c in costs) else sum(costs),
        }
    return table


def clean_overhead(outcomes: list[dict], host: str, frozen: dict) -> dict:
    limit = frozen["gates"]["max_clean_control_overhead"]
    arms = {arm: [o for o in counted(outcomes, host, arm) if o["category"] == "clean-control"] for arm in ("A", "D")}
    if not arms["A"] or not arms["D"]:
        return {"result": "unavailable", "detail": "no clean-control runs in both A and D"}
    fewest = min(len(arms["A"]), len(arms["D"]))
    if fewest < frozen["conclusive"]["min_repeats"]:
        return {"result": "inconclusive",
                "detail": f"{fewest} clean-control runs < {frozen['conclusive']['min_repeats']} in an arm"}
    pa = nearest_rank([o["durations"]["total_seconds"] for o in arms["A"]], 95)
    pd = nearest_rank([o["durations"]["total_seconds"] for o in arms["D"]], 95)
    duration_rise = (pd - pa) / pa if pa else None
    ta = [_tokens(o) for o in arms["A"]]
    td = [_tokens(o) for o in arms["D"]]
    if any(t is None for t in ta + td):
        token_rise, token_result = None, "unavailable, not passed"
    else:
        ma, md = statistics.fmean(ta), statistics.fmean(td)
        token_rise = (md - ma) / ma if ma else None
        token_result = "pass" if token_rise is not None and token_rise <= limit else "fail"
    duration_result = "pass" if duration_rise is not None and duration_rise <= limit else "fail"
    if "fail" in (duration_result, token_result):
        result = "fail"
    elif token_result != "pass":
        result = "unavailable"
    else:
        result = "pass"
    return {
        "result": result,
        "p95_duration_A": pa,
        "p95_duration_D": pd,
        "p95_duration_rise": duration_rise,
        "duration": duration_result,
        "mean_token_rise": token_rise,
        "tokens": token_result,
        "limit": limit,
    }


def gates(outcomes: list[dict], host: str, primary: dict | None, blocks: dict | None, frozen: dict) -> dict:
    """The promotion gates for bounded blocking on one host."""
    g = frozen["gates"]
    arms = {o["arm"] for o in outcomes if o["host"] == host}
    if not {"A", "D"} <= arms:
        return {"overall": "not evaluated", "detail": "arms A and D were not both run", "gates": {}}
    out = {}
    regression = [o for o in counted(outcomes, host, "D") if o["category"] in FALSE_BLOCK_REGRESSION]
    blocked = sorted({o["task"] for o in regression if o.get("continuations_requested")})
    if not regression:
        out["false_block_regressions"] = {"result": "unavailable", "detail": "no regression-case runs in D"}
    else:
        out["false_block_regressions"] = {
            "result": "fail" if blocked else "pass",
            "detail": f"{len(regression)} D runs of {len({o['task'] for o in regression})} regression cases; "
                      f"blocked in: {', '.join(blocked) or 'none'}",
        }
    if primary is None or primary["estimate"] is None:
        out["primary_improvement"] = {"result": "unavailable", "detail": "no paired D-A tasks"}
    elif not primary["conclusive"]:
        out["primary_improvement"] = {
            "result": "inconclusive",
            "detail": "below the frozen thresholds: " + "; ".join(primary["inconclusive_reasons"]),
        }
    else:
        ok = primary["estimate"] >= g["min_improvement"] and primary["ci95"][0] > 0
        out["primary_improvement"] = {
            "result": "pass" if ok else "fail",
            "detail": f"estimate {primary['estimate']:+.3f} (needs >= {g['min_improvement']:+.2f}), "
                      f"95% CI lower {primary['ci95'][0]:+.3f} (needs > 0)",
        }
    if not blocks or not blocks["eligible_boundaries"]:
        out["unnecessary_blocks"] = {"result": "unavailable", "detail": "no eligible boundaries in D"}
    else:
        limit = g["max_unnecessary_block_upper"]
        # The observed rate at or over the limit fails outright; an observed rate under
        # it whose bound is not is a sample too small to say, not a failure.
        if blocks["upper_bound"] < limit:
            result = "pass"
        elif blocks["unnecessary_block_rate"] >= limit:
            result = "fail"
        else:
            result = "inconclusive"
        out["unnecessary_blocks"] = {
            "result": result,
            "detail": f"observed {blocks['unnecessary_block_rate']:.3f}, upper 95% bound "
                      f"{blocks['upper_bound']:.3f} over {blocks['eligible_boundaries']} eligible "
                      f"boundaries (bound needs < {limit})",
        }
    overhead = clean_overhead(outcomes, host, frozen)
    out["clean_control_overhead"] = {"result": overhead["result"], "detail": overhead}
    results = [v["result"] for v in out.values()]
    if "fail" in results:
        overall = "fail"
    elif all(r == "pass" for r in results):
        overall = "pass"
    else:
        overall = "inconclusive"
    return {"overall": overall, "gates": out}


# -- the report ----------------------------------------------------------------------


def per_case(outcomes: list[dict], host: str) -> list[dict]:
    rows: dict[str, dict] = {}
    for o in outcomes:
        if o["host"] != host:
            continue
        row = rows.setdefault(o["task"], {"task": o["task"], "family": o["family"], "category": o["category"],
                                          "in_scope": o["in_scope"], "arms": {}})
        cell = row["arms"].setdefault(o["arm"], {"successes": 0, "denominator": 0, "excluded": 0})
        if o["status"] in COUNTED:
            cell["denominator"] += 1
            cell["successes"] += 1 if o.get("success") else 0
        else:
            cell["excluded"] += 1
    return [rows[k] for k in sorted(rows)]


def exclusions(outcomes: list[dict]) -> list[dict]:
    reruns = {o["rerun_of"]: o["run_id"] for o in outcomes if o.get("rerun_of")}
    return [
        {"run_id": o["run_id"], "host": o["host"], "task": o["task"], "arm": o["arm"],
         "status": o["status"], "error": o.get("error"), "rerun_of": o.get("rerun_of"),
         "rerun_by": reruns.get(o["run_id"])}
        for o in outcomes if o["status"] in EXCLUDED
    ]


def analyze(out_dir: Path, *, seed=None, replicates: int = 2000) -> dict:
    manifest, outcomes = load(out_dir)
    frozen = {**DEFAULT_FROZEN, **(manifest.get("frozen") or {})}
    seed = manifest.get("seed", 0) if seed is None else seed
    report = {
        "schema_version": 1,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "run_set_id": manifest.get("run_set_id"),
        "agent": manifest.get("agent"),
        "model": manifest.get("model"),
        "split": manifest.get("split"),
        "holdout_use": manifest.get("holdout_use"),
        "evidence": (
            "Scripted agent: these results validate the harness end to end. They are not "
            "evidence about any real agent, and no effectiveness claim follows from them."
            if manifest.get("agent") == "scripted" else
            "Live agent run. Read the thresholds, denominators and exclusions before any result."
        ),
        "bootstrap": {"replicates": replicates, "seed": seed, "unit": "families, then tasks; repeats stay with their task"},
        "thresholds": frozen,
        "pooled": None,
        "pooled_note": "No pooled cross-host result is computed; each host is reported and gated separately.",
        "hosts": {},
        "exclusions": exclusions(outcomes),
        "outcome_counts": {s: sum(1 for o in outcomes if o["status"] == s) for s in COUNTED + EXCLUDED},
    }
    for host in sorted({o["host"] for o in outcomes}):
        arms = {o["arm"] for o in outcomes if o["host"] == host}
        comparisons = []
        for x, a, role in PAIRS:
            if {x, a} <= arms:
                entry = paired(outcomes, host, x, a, role=role, seed=seed, replicates=replicates, frozen=frozen)
                entry["in_scope_only"] = paired(outcomes, host, x, a, role="exploratory", seed=seed,
                                                replicates=replicates, frozen=frozen,
                                                subset=lambda o: o["in_scope"])
                entry["in_scope_only"].pop("per_task", None)
                comparisons.append(entry)
        blocking = {
            arm: block_stats(outcomes, host, arm, seed=seed, replicates=replicates, frozen=frozen)
            for arm in sorted(arms) if arm in ("C", "D")
        }
        primary = next((c for c in comparisons if c["comparison"] == "D-A"), None)
        report["hosts"][host] = {
            "success": success_table(outcomes, host),
            "paired": comparisons,
            "null_or_negative": [
                c["comparison"] for c in comparisons
                if c["estimate"] is not None and (c["estimate"] <= 0 or (c["ci95"][0] is not None and c["ci95"][0] <= 0))
            ],
            "blocking": blocking,
            "latency": latency_table(outcomes, host),
            "gates": gates(outcomes, host, primary, blocking.get("D"), frozen),
            "per_case": per_case(outcomes, host),
        }
    return report


def _pct(v) -> str:
    return "n/a" if v is None else f"{100 * v:.1f}%"


def _num(v, fmt="{:.3f}") -> str:
    return "n/a" if v is None else fmt.format(v)


def _ci(comparison: dict) -> str:
    ci = comparison.get("ci95")
    return "n/a" if not ci or ci[0] is None else f"[{ci[0]:+.3f}, {ci[1]:+.3f}]"


def render_markdown(report: dict) -> str:
    t = report["thresholds"]
    lines = [
        "# Agent-task benchmark summary",
        "",
        f"Run set `{report['run_set_id']}` · agent `{report['agent']}` · model `{report['model']}` · "
        f"split `{report['split']}`" + (f" (held-out use: {report['holdout_use']})" if report.get("holdout_use") else ""),
        "",
        f"> {report['evidence']}",
        f"> {report['pooled_note']}",
        "",
        f"Conclusive only with at least {t['conclusive']['min_families']} repository families, "
        f"{t['conclusive']['min_tasks']} paired tasks and {t['conclusive']['min_repeats']} repeats per task per arm "
        "(bench/frozen.toml); below that every comparison is reported as inconclusive. Intervals are "
        f"{int(100 * t['gates']['confidence'])}% percentile intervals from a bootstrap that resamples families, "
        f"then tasks, keeping each task's repeats together ({report['bootstrap']['replicates']} replicates, "
        f"seed {report['bootstrap']['seed']}).",
        "",
        "Outcomes: " + ", ".join(f"{k} {v}" for k, v in report["outcome_counts"].items()),
    ]
    for host, h in report["hosts"].items():
        lines += ["", f"## Host: {host}", "", "### Primary outcome by arm", "",
                  "| Arm | Runs | Excluded | Denominator | Success | Code success | Docs correct | Runs with violations |",
                  "|---|---|---|---|---|---|---|---|"]
        for arm, r in h["success"].items():
            excluded = ", ".join(f"{k} {v}" for k, v in r["excluded"].items()) or "0"
            lines.append(
                f"| {arm} | {r['runs']} | {excluded} | {r['denominator']} | {r['successes']}/{r['denominator']} "
                f"({_pct(r['success_rate'])}) | {r['code_successes']}/{r['denominator']} | "
                f"{r['docs_correct']}/{r['denominator']} | {r['runs_with_violations']} |"
            )
        lines += ["", "In scope versus out of scope (cases the checker cannot detect, reported separately):", "",
                  "| Arm | In-scope success | Out-of-scope success |", "|---|---|---|"]
        for arm, r in h["success"].items():
            i, o = r["in_scope"], r["out_of_scope"]
            lines.append(f"| {arm} | {i['successes']}/{i['denominator']} ({_pct(i['success_rate'])}) | "
                         f"{o['successes']}/{o['denominator']} ({_pct(o['success_rate'])}) |")
        lines += ["", "### Paired differences in success", "",
                  "| Comparison | Role | Estimate | 95% CI | Tasks | Families | Min repeats | Status |",
                  "|---|---|---|---|---|---|---|---|"]
        for c in h["paired"]:
            status = "conclusive" if c["conclusive"] else "inconclusive: " + "; ".join(c["inconclusive_reasons"])
            lines.append(f"| {c['comparison']} | {c['role']} | {_num(c['estimate'], '{:+.3f}')} | {_ci(c)} | "
                         f"{c['tasks']} | {c['families']} | {c['min_repeats']} | {status} |")
            s = c["in_scope_only"]
            lines.append(f"| {c['comparison']} (in-scope only) | exploratory | {_num(s['estimate'], '{:+.3f}')} | "
                         f"{_ci(s)} | {s['tasks']} | {s['families']} | {s['min_repeats']} | exploratory |")
        if not h["paired"]:
            lines.append("| (none: the arms run do not form a pair with A) | | | | | | | |")
        lines += ["", "Null or negative comparisons (estimate at or below zero, or an interval reaching zero): "
                  + (", ".join(h["null_or_negative"]) or "none") + "."]
        lines += ["", "### Blocking", "",
                  "| Arm | Eligible boundaries | With unnecessary block | Rate | Upper 95% bound | Block events | "
                  "Unnecessary events | Fraction unnecessary | Unresolved valid | Missed hook invocations |",
                  "|---|---|---|---|---|---|---|---|---|---|"]
        for arm, b in h["blocking"].items():
            lines.append(
                f"| {arm} | {b['eligible_boundaries']} | {b['boundaries_with_unnecessary_block']} | "
                f"{_pct(b['unnecessary_block_rate'])} | {_pct(b['upper_bound'])} | {b['block_events']} | "
                f"{b['unnecessary_block_events']} | {_pct(b['fraction_of_blocks_unnecessary'])} | "
                f"{b['unresolved_valid_blocks']} | {b['missed_hook_invocations']} |"
            )
        if not h["blocking"]:
            lines.append("| (no hook arm was run) | | | | | | | | | |")
        lines += ["", "### Latency and cost", "",
                  "| Arm | Runs | Mean s | p50 s | p95 s | Checker s (mean) | Hook calls (mean) | Continuations | Tokens (mean) | Cost (USD) |",
                  "|---|---|---|---|---|---|---|---|---|---|"]
        for arm, r in h["latency"].items():
            lines.append(
                f"| {arm} | {r['runs']} | {r['duration_mean']:.2f} | {r['duration_p50']:.2f} | {r['duration_p95']:.2f} | "
                f"{r['checker_seconds_mean']:.2f} | {r['hook_invocations_mean']:.1f} | {r['continuations']} | "
                f"{_num(r['tokens_mean'], '{:.0f}') if r['tokens_mean'] is not None else 'unavailable'} | "
                f"{_num(r['cost_usd_total'], '{:.4f}') if r['cost_usd_total'] is not None else 'unavailable'} |"
            )
        g = h["gates"]
        lines += ["", f"### Promotion gates for bounded blocking: **{g['overall']}**", ""]
        if g.get("detail"):
            lines.append(g["detail"] + ".")
        else:
            lines += ["| Gate | Result | Detail |", "|---|---|---|"]
            for name, v in g["gates"].items():
                detail = v["detail"]
                if isinstance(detail, dict):
                    detail = (f"p95 duration rise {_pct(detail.get('p95_duration_rise'))} ({detail.get('duration')}); "
                              f"mean token rise {_pct(detail.get('mean_token_rise'))} ({detail.get('tokens')})"
                              if "duration" in detail else detail.get("detail", ""))
                lines.append(f"| {name} | {v['result']} | {detail} |")
        lines += ["", "### Per-case counts (successes/denominator)", "",
                  "| Task | Family | Category | In scope | " + " | ".join(ARM_IDS) + " |",
                  "|---|---|---|---|" + "---|" * len(ARM_IDS)]
        for row in h["per_case"]:
            cells = []
            for arm in ARM_IDS:
                c = row["arms"].get(arm)
                cells.append("—" if c is None else f"{c['successes']}/{c['denominator']}"
                             + (f" (+{c['excluded']} excl.)" if c["excluded"] else ""))
            lines.append(f"| {row['task']} | {row['family']} | {row['category']} | "
                         f"{'yes' if row['in_scope'] else 'no'} | " + " | ".join(cells) + " |")
    lines += ["", "## Exclusions", ""]
    if report["exclusions"]:
        lines += ["| Run | Host | Task | Arm | Status | Error | Rerun |", "|---|---|---|---|---|---|---|"]
        for e in report["exclusions"]:
            lines.append(f"| {e['run_id']} | {e['host']} | {e['task']} | {e['arm']} | {e['status']} | "
                         f"{(e['error'] or '').replace('|', '/')} | {e['rerun_by'] or 'none'} |")
    else:
        lines.append("No run was excluded.")
    return "\n".join(lines) + "\n"


def write_reports(out_dir: Path, **kwargs) -> dict:
    report = analyze(out_dir, **kwargs)
    (out_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True, default=str) + "\n",
                                         encoding="utf-8")
    (out_dir / "summary.md").write_text(render_markdown(report), encoding="utf-8")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Analyse one benchmark run set.")
    parser.add_argument("out", type=Path)
    parser.add_argument("--replicates", type=int, default=2000)
    parser.add_argument("--seed", type=int)
    args = parser.parse_args(argv)
    write_reports(args.out, seed=args.seed, replicates=args.replicates)
    print(args.out / "summary.md")
    return 0


if __name__ == "__main__":
    sys.exit(main())
