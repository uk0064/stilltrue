#!/usr/bin/env bash
# pyapp: a small Python package driven by a Makefile. Clean at HEAD: every instruction
# in CLAUDE.md describes the repository.
source "$(dirname "$0")/../_bench.sh"
init "$1"

write Makefile <<'M'
.PHONY: test lint

test:
	PYTHONPATH=src python3 -m unittest discover -s tests -q

lint:
	python3 -m compileall -q src
M
write src/pyapp/__init__.py <<'P'
from .config import load_config

__all__ = ["load_config"]
P
write src/pyapp/config.py <<'P'
"""Configuration loading."""

import json


def load_config(path):
    """Read a JSON configuration file and return it as a dict."""
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)
P
write tests/test_config.py <<'P'
import json
import os
import tempfile
import unittest

from pyapp import load_config


class LoadConfigTest(unittest.TestCase):
    def test_reads_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "c.json")
            with open(path, "w", encoding="utf-8") as handle:
                json.dump({"name": "demo"}, handle)
            self.assertEqual(load_config(path), {"name": "demo"})


if __name__ == "__main__":
    unittest.main()
P
write CLAUDE.md <<'D'
# pyapp

Run `make test` before you finish, and `make lint` to byte-compile the sources.

Configuration is read by `load_config()` in `src/pyapp/config.py`.
D
write README.md <<'D'
# pyapp

A small configuration library. Contributors: see `CLAUDE.md`.
D
commit "Start pyapp with a config loader"

write src/pyapp/config.py <<'P'
"""Configuration loading."""

import json


def load_config(path):
    """Read a JSON configuration file and return it as a dict."""
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"{path}: expected a JSON object")
    return data
P
commit "Reject configuration that is not an object"
