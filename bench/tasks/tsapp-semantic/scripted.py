"""An agent that changes the output directory and forgets the prose that names it."""

from pathlib import Path

from scripted_kit import act, replace


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        replace(worktree, "tsconfig.json", '"outDir": "dist"', '"outDir": "build"')
        replace(worktree, "package.json", "node --test dist/", "node --test build/")

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "AGENTS.md", "the dist directory", "the build directory"),
        docs_files=["AGENTS.md"],
    )
