"""Run the agent-task benchmark.

For every task × arm × repeat: build a fresh repository from the task's build.sh in its
own directory, give the run its own HOME, cache and hook-state directories, install the
arm, play the session, grade the final worktree, and append one outcome line. Arm order
is shuffled within each task and repeat from the seed, and recorded.

    python3 bench/run.py --agent scripted --host claude-code --arms A,C,D \
        --repeats 1 --split development --seed 7 --out /tmp/bench-out

The scripted agent needs no model and costs nothing; it validates the harness. A live
agent (`--agent claude-code`) spends money and refuses to start without `--authorized`
and `--spending-limit-usd N`.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import random
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

BENCH = Path(__file__).resolve().parent
ROOT = BENCH.parent
sys.path.insert(0, str(BENCH))

import arms as arms_mod  # noqa: E402
import grade  # noqa: E402
import hostsim  # noqa: E402
import live  # noqa: E402
import manifest  # noqa: E402
import shim  # noqa: E402

SCHEMA_VERSION = 1
STATUSES = ("completed", "task-failure", "budget-exhausted", "infrastructure-failure", "interrupted")
# Every session is the task prompt, then this fixed follow-up, identical in every arm.
# Claude Code's adapter delivers completion advice as context at the *next* prompt, so
# a single-turn session would structurally deny arm C any chance to act on it.
REVIEW_PROMPT = (
    "Before we finish: if anything is left to do for this task, do it now. "
    "Otherwise reply that you are done."
)
AGENTS = ("scripted", "claude-code", "codex")
BUILD_TIMEOUT = 120


def now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")


def arm_order(seed: int, task_id: str, repeat: int, arm_ids: list[str]) -> list[str]:
    """The arms of one task and repeat, in the order they run. A string seed makes the
    shuffle independent of every other task and of Python's hash randomisation."""
    order = list(arm_ids)
    random.Random(f"{seed}:{task_id}:{repeat}").shuffle(order)
    return order


def schedule(tasks, arm_ids: list[str], repeats: int, seed: int):
    runs, orders = [], {}
    for task in tasks:
        orders[task.id] = []
        for repeat in range(repeats):
            order = arm_order(seed, task.id, repeat, arm_ids)
            orders[task.id].append(order)
            runs.extend((task, repeat, arm) for arm in order)
    return runs, orders


def locate_binaries(explicit: Path | None, root: Path = ROOT) -> Path | None:
    candidates = [explicit] if explicit else [root / "target" / "release", root / "target" / "debug"]
    for directory in candidates:
        if directory and all(os.access(directory / n, os.X_OK) for n in ("stilltrue", "stilltrue-hook")):
            return directory.resolve()
    return None


def system_path(exclude: list[Path] = ()) -> str:
    """The operator's PATH minus anywhere a stilltrue binary lives: arm A must not find
    one, and the others must find only the one under test."""
    keep = []
    for entry in os.environ.get("PATH", "/usr/bin:/bin").split(os.pathsep):
        p = Path(entry)
        if not entry or any(p == e for e in exclude):
            continue
        if (p / "stilltrue").exists() or (p / "stilltrue-hook").exists():
            continue
        keep.append(entry)
    return os.pathsep.join(keep)


def sha256_file(path: Path) -> str | None:
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def revision(root: Path) -> dict:
    def git(*args):
        proc = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)
        return proc.stdout.strip() if proc.returncode == 0 else None

    return {"commit": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain"))}


def tool_version(argv: list[str]) -> str | None:
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return proc.stdout.strip() or None


def write_json(path: Path, value) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, sort_keys=True, default=str) + "\n", encoding="utf-8")
    tmp.replace(path)


class Runner:
    def __init__(self, args, tasks, bin_dir: Path | None, work_root: Path):
        self.args, self.tasks, self.bin_dir, self.work_root = args, tasks, bin_dir, work_root
        self.out = args.out
        self.outcomes = self.out / "outcomes.jsonl"
        self.spent = 0.0
        self.planned_turns = 1

    # -- one run ----------------------------------------------------------------------

    def run_one(self, task, arm_id: str, repeat: int, rerun_of: str | None) -> dict:
        args = self.args
        arm = arms_mod.ARMS[arm_id]
        run_id = f"{task.id}.{arm_id}.r{repeat}.{uuid.uuid4().hex[:8]}"
        base = self.work_root / run_id
        dirs = {name: base / name for name in ("worktree", "home", "cache", "state", "agent", "shim", "tmp")}
        for name, path in dirs.items():
            if name != "worktree":
                path.mkdir(parents=True)
        started, t0 = now(), time.monotonic()
        outcome = {
            "schema_version": SCHEMA_VERSION,
            "run_id": run_id,
            "rerun_of": rerun_of,
            **task.summary(),
            "arm": arm_id,
            "repeat": repeat,
            "host": args.host,
            "agent": args.agent,
            "model": "scripted" if args.agent == "scripted" else args.model,
            "status": None,
            "error": None,
            "success": None,
            "started_at": started,
            "dirs": {k: str(v) for k, v in dirs.items()},
            "tokens": {"input": None, "output": None, "cached": None},
            "cost_usd": None,
            "pricing_date": None,
            "grading": None,
            "trace": [],
        }
        durations = {"build_seconds": None, "agent_seconds": None, "grading_seconds": None}
        path = system_path([dirs["shim"]] + ([self.bin_dir] if self.bin_dir else []))
        env = {
            "PATH": path,
            "HOME": str(dirs["home"]),
            "XDG_CACHE_HOME": str(dirs["cache"]),
            # Named outright, so no arm can ever read another's warmed history cache,
            # whatever the environment the harness itself was started in.
            "STILLTRUE_CACHE_DIR": str(dirs["cache"] / "stilltrue"),
            "XDG_CONFIG_HOME": str(dirs["home"] / ".config"),
            "TMPDIR": str(dirs["tmp"]),
            "LANG": "C.UTF-8",
            "GIT_CONFIG_NOSYSTEM": "1",
            "PYTHONDONTWRITEBYTECODE": "1",
        }
        install = None
        session = None
        try:
            # Build the starting repository.
            b0 = time.monotonic()
            proc = subprocess.run(
                ["bash", str(task.build_script), str(dirs["worktree"])],
                env=env, capture_output=True, text=True, timeout=BUILD_TIMEOUT,
            )
            durations["build_seconds"] = round(time.monotonic() - b0, 3)
            if proc.returncode != 0:
                raise InfrastructureFailure(f"build.sh failed: {proc.stderr.strip()[-500:]}")
            outcome["base_commit"] = subprocess.run(
                ["git", "-C", str(dirs["worktree"]), "rev-parse", "HEAD"],
                env=env, capture_output=True, text=True,
            ).stdout.strip()

            install = arms_mod.install(
                args.host, arm_id, run_dir=base, worktree=dirs["worktree"], home=dirs["home"]
            )
            outcome["arm_install"] = install.describe()
            if arm.checker_on_path or install.hooks:
                if self.bin_dir is None:
                    raise InfrastructureFailure("no stilltrue binaries (pass --stilltrue-bin-dir)")
                shim.install(dirs["shim"])
                env["PATH"] = os.pathsep.join([str(dirs["shim"]), path])
                env.update(
                    BENCH_SHIM_LOG=str(dirs["agent"] / "shim.jsonl"),
                    BENCH_REAL_BIN_DIR=str(self.bin_dir),
                    BENCH_TASK_DIR=str(task.dir),
                )
            env.update(install.env)
            agent_env = dict(
                env,
                BENCH_CONTEXT_FILE=str(dirs["agent"] / "context.txt"),
                BENCH_AGENT_LOG=str(dirs["agent"] / "actions.jsonl"),
            )
            hook_env = dict(env, STILLTRUE_HOOK_STATE=str(dirs["state"]))
            turns = [("task", task.prompt), ("review", REVIEW_PROMPT)]
            deadline = time.monotonic() + args.timeout

            if args.agent == "scripted":
                session = hostsim.ScriptedSession(
                    task=task, arm=arm_id, host=args.host, hooks=install.hooks,
                    worktree=dirs["worktree"], agent_dir=dirs["agent"],
                    agent_env=agent_env, hook_env=hook_env, preamble=install.preamble,
                    turns=turns, session_id=run_id, deadline=deadline,
                )
                try:
                    session.run()
                    outcome["status"] = "completed"
                except hostsim.BudgetExhausted:
                    outcome["status"] = "budget-exhausted"
                    outcome["error"] = f"the run's {args.timeout}s budget ran out"
                except hostsim.AgentFailed as error:
                    outcome["status"] = "task-failure"
                    outcome["error"] = f"agent failed: {error}"
                outcome["trace"] = getattr(session, "trace", [])
                durations["agent_seconds"] = round(session.agent_seconds, 3)
            else:
                budget = self.args.spending_limit_usd / max(1, self.planned_turns)
                result = live.run_claude_code(
                    task=task, install=install, worktree=dirs["worktree"],
                    agent_dir=dirs["agent"], env=dict(agent_env, **{
                        "STILLTRUE_HOOK_STATE": str(dirs["state"])}),
                    turns=turns, deadline=deadline, budget_per_turn=budget,
                    model=args.model, shim_log=dirs["agent"] / "shim.jsonl",
                )
                outcome["trace"] = result["trace"]
                outcome["tokens"] = result["tokens"]
                outcome["cost_usd"] = result["cost_usd"]
                outcome["pricing_date"] = dt.date.today().isoformat() if result["cost_usd"] is not None else None
                if result["budget_exhausted"]:
                    outcome["status"] = "budget-exhausted"
                elif result["error"]:
                    outcome["status"] = "task-failure"
                    outcome["error"] = result["error"]
                else:
                    outcome["status"] = "completed"
                durations["agent_seconds"] = round(time.monotonic() - t0, 3)
                if result["cost_usd"] is None:
                    self.spent = float("inf")  # spend unknown: stop scheduling
                else:
                    self.spent += result["cost_usd"]
        except KeyboardInterrupt:
            outcome["status"] = "interrupted"
            outcome["error"] = "interrupted by the operator"
        except subprocess.TimeoutExpired:
            outcome["status"] = "budget-exhausted"
            outcome["error"] = f"the run's {args.timeout}s budget ran out"
        except InfrastructureFailure as error:
            outcome["status"] = "infrastructure-failure"
            outcome["error"] = str(error)
        except Exception as error:  # a harness fault is recorded, never dropped
            outcome["status"] = "infrastructure-failure"
            outcome["error"] = f"{type(error).__name__}: {error}"

        if outcome["status"] in ("completed", "task-failure", "budget-exhausted") and dirs["worktree"].is_dir():
            g0 = time.monotonic()
            grade_home = base / "grade-home"
            grade_home.mkdir(exist_ok=True)
            outcome["grading"] = grade.grade(
                task, dirs["worktree"], base_commit=outcome.get("base_commit", "HEAD"),
                env=grade.check_env(task, grade_home, system_path([dirs["shim"]] + ([self.bin_dir] if self.bin_dir else []))),
                protected=install.protected if install else None, home=dirs["home"],
            )
            durations["grading_seconds"] = round(time.monotonic() - g0, 3)
            # Budget exhaustion and a failed agent are task failures, whatever remains.
            outcome["success"] = outcome["status"] == "completed" and outcome["grading"]["success"]

        records = shim.read_log(dirs["agent"] / "shim.jsonl")
        hooks = [r for r in records if r["program"] == "stilltrue-hook"]
        checks = [r for r in records if r["program"] == "stilltrue"]
        events: dict[str, int] = {}
        for r in hooks:
            events[r.get("event") or "unknown"] = events.get(r.get("event") or "unknown", 0) + 1
        outcome["hook_invocations"] = len(hooks)
        outcome["hook_events"] = events
        outcome["hook_failures"] = sum(1 for r in hooks if r.get("exit") != 0)
        outcome["checker_invocations"] = {"hook": len(hooks), "agent": len(checks)}
        outcome["checker_seconds"] = round(sum(r.get("seconds", 0) for r in hooks + checks), 4)
        outcome["continuations_requested"] = sum(t.get("continuations", 0) for t in outcome["trace"])
        outcome["finished_at"] = now()
        durations["total_seconds"] = round(time.monotonic() - t0, 3)
        outcome["durations"] = durations
        transcript = dirs["agent"] / "transcript.jsonl"
        outcome["transcript_sha256"] = sha256_file(transcript)

        # Keep the evidence; drop the bulky, disposable parts.
        evidence = self.out / "runs" / run_id
        shutil.copytree(dirs["agent"], evidence / "agent")
        if any(dirs["state"].iterdir()):
            shutil.copytree(dirs["state"], evidence / "hook-state")
        if not args.keep_worktrees:
            shutil.rmtree(base, ignore_errors=True)
        return outcome

    # -- the whole set ----------------------------------------------------------------

    def run(self, plan) -> tuple[list[dict], str]:
        results = []
        self.planned_turns = 2 * len(plan)
        status = "complete"
        for task, repeat, arm_id in plan:
            if self.args.agent != "scripted" and self.spent >= self.args.spending_limit_usd:
                status = "stopped-at-spending-limit"
                break
            rerun_of, tries = None, 0
            while True:
                outcome = self.run_one(task, arm_id, repeat, rerun_of)
                results.append(outcome)
                with self.outcomes.open("a", encoding="utf-8") as handle:
                    handle.write(json.dumps(outcome, sort_keys=True) + "\n")
                print(
                    f"{outcome['run_id']}: {outcome['status']}"
                    + (f" success={outcome['success']}" if outcome["success"] is not None else "")
                    + (f" ({outcome['error']})" if outcome["error"] else ""),
                    flush=True,
                )
                if outcome["status"] == "infrastructure-failure" and tries < self.args.retries:
                    tries += 1
                    rerun_of = outcome["run_id"]
                    continue
                break
            if outcome["status"] == "interrupted":
                status = "interrupted"
                break
        return results, status


class InfrastructureFailure(Exception):
    pass


def parse_args(argv):
    p = argparse.ArgumentParser(description="Run the stilltrue agent-task benchmark.")
    p.add_argument("--agent", choices=AGENTS, required=True)
    p.add_argument("--host", choices=arms_mod.HOSTS, required=True)
    p.add_argument("--arms", default="A,C", help="comma-separated subset of A,B,C,D")
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--split", choices=("development", "holdout", "all"), default="development")
    p.add_argument("--holdout-use", choices=("exploratory", "confirmatory"),
                   help="required when the split includes held-out tasks; recorded")
    p.add_argument("--tasks", help="comma-separated task ids (default: the whole split)")
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--stilltrue-bin-dir", type=Path)
    p.add_argument("--timeout", type=float, default=300.0, help="seconds per run")
    p.add_argument("--retries", type=int, default=1,
                   help="reruns of an infrastructure failure, each linked to the original")
    p.add_argument("--model", help="live runs: the model to pin (recorded)")
    p.add_argument("--authorized", action="store_true", help="live runs: spending is authorized")
    p.add_argument("--spending-limit-usd", type=float, help="live runs: total spending ceiling")
    p.add_argument("--keep-worktrees", action="store_true")
    p.add_argument("--work-dir", type=Path, help="where run directories are created")
    p.add_argument("--no-analyze", action="store_true")
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    # Refuse a live run before touching anything: no directory, no build, no host.
    try:
        live.authorize(args.agent, args.host, args.authorized, args.spending_limit_usd)
    except live.Refused as error:
        print(f"run.py: {error}", file=sys.stderr)
        return 2
    arm_ids = [a.strip() for a in args.arms.split(",") if a.strip()]
    bad = [a for a in arm_ids if a not in arms_mod.ARMS]
    if bad or not arm_ids or len(set(arm_ids)) != len(arm_ids):
        print(f"run.py: --arms must be distinct letters from A,B,C,D, not {args.arms!r}", file=sys.stderr)
        return 2
    if args.repeats < 1:
        print("run.py: --repeats must be at least 1", file=sys.stderr)
        return 2
    if args.split in ("holdout", "all") and not args.holdout_use:
        print("run.py: running held-out tasks needs --holdout-use exploratory|confirmatory, "
              "which is recorded in the manifest", file=sys.stderr)
        return 2
    try:
        tasks = manifest.load_tasks(split=args.split, ids=args.tasks.split(",") if args.tasks else None)
    except manifest.ManifestError as error:
        print(f"run.py: {error}", file=sys.stderr)
        return 2
    if (args.out / "outcomes.jsonl").exists():
        print(f"run.py: {args.out} already holds outcomes; choose a new --out", file=sys.stderr)
        return 2
    bin_dir = locate_binaries(args.stilltrue_bin_dir)
    needs_bin = any(arms_mod.ARMS[a].checker_on_path or arms_mod.ARMS[a].hooks for a in arm_ids)
    if needs_bin and bin_dir is None:
        print("run.py: arms B, C and D need stilltrue and stilltrue-hook; build them with "
              "`cargo build` or pass --stilltrue-bin-dir", file=sys.stderr)
        return 2

    args.out.mkdir(parents=True, exist_ok=True)
    plan, orders = schedule(tasks, arm_ids, args.repeats, args.seed)
    work_root = Path(tempfile.mkdtemp(prefix="stilltrue-bench-", dir=args.work_dir))
    frozen = manifest.load_frozen()
    record = {
        "schema_version": SCHEMA_VERSION,
        "run_set_id": uuid.uuid4().hex,
        "status": "running",
        "created_at": now(),
        "finished_at": None,
        "stilltrue": {
            "revision": revision(ROOT),
            "version": tool_version([str(bin_dir / "stilltrue"), "--version"]) if bin_dir else None,
            "binaries": {
                name: {"path": str(bin_dir / name), "sha256": sha256_file(bin_dir / name)}
                for name in ("stilltrue", "stilltrue-hook")
            } if bin_dir else None,
        },
        "agent": args.agent,
        "host": args.host,
        "host_version": "simulated (scripted host)" if args.agent == "scripted" else live.host_version(args.agent),
        "model": "scripted" if args.agent == "scripted" else (args.model or "host default (not pinned)"),
        "reasoning": None,
        "machine": {"arch": platform.machine(), "cpus": os.cpu_count(), "system": platform.system()},
        "platform": {
            "platform": platform.platform(),
            "python": platform.python_version(),
            "git": tool_version(["git", "--version"]),
        },
        "seed": args.seed,
        "arms": arm_ids,
        "arm_order": orders,
        "arm_installs": arms_mod.describe(args.host),
        "repeats": args.repeats,
        "split": args.split,
        "holdout_use": args.holdout_use,
        "tasks": [t.id for t in tasks],
        "task_digests": {
            t.id: hashlib.sha256(
                b"".join(p.read_bytes() for p in sorted(t.dir.iterdir()) if p.is_file())
                + t.build_script.read_bytes()
            ).hexdigest()
            for t in tasks
        },
        "timeout_seconds": args.timeout,
        "retry_rule": {"infrastructure_retries": args.retries,
                       "note": "each rerun is a new outcome linked by rerun_of"},
        "session_protocol": {"turns": ["task", "review"], "review_prompt": REVIEW_PROMPT,
                             "host_continuation_limit": hostsim.CONTINUATION_LIMIT},
        "authorized": bool(args.authorized),
        "spending_limit_usd": args.spending_limit_usd,
        "frozen": frozen,
        "outcomes_file": "outcomes.jsonl",
        "work_dir": str(work_root),
    }
    write_json(args.out / "manifest.json", record)

    runner = Runner(args, tasks, bin_dir, work_root)
    try:
        results, status = runner.run(plan)
    finally:
        if not args.keep_worktrees:
            shutil.rmtree(work_root, ignore_errors=True)
    counts: dict[str, int] = {}
    for outcome in results:
        counts[outcome["status"]] = counts.get(outcome["status"], 0) + 1
    record.update(status=status, finished_at=now(), counts=counts,
                  spent_usd=None if args.agent == "scripted" else runner.spent)
    write_json(args.out / "manifest.json", record)
    if not args.no_analyze:
        import analyze

        analyze.write_reports(args.out)
        print(f"report: {args.out / 'summary.md'}")
    return 130 if status == "interrupted" else 0


if __name__ == "__main__":
    sys.exit(main())
