# stilltrue with a coding agent

Two thin adapters run the same checker at the moments an agent can act on what it
says: when a session starts, and each time the agent finishes a turn. They ship as one
binary, `stilltrue-hook`, installed beside `stilltrue`, and a hook configuration for
each host. No server, no model, no account: the adapter runs the checker and passes on
what it found.

## What happens

| Boundary | What the adapter does | Default |
|---|---|---|
| Session start or resume | Scans every configured document, remembers the findings as this session's starting point, and gives the agent compact evidence about instructions that may be stale | Advisory. The agent is told the findings predate the session and to repair them only if the task calls for it |
| Completion, after the repository changed | Rescans every configured document — including ones nobody touched, because a code change is what breaks them — and compares with the starting point. A finding present now and absent then is *newly observed* | Advisory: a warning to you, and context for the agent at your next prompt |
| CI | Independent. It scans the final repository with its own gate and trusts nothing a hook did | Unchanged |

A completion check is skipped only when the working tree's content is identical to the
last complete check — every file the checker would read, untracked ones and ignore rules
included, plus HEAD. An edit event is not enough to decide that, because a shell command
can change files without one.

"Newly observed" never means "introduced by the agent": someone else may have pushed,
or a pull may have landed. The adapter says newly observed, and nothing stronger.

The adapter never runs `--fix`, never runs a command a document names, and never edits
a file. It reports, and the agent repairs with its ordinary permissions.

## Install

Both hosts run `stilltrue-hook`, which runs `stilltrue`; install both on PATH first —
see [the quickstart](quickstart.md). `cargo install` installs the two together.

### Claude Code

The plugin in `integrations/claude-code/` registers the hooks and a skill describing
how to repair a finding. This repository is also a plugin marketplace, so it installs
once for every project:

```bash
claude plugin marketplace add uk0064/stilltrue
claude plugin install stilltrue@stilltrue
```

Inside a session, `/plugin marketplace add uk0064/stilltrue` does the same. For a local
checkout, without installing anything:

```bash
claude --plugin-dir integrations/claude-code
```

Or copy the three entries from `integrations/claude-code/hooks/hooks.json` into the
`hooks` section of your `.claude/settings.json`. Claude Code asks you to trust a folder
before its settings run hooks; plugins you install are reviewed the same way.

To turn it off, disable the plugin with `claude plugin disable stilltrue`, or remove the
entries from your settings; `"disableAllHooks": true` turns off every hook at once. To
remove it, uninstall the plugin and delete the session state directory below.

### Codex

Codex reads hooks from `~/.codex/hooks.json` or from a project's `.codex/hooks.json`.
Merge the entries in `integrations/codex/hooks.json` into either. Codex records trust
against each hook's contents, so new hooks are skipped until you review and trust them
with its `/hooks` command, and project-level hooks load only in a trusted project.

To turn every hook off, set `hooks = false` under `[features]` in `~/.codex/config.toml`;
to remove this one, delete its three entries. Codex is **untested live** so far — see
the matrix below.

## Opt-in blocking

`stilltrue-hook claude-code --block` (or `STILLTRUE_HOOK_MODE=block` in the hook's
environment) lets a completion check ask the agent for one more pass. It does so only
when **all** of these hold, and otherwise advises:

- the finding is newly observed rot — never a lie, an ambiguous claim, or anything the
  session started with;
- the entry scan and the completion scan were both complete, and nothing — tool,
  configuration, branch — changed between them;
- no file changed while the completion scan ran;
- this user turn has not already had its continuation, the host does not report that
  this stop is itself a continuation, and the user did not interrupt the turn.

After the one continuation, the next completion is a verification run: it reports the
finding resolved, still unresolved, or unverified, and the turn ends. CI keeps its own
gate either way. The agent is told plainly not to delete instructions, add suppressions
or disable the check to get a pass.

Blocking is not recommended outside benchmark runs yet: whether it improves an agent's
work enough to justify interrupting it is exactly what the paired agent-task benchmark
is for, and no such result exists.

## Limits

- Each scan has five seconds (`--timeout SECONDS` to change it). One that overruns is
  abandoned, with everything it started, and reported as unverified; it never blocks.
  What it had already learned from history is kept, so the next scan has less to do.
- On a large repository the first scan with an empty cache searches its whole history,
  and a single search can take longer than the budget. On codex, 8,650 files, every
  hook scan timed out until one ordinary `stilltrue` run — nine seconds — filled the
  cache the adapter reads; after that, session start took 1.1 seconds and an unchanged
  completion check 0.3. Run it once when you install the adapter on a big repository.
- What the agent is told is at most 4 KiB. A longer list is cut, says how many findings
  it dropped, and points to the full report on disk. A shortened list is not evidence
  the rest were dealt with.
- Claim text and commit subjects are repository content, so they are quoted between
  markers as data, one line each, with anything that could end the quotation removed.
- History is never deepened over the network inside a hook. In a shallow clone the scan
  is incomplete, says so, and cannot block.
- The adapter always exits 0 and answers only in JSON a host documents. On Claude Code a
  Stop hook's exit status of 2 means "keep going", so no failure of the adapter or the
  checker — a bad flag, a missing binary, a crash — is allowed to surface as one.

## Session state

Kept outside the repository, in the platform cache directory under
`stilltrue/sessions/`, or wherever `STILLTRUE_HOOK_STATE` points: one file per
worktree and host session, so simultaneous sessions and worktrees never share a starting
point. Each file records the starting findings and whether that scan was complete, the
tool and configuration they were computed under, the branch, and the latest scan. State
unused for seven days is removed, and at most 256 sessions are kept.

Resuming a session keeps its starting point, so a finding newly observed before a
restart is still newly observed after it. A new session, a cleared one, or a change of
tool, configuration or branch starts a new comparison: the reason is recorded, whatever
was still unresolved is carried forward as advice, and the new starting point is only
ever taken from a complete scan.

The latest full report per session is kept beside the state under `reports/`, and every
message that was shortened names it.

## Compatibility

| Host | Version tested | Platform | Events | Live run | Trust and setup |
|---|---|---|---|---|---|
| Claude Code | 2.1.283 | macOS arm64 | SessionStart, UserPromptSubmit, Stop | Passed 2026-09-26, advisory: the manifest validated, all three hooks ran, the entry scan found the documented rot and the agent's reply named it, nothing was written into the repository, and with `disableAllHooks` no hook ran | Folder trust; `--plugin-dir` or settings |
| Codex CLI | — | — | SessionStart, Stop, Interrupt | **Untested**: not installed where this was built | Per-hook trust in `/hooks`; trusted project for `.codex/hooks.json` |

Both hosts are covered by replay tests of every payload shape their documentation gives,
and Claude Code by payloads recorded from the live run above (`tests/hook/payloads/`).
Replay is necessary and not sufficient: a host is marked tested only after
`tests/product/hooks-live.sh` passes against it, and a host that could not be run is
recorded as untested, never as passed. That script runs a real agent on your own model
access, with a spending cap, `STILLTRUE_LIVE_BUDGET_USD`, defaulting to a quarter.

Linux and Windows are untested for both hosts.
