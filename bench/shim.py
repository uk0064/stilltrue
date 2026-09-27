"""A recording pass-through for `stilltrue-hook` and `stilltrue`.

The runner puts wrappers first on the session's PATH that exec this file, so whatever
calls the checker — a host's hook runtime, the simulated host, or the agent itself —
reaches the real binary unchanged while the benchmark records the call: arguments,
exit status, wall time of the real binary alone, and for a hook its payload and answer.
At each Stop it also records the worktree's content fingerprint and which documentation
obligations were unmet at that moment, which is how a block is later judged necessary
or not without trusting the adapter's own account.

Environment: BENCH_SHIM_LOG (a JSON-lines file), BENCH_REAL_BIN_DIR (the real
binaries), BENCH_TASK_DIR (optional; the task whose obligations to evaluate).
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path

BENCH = Path(__file__).resolve().parent


def _log(record: dict) -> None:
    path = os.environ.get("BENCH_SHIM_LOG")
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, sort_keys=True) + "\n")


def _observe(cwd: Path) -> dict:
    """The worktree as the hook is about to see it."""
    sys.path.insert(0, str(BENCH))
    import grade
    import manifest

    out = {"fingerprint": grade.fingerprint(cwd)}
    task_dir = os.environ.get("BENCH_TASK_DIR")
    if task_dir:
        out["unmet_files"] = grade.unmet_files(manifest.load_task(Path(task_dir)), cwd)
    return out


def main() -> int:
    name, args = sys.argv[1], sys.argv[2:]
    real = Path(os.environ.get("BENCH_REAL_BIN_DIR", "")) / name
    record: dict = {"program": name, "args": args, "cwd": os.getcwd(), "started_at": time.time()}
    stdin = sys.stdin.buffer.read() if name == "stilltrue-hook" else None
    if stdin is not None:
        try:
            payload = json.loads(stdin)
        except ValueError:
            payload = {}
        record["event"] = payload.get("hook_event_name")
        record["stop_hook_active"] = payload.get("stop_hook_active")
        record["session_id"] = payload.get("session_id")
        if record["event"] == "Stop" and payload.get("cwd"):
            record.update(_observe(Path(payload["cwd"])))
    start = time.monotonic()
    try:
        proc = subprocess.run(
            [str(real), *args],
            input=stdin,
            stdin=None if stdin is not None else sys.stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        record.update(exit=127, seconds=round(time.monotonic() - start, 4), error=str(error))
        _log(record)
        print(f"{name}: {error}", file=sys.stderr)
        return 127
    record["seconds"] = round(time.monotonic() - start, 4)
    record["exit"] = proc.returncode
    if stdin is not None:
        record["stdout"] = proc.stdout.decode("utf-8", "replace")[:16384]
    _log(record)
    sys.stdout.buffer.write(proc.stdout)
    sys.stderr.buffer.write(proc.stderr)
    return proc.returncode


def install(directory: Path, python: str = sys.executable) -> Path:
    """Write `stilltrue-hook` and `stilltrue` wrappers into `directory`."""
    directory.mkdir(parents=True, exist_ok=True)
    for name in ("stilltrue-hook", "stilltrue"):
        wrapper = directory / name
        wrapper.write_text(
            f'#!/bin/sh\nexec "{python}" "{Path(__file__).resolve()}" {name} "$@"\n',
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
    return directory


def read_log(path: Path) -> list[dict]:
    if not path.is_file():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


if __name__ == "__main__":
    sys.exit(main())
