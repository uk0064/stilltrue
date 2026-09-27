"""An agent that renames the function and forgets CLAUDE.md, where no check can see it."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        for path in ("ingest.py", "test_ingest.py"):
            replace(worktree, path, "fold_events", "collapse_events")

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "CLAUDE.md", "fold_events", "collapse_events"),
        docs_files=["CLAUDE.md"],
    )
