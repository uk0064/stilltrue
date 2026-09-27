"""Live host sessions — real agents, real model spend. Never run by the tests.

Nothing here runs unless `run.py` was given both `--authorized` and a positive
`--spending-limit-usd` (see `authorize`). It has not been executed against a live host:
treat it as a reviewed plan, not a validated harness, until a first authorized pilot
has been run and its outcomes inspected.

Claude Code: each user turn is one `claude -p` invocation with `--output-format json`,
the second resuming the first's session. The spending limit is divided evenly across
every planned invocation and passed as `--max-budget-usd`; the runner also stops
scheduling once reported spend reaches the limit.

Codex: refused. `codex exec` reports no billed cost, so a spending limit could not be
enforced, and an unenforceable limit is not a limit.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from pathlib import Path

import grade
import shim

# Credentials the live host needs, passed through into the otherwise isolated
# environment. Nothing else from the operator's environment reaches the session.
AUTH_VARS = ("ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN")


class Refused(Exception):
    """A live run may not start."""


def authorize(agent: str, host: str, authorized: bool, limit: float | None) -> None:
    """Raise Refused unless a live run is explicitly authorized and capped."""
    if agent == "scripted":
        return
    problems = []
    if not authorized:
        problems.append("--authorized was not given")
    if limit is None:
        problems.append("--spending-limit-usd was not given")
    elif not (limit > 0):
        problems.append(f"--spending-limit-usd must be greater than 0, not {limit}")
    if problems:
        raise Refused(
            f"refusing to start a live {agent} run: {'; '.join(problems)}. Live runs spend "
            "money on the operator's model access; pass --authorized and "
            "--spending-limit-usd N (N > 0) to start one."
        )
    if agent != host:
        raise Refused(f"--agent {agent} runs on its own host; pass --host {agent}")
    if agent == "codex":
        raise Refused(
            "refusing to start a live codex run: `codex exec` reports no billed cost, so "
            "the spending limit cannot be enforced. Live codex runs are not implemented."
        )


def host_version(agent: str) -> str | None:
    exe = shutil.which(agent)
    if exe is None:
        return None
    proc = subprocess.run([exe, "--version"], capture_output=True, text=True, timeout=30)
    return proc.stdout.strip().splitlines()[0] if proc.stdout.strip() else None


def _usage(result: dict) -> dict:
    usage = result.get("usage") or {}
    return {
        "input": usage.get("input_tokens"),
        "output": usage.get("output_tokens"),
        "cached": usage.get("cache_read_input_tokens"),
    }


def _sum(values):
    return None if any(v is None for v in values) or not values else sum(values)


def run_claude_code(
    *,
    task,
    install,
    worktree: Path,
    agent_dir: Path,
    env: dict,
    turns: list[tuple[str, str]],
    deadline: float,
    budget_per_turn: float,
    model: str | None,
    shim_log: Path,
) -> dict:
    """One live Claude Code session. Returns the same shape as a scripted session."""
    exe = shutil.which("claude", path=env.get("PATH"))
    if exe is None:
        raise FileNotFoundError("claude is not on PATH")
    for name in AUTH_VARS:
        if name in os.environ:
            env[name] = os.environ[name]
    session_id = None
    trace, costs, usages = [], [], []
    for index, (kind, prompt) in enumerate(turns, start=1):
        argv = [exe, "-p", prompt, "--output-format", "json",
                "--max-budget-usd", f"{budget_per_turn:.4f}", "--permission-mode", "acceptEdits"]
        if model:
            argv += ["--model", model]
        if session_id:
            argv += ["--resume", session_id]
        argv += install.argv
        turn = {"index": index, "kind": kind, "start_fingerprint": grade.fingerprint(worktree),
                "started_at": time.time(), "stops": [], "continuations": 0,
                "continuation_limit_reached": False}
        trace.append(turn)
        proc = subprocess.run(argv, cwd=worktree, env=env, capture_output=True, text=True,
                              timeout=max(1.0, deadline - time.monotonic()))
        (agent_dir / f"turn-{index}.json").write_text(proc.stdout, encoding="utf-8")
        try:
            result = json.loads(proc.stdout)
        except ValueError:
            result = {"is_error": True, "subtype": "unreadable-output"}
        session_id = result.get("session_id", session_id)
        costs.append(result.get("total_cost_usd"))
        usages.append(_usage(result))
        turn["ended_at"] = time.time()
        turn["end_fingerprint"] = grade.fingerprint(worktree)
        turn["edits"] = turn["end_fingerprint"] != turn["start_fingerprint"]
        turn["host_result"] = {k: result.get(k) for k in ("subtype", "is_error", "num_turns")}
        if result.get("is_error"):
            break
    # Completion checks come from the shim, which saw every Stop the host sent.
    stops = [r for r in shim.read_log(shim_log) if r.get("event") == "Stop"]
    for turn in trace:
        for record in stops:
            if turn["started_at"] <= record["started_at"] <= turn.get("ended_at", float("inf")):
                answer = {}
                try:
                    answer = json.loads(record.get("stdout") or "{}")
                except ValueError:
                    pass
                decision = "block" if answer.get("decision") == "block" else (
                    "advise" if answer.get("systemMessage") else "nothing")
                turn["stops"].append({
                    "stop_hook_active": bool(record.get("stop_hook_active")),
                    "fingerprint": record.get("fingerprint"),
                    "unmet_files": record.get("unmet_files", []),
                    "edits_before": record.get("fingerprint") != turn["start_fingerprint"],
                    "hooked": True,
                    "hook_failed": record.get("exit") != 0,
                    "decision": decision,
                    "block_reason": answer.get("reason") if decision == "block" else None,
                })
        turn["continuations"] = sum(1 for s in turn["stops"] if s["decision"] == "block")
    last = trace[-1]["host_result"] if trace else {}
    return {
        "trace": trace,
        "session_id": session_id,
        "cost_usd": _sum(costs),
        "tokens": {key: _sum([u[key] for u in usages]) for key in ("input", "output", "cached")},
        "budget_exhausted": "budget" in str(last.get("subtype") or ""),
        "error": last.get("subtype") if last.get("is_error") else None,
    }
