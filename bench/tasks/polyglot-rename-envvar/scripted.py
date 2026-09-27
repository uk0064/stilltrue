"""An agent that renames the variable in code and tests, and forgets CLAUDE.md."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        for path in ("ingest.py", "test_ingest.py"):
            replace(worktree, path, "EVENTS_DB_URL", "EVENTS_DATABASE_URL")

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "CLAUDE.md", "EVENTS_DB_URL", "EVENTS_DATABASE_URL"),
        docs_files=["CLAUDE.md"],
    )
