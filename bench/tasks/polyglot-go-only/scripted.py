"""A Go-only change that no document describes."""

from pathlib import Path

from scripted_kit import act, replace

HANDLER = '''	http.HandleFunc("/healthz", func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("ok\\n"))
	})
}
'''


def solve(worktree: Path, arm: str, attempt: int) -> str:
    def code():
        path = worktree / "cmd/server/routes.go"
        if "/healthz" not in path.read_text():
            replace(worktree, "cmd/server/routes.go", "\t})\n}\n", "\t})\n" + HANDLER)

    return act(worktree, attempt, code=code)
