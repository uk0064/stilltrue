"""A deletion that a historical record mentions, correctly."""

from pathlib import Path

from scripted_kit import act


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        (worktree / "scripts/legacy_build.py").unlink(missing_ok=True)

    return act(worktree, attempt, code=code)
