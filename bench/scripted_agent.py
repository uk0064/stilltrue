"""Run one attempt of a task's scripted solution, as a separate process.

The runner starts this the way it would start a live agent: in the worktree, with the
run's isolated HOME and PATH, and with the host's injected context in the file named
by BENCH_CONTEXT_FILE. It loads ``bench/tasks/<id>/scripted.py`` and calls
``solve(worktree, arm, attempt)``; whatever that returns is the agent's closing message.

    python3 bench/scripted_agent.py --task-dir DIR --worktree DIR --arm A --attempt 1
"""

from __future__ import annotations

import argparse
import importlib.util
import sys
from pathlib import Path

BENCH = Path(__file__).resolve().parent


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--task-dir", type=Path, required=True)
    parser.add_argument("--worktree", type=Path, required=True)
    parser.add_argument("--arm", required=True)
    parser.add_argument("--attempt", type=int, required=True)
    args = parser.parse_args(argv)

    sys.path.insert(0, str(BENCH))
    path = args.task_dir / "scripted.py"
    spec = importlib.util.spec_from_file_location(f"scripted_{args.task_dir.name}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    message = module.solve(args.worktree, args.arm, args.attempt)
    print(message or "done")
    return 0


if __name__ == "__main__":
    sys.exit(main())
