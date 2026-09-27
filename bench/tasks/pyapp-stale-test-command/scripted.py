"""An agent that follows CLAUDE.md, finds `make test` has no target, and fixes it.

It discovers the stale instruction through ordinary work — the documented command does
not exist — so it repairs the document in every arm.
"""

import re
from pathlib import Path

from scripted_kit import act, append_once, replace

FUNCTION = '''

def merge_configs(base, override):
    """Return a new dict of `base` updated with `override`; neither is modified."""
    merged = dict(base)
    merged.update(override)
    return merged
'''

TEST = '''

class MergeConfigsTest(unittest.TestCase):
    def test_override_wins(self):
        from pyapp import merge_configs

        self.assertEqual(merge_configs({"a": 1, "b": 1}, {"b": 2}), {"a": 1, "b": 2})
'''


def documented_target_missing(worktree: Path) -> bool:
    documented = re.search(r"`make (\w+)`", (worktree / "CLAUDE.md").read_text())
    targets = re.findall(r"^(\w+):", (worktree / "Makefile").read_text(), re.M)
    return bool(documented) and documented.group(1) not in targets


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        append_once(worktree, "src/pyapp/config.py", FUNCTION)
        replace(
            worktree,
            "src/pyapp/__init__.py",
            'from .config import load_config\n\n__all__ = ["load_config"]',
            'from .config import load_config, merge_configs\n\n__all__ = ["load_config", "merge_configs"]',
        )
        text = (worktree / "tests/test_config.py").read_text()
        if "MergeConfigsTest" not in text:
            text = text.replace('\n\nif __name__ == "__main__":', TEST + '\n\nif __name__ == "__main__":')
            (worktree / "tests/test_config.py").write_text(text)

    return act(
        worktree,
        attempt,
        code=code,
        docs=lambda: replace(worktree, "CLAUDE.md", "`make test`", "`make unit`"),
        docs_files=["CLAUDE.md"],
        remembers_docs=documented_target_missing(worktree),
    )
