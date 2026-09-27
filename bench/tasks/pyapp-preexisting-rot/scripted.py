"""A code-only change in a repository with unrelated rot already present."""

from pathlib import Path

from scripted_kit import act, write

CONFIG = '''"""Configuration loading."""

import json

_MISSING = object()


def load_config(path, default=_MISSING):
    """Read a JSON configuration file and return it as a dict.

    When the file does not exist, return `default` if one was given.
    """
    try:
        handle = open(path, encoding="utf-8")
    except FileNotFoundError:
        if default is _MISSING:
            raise
        return default
    with handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"{path}: expected a JSON object")
    return data
'''

TEST = '''import unittest

from pyapp import load_config


class DefaultTest(unittest.TestCase):
    def test_missing_file_returns_default(self):
        self.assertEqual(load_config("/nonexistent/x.json", default={}), {})

    def test_missing_file_without_default_raises(self):
        with self.assertRaises(FileNotFoundError):
            load_config("/nonexistent/x.json")


if __name__ == "__main__":
    unittest.main()
'''


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        write(worktree, "src/pyapp/config.py", CONFIG)
        write(worktree, "tests/test_default.py", TEST)

    return act(worktree, attempt, code=code)
