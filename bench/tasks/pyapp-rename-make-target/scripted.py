"""An agent that renames the target and forgets the document that names it."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        replace(worktree, "Makefile", ".PHONY: test lint", ".PHONY: test check")
        replace(worktree, "Makefile", "\nlint:", "\ncheck:")

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "CLAUDE.md", "`make lint`", "`make check`"),
        docs_files=["CLAUDE.md"],
    )
