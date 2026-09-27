"""An agent that renames the function in code and tests, and forgets CLAUDE.md."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        for path in ("src/pyapp/config.py", "src/pyapp/__init__.py", "tests/test_config.py"):
            replace(worktree, path, "load_config", "read_config")

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "CLAUDE.md", "load_config", "read_config"),
        docs_files=["CLAUDE.md"],
    )
