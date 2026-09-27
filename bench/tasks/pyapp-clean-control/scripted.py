"""A code-only change that no document describes."""

from pathlib import Path

from scripted_kit import act, append_once, replace, write

FUNCTION = '''

def config_keys(config):
    """The configuration's top-level keys, sorted."""
    return sorted(config)
'''

TEST = '''import unittest

from pyapp import config_keys


class ConfigKeysTest(unittest.TestCase):
    def test_sorted(self):
        self.assertEqual(config_keys({"b": 1, "a": 2}), ["a", "b"])


if __name__ == "__main__":
    unittest.main()
'''


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        append_once(worktree, "src/pyapp/config.py", FUNCTION)
        replace(
            worktree,
            "src/pyapp/__init__.py",
            'from .config import load_config\n\n__all__ = ["load_config"]',
            'from .config import config_keys, load_config\n\n__all__ = ["config_keys", "load_config"]',
        )
        write(worktree, "tests/test_keys.py", TEST)

    return act(worktree, attempt, code=code)
