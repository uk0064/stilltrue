"""What a scripted solution can see and do.

The scripted agent stands in for a model so the whole pipeline — arms, hooks, agent,
grading, analysis — runs deterministically and for free. It is not evidence about any
real agent: its behaviour is a fixed policy chosen to exercise the harness.

The policy (``act``) never looks at the arm. Like a live agent, it acts only on what it
can observe: the task prompt, its context file (what the host injected: hook context,
a block's reason, standing instructions), and whether ``stilltrue`` is on its PATH.

- It always makes the task's code change.
- It updates documentation when it "remembers to" (a per-task trait), or when a
  stilltrue finding in its context names a document the repair touches, or — when a
  standing instruction tells it to run stilltrue and the checker is on PATH — when its
  own run of the checker names such a document.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from pathlib import Path
from typing import Callable, Iterable

# The phrase a standing instruction uses to tell the agent to run the checker itself
# (arm B's instruction contains it; see arms.INSTRUCTION).
CHECK_INSTRUCTION = "run `stilltrue`"
# One quoted finding line, as stilltrue-hook writes it: "- FILE:LINE TIER: message".
FINDING = re.compile(r"^- (?P<file>.+?):(?P<line>\d+) (?P<tier>rot|lie|\S+): ")


def context() -> str:
    path = os.environ.get("BENCH_CONTEXT_FILE")
    if not path or not Path(path).is_file():
        return ""
    return Path(path).read_text(encoding="utf-8")


def prompt() -> str:
    path = os.environ.get("BENCH_PROMPT_FILE")
    if not path or not Path(path).is_file():
        return ""
    return Path(path).read_text(encoding="utf-8")


def evidence_files(text: str) -> set[str]:
    """Documents named by stilltrue findings quoted in `text`."""
    return {m.group("file") for m in map(FINDING.match, text.splitlines()) if m}


def instructed_to_check(text: str) -> bool:
    return CHECK_INSTRUCTION in text


def checker_findings(worktree: Path) -> list[dict] | None:
    """Run `stilltrue` from PATH, as an agent following an instruction would. None when
    it is not installed or its output cannot be read."""
    checker = shutil.which("stilltrue")
    if checker is None:
        return None
    result = subprocess.run(
        [checker, "--format", "json", "--fail-on", "none"],
        cwd=worktree,
        capture_output=True,
        text=True,
        timeout=60,
    )
    try:
        return json.loads(result.stdout).get("findings", [])
    except (json.JSONDecodeError, AttributeError):
        return None


def replace(worktree: Path, path: str, old: str, new: str) -> None:
    """Replace every `old` with `new` in one file. A no-op once applied."""
    target = worktree / path
    text = target.read_text(encoding="utf-8")
    target.write_text(text.replace(old, new), encoding="utf-8")


def write(worktree: Path, path: str, text: str) -> None:
    target = worktree / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, encoding="utf-8")


def append_once(worktree: Path, path: str, text: str) -> None:
    target = worktree / path
    current = target.read_text(encoding="utf-8")
    if text.strip() not in current:
        target.write_text(current.rstrip("\n") + "\n" + text, encoding="utf-8")


def log(record: dict) -> None:
    path = os.environ.get("BENCH_AGENT_LOG")
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, sort_keys=True) + "\n")


def act(
    worktree: Path,
    attempt: int,
    *,
    code: Callable[[], None] | None = None,
    docs: Callable[[], None] | None = None,
    docs_files: Iterable[str] = (),
    remembers_docs: bool = False,
) -> str:
    """Apply the shared policy and return the agent's closing message."""
    docs_files = set(docs_files)
    seen = context()
    if code is not None:
        code()
    reason = "remembered" if remembers_docs else None
    if docs is not None and reason is None and evidence_files(seen) & docs_files:
        reason = "context-finding"
    if docs is not None and reason is None and instructed_to_check(seen):
        found = checker_findings(worktree)
        if found and {f.get("file") for f in found} & docs_files:
            reason = "own-check"
    if docs is not None and reason is not None:
        docs()
    log({"attempt": attempt, "code": code is not None, "docs": reason})
    return f"attempt {attempt}: code {'changed' if code else 'unchanged'}; docs {reason or 'untouched'}"
