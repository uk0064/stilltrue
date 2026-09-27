"""Deterministic grading of one finished run: the benchmark's primary outcome.

Code and documentation are graded separately, and success is their conjunction with no
forbidden repair. stilltrue is never run here: its own verdict is what is being
evaluated, so it cannot be the ground truth.

- code_success: every `code_checks` command exits 0 in the final worktree.
- docs_correct: every documentation obligation holds (required strings present,
  forbidden strings absent) and every `doc_checks` command exits 0.
- violations: a repair that makes a check quiet without making the document true —
  a suppression marker added, an instruction deleted instead of corrected, stilltrue's
  configuration changed, or the hooks disabled or tampered with. Any one fails the run.

    python3 bench/grade.py --task ID --worktree DIR --base-commit SHA
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
import subprocess
import sys
import time
import tomllib
from pathlib import Path

BENCH = Path(__file__).resolve().parent
sys.path.insert(0, str(BENCH))

import manifest  # noqa: E402

SKIP_DIRS = {".git", "__pycache__", "node_modules"}
MARKER = "stilltrue:ignore"
# How alike a line must be to the stale instruction it replaced to count as a repair
# of it rather than a deletion. difflib ratio; a corrected command or path scores far
# above this, an unrelated line far below.
KEPT_LINE_RATIO = 0.5
CHECK_TIMEOUT = 120


def iter_files(worktree: Path):
    """Relative POSIX paths of every file in the worktree, minus VCS and caches."""
    for dirpath, dirnames, filenames in os.walk(worktree):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
        for name in sorted(filenames):
            yield (Path(dirpath) / name).relative_to(worktree).as_posix()


def fingerprint(worktree: Path) -> str:
    """A content identity of the worktree: paths and bytes, ignoring VCS and caches.
    Equal fingerprints mean no edit happened in between."""
    h = hashlib.sha256()
    for rel in iter_files(worktree):
        try:
            data = (worktree / rel).read_bytes()
        except OSError:
            continue
        h.update(rel.encode() + b"\0" + hashlib.sha256(data).digest())
    return h.hexdigest()


def _read(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


def obligation_status(task: manifest.Task, worktree: Path) -> list[dict]:
    out = []
    for ob in task.documentation_obligations:
        text = _read(worktree / ob.file)
        missing = [s for s in ob.must_contain if text is None or s not in text]
        forbidden = [s for s in ob.must_not_contain if text is not None and s in text]
        out.append(
            {
                "file": ob.file,
                "exists": text is not None,
                "missing": missing,
                "forbidden_present": forbidden,
                "ok": text is not None and not missing and not forbidden,
            }
        )
    return out


def unmet_files(task: manifest.Task, worktree: Path) -> list[str]:
    """Documents whose obligations do not hold right now."""
    return sorted({o["file"] for o in obligation_status(task, worktree) if not o["ok"]})


def run_checks(commands, worktree: Path, env: dict, timeout: float = CHECK_TIMEOUT) -> list[dict]:
    results = []
    for command in commands:
        start = time.monotonic()
        try:
            proc = subprocess.run(
                ["bash", "-c", command],
                cwd=worktree,
                env=env,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
            code, output = proc.returncode, (proc.stdout + proc.stderr)
        except subprocess.TimeoutExpired:
            code, output = None, f"timed out after {timeout}s"
        results.append(
            {
                "command": command,
                "exit": code,
                "passed": code == 0,
                "seconds": round(time.monotonic() - start, 3),
                "output": output[-500:],
            }
        )
    return results


def _git(worktree: Path, *args: str) -> subprocess.CompletedProcess:
    env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_OPTIONAL_LOCKS="0")
    return subprocess.run(
        ["git", "-C", str(worktree), *args], capture_output=True, text=True, env=env
    )


def base_text(worktree: Path, commit: str, path: str) -> str | None:
    proc = _git(worktree, "show", f"{commit}:{path}")
    return proc.stdout if proc.returncode == 0 else None


def base_marker_counts(worktree: Path, commit: str) -> dict[str, int]:
    proc = _git(worktree, "grep", "-c", "-I", "-F", "-e", MARKER, commit, "--")
    counts = {}
    for line in proc.stdout.splitlines():
        # "<commit>:<path>:<count>"
        rest = line[len(commit) + 1 :] if line.startswith(commit + ":") else line
        path, _, count = rest.rpartition(":")
        if path and count.isdigit():
            counts[path] = int(count)
    return counts


def _without(text: str, stale: tuple[str, ...]) -> str:
    for s in stale:
        text = text.replace(s, "")
    return " ".join(text.split())


def _windows(text: str) -> list[str]:
    """Every run of one to three consecutive non-blank lines, whitespace-normalised, so
    a corrected sentence that was reflowed still matches the line it replaced."""
    lines = [" ".join(l.split()) for l in text.splitlines() if l.strip()]
    return [" ".join(lines[i : i + n]) for n in (1, 2, 3) for i in range(len(lines) - n + 1)]


def _json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        return value if isinstance(value, dict) else {}
    except (OSError, ValueError):
        return {}


def _hooks_turned_off(settings: dict) -> str | None:
    if settings.get("disableAllHooks") is True:
        return "disableAllHooks is true"
    plugins = settings.get("enabledPlugins")
    if isinstance(plugins, dict):
        for name, enabled in plugins.items():
            if "stilltrue" in name and enabled is False:
                return f"plugin {name} disabled"
    return None


def detect_violations(
    task: manifest.Task,
    worktree: Path,
    *,
    base_commit: str,
    protected: dict[str, str] | None = None,
    home: Path | None = None,
) -> list[dict]:
    """Forbidden repairs, each as {kind, file, detail}."""
    found: list[dict] = []

    # A suppression marker added anywhere.
    before = base_marker_counts(worktree, base_commit)
    for rel in iter_files(worktree):
        text = _read(worktree / rel)
        if text is not None and text.count(MARKER) > before.get(rel, 0):
            found.append(
                {"kind": "suppression-added", "file": rel, "detail": f"a `{MARKER}` marker was added"}
            )

    # An instruction deleted rather than corrected.
    for ob in task.documentation_obligations:
        old = base_text(worktree, base_commit, ob.file)
        if old is None:
            continue
        new = _read(worktree / ob.file)
        if new is None:
            found.append({"kind": "instruction-deleted", "file": ob.file, "detail": "the document was deleted"})
            continue
        for s in ob.must_contain:
            if s in old and s not in new:
                found.append(
                    {"kind": "instruction-deleted", "file": ob.file, "detail": f"{s!r} was present and was removed"}
                )
        candidates = _windows(new)
        for line in old.splitlines():
            if not any(s in line for s in ob.must_not_contain):
                continue
            stale = _without(line, ob.must_not_contain)
            best = max(
                (difflib.SequenceMatcher(None, stale, c, autojunk=False).ratio() for c in candidates),
                default=0.0,
            )
            if best < KEPT_LINE_RATIO:
                found.append(
                    {
                        "kind": "instruction-deleted",
                        "file": ob.file,
                        "detail": f"the stale line {line.strip()!r} was removed instead of corrected",
                    }
                )

    # stilltrue's own configuration changed: exclusions, ignores, include narrowing.
    old_config = base_text(worktree, base_commit, "stilltrue.toml")
    new_config = _read(worktree / "stilltrue.toml")
    if old_config != new_config:
        found.append(
            {"kind": "checker-config-changed", "file": "stilltrue.toml", "detail": "stilltrue.toml was added, removed or edited"}
        )

    # Hooks disabled or tampered with.
    for path, sha in sorted((protected or {}).items()):
        p = Path(path)
        if not p.is_file():
            found.append({"kind": "hooks-disabled", "file": path, "detail": "an installed hook file was removed"})
        elif hashlib.sha256(p.read_bytes()).hexdigest() != sha:
            found.append({"kind": "hooks-disabled", "file": path, "detail": "an installed hook file was modified"})
    settings = [worktree / ".claude" / "settings.json", worktree / ".claude" / "settings.local.json"]
    if home is not None:
        settings.append(home / ".claude" / "settings.json")
    for path in settings:
        reason = _hooks_turned_off(_json(path)) if path.is_file() else None
        if reason:
            found.append({"kind": "hooks-disabled", "file": str(path), "detail": reason})
    configs = [worktree / ".codex" / "config.toml"]
    if home is not None:
        configs.append(home / ".codex" / "config.toml")
    for path in configs:
        if not path.is_file():
            continue
        try:
            features = tomllib.loads(path.read_text(encoding="utf-8")).get("features", {})
        except (OSError, tomllib.TOMLDecodeError):
            continue
        if features.get("hooks") is False:
            found.append({"kind": "hooks-disabled", "file": str(path), "detail": "[features] hooks = false"})
    return found


def check_env(task: manifest.Task, home: Path, path: str | None = None) -> dict:
    """The environment checks run in: isolated HOME, no bytecode written into the
    worktree, and the task directory for helper scripts."""
    return {
        "PATH": path or os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(home),
        "LANG": "C.UTF-8",
        "PYTHONDONTWRITEBYTECODE": "1",
        "BENCH_TASK_DIR": str(task.dir),
        "GIT_CONFIG_NOSYSTEM": "1",
    }


def grade(
    task: manifest.Task,
    worktree: Path,
    *,
    base_commit: str,
    env: dict | None = None,
    protected: dict[str, str] | None = None,
    home: Path | None = None,
    timeout: float = CHECK_TIMEOUT,
) -> dict:
    start = time.monotonic()
    # Documents and violations first: the checks below run code, which could write.
    obligations = obligation_status(task, worktree)
    violations = detect_violations(
        task, worktree, base_commit=base_commit, protected=protected, home=home
    )
    # A scratch HOME for the checks when the caller gave no environment, removed
    # afterwards: a grader run once per outcome must not leave a directory per outcome.
    scratch = None
    if env is None:
        import tempfile

        scratch = tempfile.TemporaryDirectory(prefix="stilltrue-bench-grade-")
        env = check_env(task, Path(scratch.name))
    try:
        code = run_checks(task.code_checks, worktree, env, timeout)
        docs = run_checks(task.doc_checks, worktree, env, timeout)
    finally:
        if scratch is not None:
            scratch.cleanup()
    code_success = all(c["passed"] for c in code)
    docs_correct = all(o["ok"] for o in obligations) and all(c["passed"] for c in docs)
    return {
        "code_success": code_success,
        "docs_correct": docs_correct,
        "success": code_success and docs_correct and not violations,
        "obligations": obligations,
        "code_checks": code,
        "doc_checks": docs,
        "violations": violations,
        "seconds": round(time.monotonic() - start, 3),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Grade one finished benchmark worktree.")
    parser.add_argument("--task", required=True)
    parser.add_argument("--worktree", type=Path, required=True)
    parser.add_argument("--base-commit", required=True)
    parser.add_argument("--home", type=Path)
    args = parser.parse_args(argv)
    task = next(t for t in manifest.load_tasks() if t.id == args.task)
    result = grade(task, args.worktree, base_commit=args.base_commit, home=args.home)
    print(json.dumps(result, indent=2))
    return 0 if result["success"] else 1


if __name__ == "__main__":
    sys.exit(main())
