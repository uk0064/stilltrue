# Hook payloads for replay

One file per event, per host, in the shape each host's hook documentation gives —
extra fields included, because both hosts add fields release by release and the adapter
must ignore what it does not use. `tests/hook_replay.rs` feeds each through the adapter
and holds the answer to the output shapes that host accepts.

Files named `recorded-*.json` were captured from a live host with
`STILLTRUE_HOOK_RECORD`, with the session id, prompt id and paths replaced. The Claude
Code ones come from Claude Code 2.1.283 on macOS arm64 (2026-09-26), and they differ from
its documentation in both directions: SessionStart arrived without `model` or
`permission_mode`, and UserPromptSubmit and Stop carried an undocumented `prompt_id`. The rest were written
from the documentation, and the Codex `Interrupt` payload in particular is inferred:
its fields were not confirmed from a primary source. Replay is necessary and not
sufficient — the compatibility matrix in docs/agents.md records which hosts were also
run live.
