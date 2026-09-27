"""An agent that moves the script and forgets the README that names it."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        old, new = worktree / "scripts/serve.py", worktree / "tools/serve.py"
        if old.exists():
            new.parent.mkdir(parents=True, exist_ok=True)
            old.rename(new)

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "README.md", "scripts/serve.py", "tools/serve.py"),
        docs_files=["README.md"],
    )
