#!/usr/bin/env bash
# polyglot: a Python ingester beside a Go server. With two Go files present no symbol
# resolver reads the whole repository, so stilltrue abstains on symbols here by design
# (ADR-0004); paths and environment variables are still checked.
source "$(dirname "$0")/../_bench.sh"
init "$1"

write ingest.py <<'P'
"""Event ingestion."""

import os


def fold_events(events):
    """Merge consecutive duplicate events."""
    out = []
    for event in events:
        if not out or out[-1] != event:
            out.append(event)
    return out


def database_url():
    """Where ingested events are stored."""
    return os.environ["EVENTS_DB_URL"]
P
write test_ingest.py <<'P'
import os
import unittest
from unittest import mock

import ingest


class IngestTest(unittest.TestCase):
    def test_folds_duplicates(self):
        self.assertEqual(ingest.fold_events([1, 1, 2, 2, 1]), [1, 2, 1])

    def test_database_url(self):
        with mock.patch.dict(os.environ, {"EVENTS_DB_URL": "sqlite://x"}):
            self.assertEqual(ingest.database_url(), "sqlite://x")


if __name__ == "__main__":
    unittest.main()
P
write cmd/server/main.go <<'G'
package main

import "net/http"

func main() {
	routes()
	http.ListenAndServe(":8080", nil)
}
G
write cmd/server/routes.go <<'G'
package main

import "net/http"

func routes() {
	http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("events\n"))
	})
}
G
write CLAUDE.md <<'D'
# events

Set `EVENTS_DB_URL` before running the ingester. Duplicate events are merged by
`fold_events()` in `ingest.py`. The Go server lives in `cmd/server/main.go`.
D
commit "Start the events ingester and server"

write ingest.py <<'P'
"""Event ingestion."""

import os


def fold_events(events):
    """Merge consecutive duplicate events, keeping the first of each run."""
    out = []
    for event in events:
        if not out or out[-1] != event:
            out.append(event)
    return out


def database_url():
    """Where ingested events are stored."""
    return os.environ["EVENTS_DB_URL"]
P
commit "Clarify which duplicate fold_events keeps"
