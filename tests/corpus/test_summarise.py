"""The corpus summary states rates with denominators and never counts a failure as quiet."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from summarise import ratio, summarise  # noqa: E402


def result(repositories, status="complete"):
    return {
        "status": status,
        "tool": {"version": "0.1.0", "revision": "abcdef1234567890"},
        "machine": {"platform": "test"},
        "startedAt": "2026-09-26T00:00:00",
        "repositories": repositories,
    }


def repo(name, status="complete", findings=(), **extra):
    entry = {"name": name, "commit": "0" * 40, "status": status, "findings": list(findings)}
    entry.update(extra)
    return entry


def finding(label):
    return {"label": label, "claim": "x", "file": "README.md", "line": 1, "note": ""}


class Summary(unittest.TestCase):
    def test_precision_is_over_reviewed_findings_only(self):
        text = summarise(result([
            repo("a", findings=[finding("true-positive"), finding("false-positive")]),
            repo("b", findings=[finding("unreviewed")]),
        ]))
        self.assertIn("Precision over reviewed findings: 1/2 (50%)", text)
        self.assertIn("1 true positive, 1 false positive, 1 unreviewed", text)

    def test_no_reviewed_findings_is_said_rather_than_a_rate(self):
        self.assertEqual(ratio(0, 0), "no reviewed findings")
        text = summarise(result([repo("a")]))
        self.assertIn("Precision over reviewed findings: no reviewed findings", text)

    def test_a_failed_or_incomplete_repository_is_never_quiet(self):
        text = summarise(result([
            repo("clean"),
            repo("broken", status="failed", error="clone failed"),
            repo("partial", status="incomplete"),
        ], status="incomplete"))
        self.assertIn("complete and quiet: 1 of 3", text)
        self.assertIn("incomplete: partial", text)
        self.assertIn("failed: broken", text)
        self.assertIn("**incomplete**", text)
        self.assertIn("failed (clone failed)", text)

    def test_times_are_totalled_with_the_slowest_named(self):
        text = summarise(result([
            repo("a", coldMs=100, warmMs=10),
            repo("b", coldMs=300, warmMs=30),
        ]))
        self.assertIn("cold 400 ms in total (slowest 300 ms)", text)
        self.assertIn("warm 40 ms in total (slowest 30 ms)", text)


if __name__ == "__main__":
    unittest.main()
