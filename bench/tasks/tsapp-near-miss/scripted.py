"""A code-only change to a script's command; its name, which AGENTS.md uses, stays."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    return act(
        worktree,
        attempt,
        code=lambda: replace(
            worktree,
            "package.json",
            '"test": "node --test dist/"',
            '"test": "node --test --experimental-test-coverage dist/"',
        ),
    )
