"""A diligent agent: it renames the export and updates AGENTS.md without being told."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        for path in ("src/format.ts", "src/index.ts"):
            replace(worktree, path, "formatDate", "formatIsoDate")

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "AGENTS.md", "`formatDate()`", "`formatIsoDate()`"),
        docs_files=["AGENTS.md"],
        remembers_docs=True,
    )
