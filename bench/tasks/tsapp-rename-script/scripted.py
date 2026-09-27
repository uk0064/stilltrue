"""An agent that renames the script and forgets AGENTS.md."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    return act(
        worktree,
        attempt,
        code=lambda: replace(worktree, "package.json", '"typecheck":', '"check:types":'),
        docs=lambda: replace(worktree, "AGENTS.md", "`npm run typecheck`", "`npm run check:types`"),
        docs_files=["AGENTS.md"],
    )
