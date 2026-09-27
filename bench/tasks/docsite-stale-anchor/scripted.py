"""An agent that writes the new section and never follows the README's links."""

from pathlib import Path

from scripted_kit import act, append_once, replace

SECTION = """
## Upgrading

Replace the copied docs directory with the one from the new release.
"""


def solve(worktree: Path, arm: str, attempt: int) -> str:
    return act(
        worktree,
        attempt,
        code=lambda: append_once(worktree, "docs/install.md", SECTION),
        docs=lambda: replace(worktree, "README.md", "#requirements", "#prerequisites"),
        docs_files=["README.md"],
    )
