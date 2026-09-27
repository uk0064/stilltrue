"""A simulated host session, so the scripted agent can run the real adapter.

It plays the host's side of the hook protocol for Claude Code or Codex: it sends the
payload shapes the adapter's replay tests use (tests/hook/payloads/<host>/), runs each
hook command from the arm's installed configuration exactly as written, and does what
the host documents with the answer:

- SessionStart / UserPromptSubmit `additionalContext` goes into the agent's context;
- a Stop answer of `{"decision": "block"}` gives the agent one more attempt with the
  reason as its instruction, and the next Stop carries `stop_hook_active: true`;
- a Stop `systemMessage` is shown to the user, not the agent;
- a hook that exits 2 is treated as the host would (a Stop continues with stderr as the
  reason), and recorded as a contract violation — the adapter must never do it.

Codex sends no UserPromptSubmit here (its integration registers none) and numbers its
turns with `turn_id`; Claude Code counts turns at UserPromptSubmit.
"""

from __future__ import annotations

import json
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

BENCH = Path(__file__).resolve().parent
ROOT = BENCH.parent
sys.path.insert(0, str(BENCH))

import grade  # noqa: E402

PAYLOADS = ROOT / "tests" / "hook" / "payloads"
TEMPLATES = {
    ("claude-code", "SessionStart"): "session-start.json",
    ("claude-code", "UserPromptSubmit"): "user-prompt-submit.json",
    ("claude-code", "Stop"): "stop.json",
    ("codex", "SessionStart"): "session-start.json",
    ("codex", "Stop"): "stop.json",
    ("codex", "Interrupt"): "interrupt.json",
}
# A host-side safety net. The adapter asks for at most one continuation per user turn;
# if it ever asked for more, the simulated host stops the loop here and records it.
CONTINUATION_LIMIT = 3


class BudgetExhausted(Exception):
    """The run's time budget ran out."""


class AgentFailed(Exception):
    """The agent process failed."""


def payload(host: str, event: str, **fields) -> dict:
    template = json.loads((PAYLOADS / host / TEMPLATES[(host, event)]).read_text())
    if "model" in template:
        template["model"] = "scripted"
    template.update({k: v for k, v in fields.items() if v is not None or k in template})
    template["hook_event_name"] = event
    return template


def interpret(event: str, record: dict) -> dict:
    """What the host does with one hook's answer."""
    if record.get("missing") or record.get("timed_out"):
        return {"kind": "failed"}
    code, stdout, stderr = record["exit"], record["stdout"], record["stderr"]
    if code == 2:
        # Both hosts read exit 2 from a Stop hook as "keep going", stderr as the reason.
        if event == "Stop":
            return {"kind": "block", "text": stderr, "contract_violation": "exit 2"}
        return {"kind": "failed", "contract_violation": "exit 2"}
    if code != 0:
        return {"kind": "failed"}
    if not stdout.strip():
        return {"kind": "nothing"}
    try:
        answer = json.loads(stdout)
    except ValueError:
        if event in ("SessionStart", "UserPromptSubmit"):
            return {"kind": "context", "text": stdout, "contract_violation": "plain text"}
        return {"kind": "invalid"}
    if event == "Stop" and answer.get("decision") == "block":
        return {"kind": "block", "text": answer.get("reason", "")}
    context = (answer.get("hookSpecificOutput") or {}).get("additionalContext")
    if context and event in ("SessionStart", "UserPromptSubmit"):
        return {"kind": "context", "text": context}
    if answer.get("systemMessage"):
        return {"kind": "advise", "text": answer["systemMessage"]}
    return {"kind": "nothing"}


class ScriptedSession:
    """One scripted-agent session on a simulated host."""

    def __init__(
        self,
        *,
        task,
        arm: str,
        host: str,
        hooks: dict[str, list[dict]],
        worktree: Path,
        agent_dir: Path,
        agent_env: dict,
        hook_env: dict,
        preamble: str,
        turns: list[tuple[str, str]],
        session_id: str,
        deadline: float,
    ):
        self.task, self.arm, self.host, self.hooks = task, arm, host, hooks
        self.worktree, self.agent_dir = worktree, agent_dir
        self.agent_env, self.hook_env = agent_env, hook_env
        self.turns, self.session_id, self.deadline = turns, session_id, deadline
        self.context = agent_dir / "context.txt"
        self.transcript = agent_dir / "transcript.jsonl"
        self.context.write_text(preamble, encoding="utf-8")
        self.invocations: list[dict] = []
        self.attempt = 0
        self.agent_seconds = 0.0
        self.continuations = 0
        self.last_message = ""

    # -- plumbing ---------------------------------------------------------------------

    def _note(self, kind: str, **fields) -> None:
        with self.transcript.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps({"t": time.time(), "kind": kind, **fields}, sort_keys=True) + "\n")

    def _inject(self, source: str, text: str) -> None:
        with self.context.open("a", encoding="utf-8") as handle:
            handle.write(f"\n[{source}]\n{text}\n")
        self._note("context", source=source, text=text)

    def _remaining(self) -> float:
        left = self.deadline - time.monotonic()
        if left <= 0:
            raise BudgetExhausted()
        return left

    def hook(self, event: str, **fields) -> list[dict]:
        """Run every hook configured for `event`; return the host's reading of each."""
        results = []
        for spec in self.hooks.get(event, []):
            body = payload(self.host, event, session_id=self.session_id, cwd=str(self.worktree), **fields)
            argv = shlex.split(spec["command"])
            exe = shutil.which(argv[0], path=self.hook_env.get("PATH"))
            record = {"event": event, "command": spec["command"], "attempt": self.attempt}
            start = time.monotonic()
            if exe is None:
                record.update(missing=True, exit=None, stdout="", stderr="command not found")
            else:
                try:
                    proc = subprocess.run(
                        [exe, *argv[1:]],
                        input=json.dumps(body),
                        cwd=self.worktree,
                        env=self.hook_env,
                        capture_output=True,
                        text=True,
                        timeout=min(float(spec.get("timeout", 60)), self._remaining()),
                    )
                    record.update(exit=proc.returncode, stdout=proc.stdout, stderr=proc.stderr[-2000:])
                except subprocess.TimeoutExpired:
                    record.update(timed_out=True, exit=None, stdout="", stderr="")
            record["seconds"] = round(time.monotonic() - start, 4)
            record["response"] = interpret(event, record)
            record["stop_hook_active"] = fields.get("stop_hook_active")
            self.invocations.append(record)
            self._note("hook", event=event, exit=record["exit"], response=record["response"])
            results.append(record["response"])
        return results

    def agent(self, prompt: str) -> None:
        self.attempt += 1
        prompt_file = self.agent_dir / f"prompt-{self.attempt}.txt"
        prompt_file.write_text(prompt, encoding="utf-8")
        env = dict(self.agent_env, BENCH_PROMPT_FILE=str(prompt_file))
        self._note("agent-start", attempt=self.attempt, prompt=prompt)
        start = time.monotonic()
        try:
            proc = subprocess.run(
                [
                    sys.executable,
                    str(BENCH / "scripted_agent.py"),
                    "--task-dir", str(self.task.dir),
                    "--worktree", str(self.worktree),
                    "--arm", self.arm,
                    "--attempt", str(self.attempt),
                ],
                cwd=self.worktree,
                env=env,
                capture_output=True,
                text=True,
                timeout=self._remaining(),
            )
        except subprocess.TimeoutExpired as error:
            raise BudgetExhausted() from error
        finally:
            self.agent_seconds += time.monotonic() - start
        self._note("agent-end", attempt=self.attempt, exit=proc.returncode, stdout=proc.stdout[-2000:], stderr=proc.stderr[-2000:])
        if proc.returncode != 0:
            raise AgentFailed(proc.stderr.strip().splitlines()[-1] if proc.stderr.strip() else f"exit {proc.returncode}")
        self.last_message = proc.stdout.strip().splitlines()[-1] if proc.stdout.strip() else ""

    # -- the session ------------------------------------------------------------------

    def run(self) -> list[dict]:
        """Play the session; return its turn-by-turn trace. Raises BudgetExhausted or
        AgentFailed, after which `self.trace` still holds what happened so far."""
        self.trace: list[dict] = []
        for response in self.hook("SessionStart", source="startup"):
            if response["kind"] == "context":
                self._inject("SessionStart hook", response["text"])
        for index, (kind, prompt) in enumerate(self.turns, start=1):
            turn = {
                "index": index,
                "kind": kind,
                "start_fingerprint": grade.fingerprint(self.worktree),
                "stops": [],
                "continuations": 0,
                "continuation_limit_reached": False,
            }
            self.trace.append(turn)
            self._note("user-prompt", turn=index, prompt=prompt)
            if self.host == "claude-code":
                for response in self.hook("UserPromptSubmit", prompt=prompt):
                    if response["kind"] == "context":
                        self._inject("UserPromptSubmit hook", response["text"])
            self.agent(prompt)
            active = False
            while True:
                stop = {
                    "stop_hook_active": active,
                    "fingerprint": grade.fingerprint(self.worktree),
                    "unmet_files": grade.unmet_files(self.task, self.worktree),
                    "hooked": bool(self.hooks.get("Stop")),
                }
                stop["edits_before"] = stop["fingerprint"] != turn["start_fingerprint"]
                extra = {"turn_id": f"turn-{index}"} if self.host == "codex" else {}
                responses = self.hook(
                    "Stop", stop_hook_active=active, last_assistant_message=self.last_message, **extra
                )
                stop["responses"] = [r["kind"] for r in responses]
                stop["hook_failed"] = stop["hooked"] and (
                    not responses or any(r["kind"] in ("failed", "invalid") for r in responses)
                )
                blocks = [r for r in responses if r["kind"] == "block"]
                stop["decision"] = "block" if blocks else (
                    "advise" if any(r["kind"] == "advise" for r in responses) else ("nothing" if responses else None)
                )
                stop["block_reason"] = blocks[0]["text"] if blocks else None
                turn["stops"].append(stop)
                for response in responses:
                    if response["kind"] == "advise":
                        self._note("user-visible", text=response["text"])
                if not blocks:
                    break
                if turn["continuations"] >= CONTINUATION_LIMIT:
                    turn["continuation_limit_reached"] = True
                    break
                turn["continuations"] += 1
                self.continuations += 1
                self._inject("Stop hook: continue", blocks[0]["text"])
                self.agent(blocks[0]["text"])
                active = True
            turn["end_fingerprint"] = grade.fingerprint(self.worktree)
            turn["edits"] = turn["end_fingerprint"] != turn["start_fingerprint"]
        return self.trace
