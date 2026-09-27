# Agent-task benchmark

Does running stilltrue inside a coding agent's session make the agent's work better?
This directory is the harness for answering that: real repositories with history,
labelled documentation obligations, four experimental arms, deterministic grading, and a
clustered analysis with frozen promotion gates.

**No paid or live run has been executed, and no effectiveness claim is made.** Every
result this harness has produced so far comes from a *scripted agent* — a fixed,
model-free policy that exists to prove the pipeline works end to end. Its outcomes say
nothing about any real agent, and every report it writes says so.

## Three evidence layers

The evidence is kept in three layers, each with its own report. A strong result in one
never stands in for another.

| Layer | Question | Where |
|---|---|---|
| Scanner correctness | Does the checker report true findings and stay silent on near-misses? | `cargo test`, the fixtures under `tests/fixtures/`, the corpus in `tests/corpus/run.sh` |
| Adapter reliability | Do the hooks keep each host's contract? | `tests/hook.rs`, `tests/hook_replay.rs`, and the live smoke test `tests/product/hooks-live.sh` |
| Agent outcomes | Does an agent with the hooks finish tasks better? | this directory |

The benchmark may orchestrate models; the checker and the truth labels stay
deterministic. Grading never runs stilltrue: its verdict is what is under evaluation.

## What is here

| File | Role |
|---|---|
| `bench/tasks/<id>/task.toml` | One case: prompt, required behaviour, documentation obligations, code and doc checks, known limitations, scope |
| `bench/tasks/<id>/scripted.py` | That case's scripted solution, `solve(worktree, arm, attempt)` |
| `bench/repos/<family>/build.sh` | Builds the case's repository as a real git history, with fixed commit dates |
| `bench/frozen.toml` | The frozen split (whole families held out) and the gate thresholds |
| `bench/manifest.py` | Validates every manifest and the suite; unknown or missing fields are errors |
| `bench/arms.py` | What each arm installs, per host |
| `bench/run.py` | Runs task × arm × repeat, isolated, in seeded random arm order |
| `bench/hostsim.py` | Plays a host's side of the hook protocol for the scripted agent |
| `bench/shim.py` | Records every hook and checker call on the way to the real binary |
| `bench/grade.py` | Deterministic grading and forbidden-repair detection |
| `bench/analyze.py` | Success rates, paired differences, clustered intervals, blocking, gates |
| `bench/live.py` | The live Claude Code session driver — never run so far |
| `bench/test_bench.py` | The harness's own tests |

## Tasks

Fifteen cases in four repository families. `python3 bench/manifest.py` validates them
and prints the split.

| Family | Split | Repository | Cases |
|---|---|---|---|
| pyapp | development | Python package driven by a Makefile | a renamed target, a renamed function, a stale test command at entry, unrelated pre-existing rot, a clean control |
| tsapp | development | TypeScript package with package.json scripts | a renamed script, a renamed export, a near-miss script change, a semantic change the checker cannot see |
| polyglot | development | Python beside Go, so symbol checking abstains | a renamed function (unsupported analysis), a renamed environment variable, a Go-only clean control |
| docsite | holdout | Documentation-heavy: anchored links, script paths, a changelog | a moved script, a stale anchor at entry, a near-miss deletion |

Every category of case is covered: stale at entry, code that breaks an
untouched document, clean controls, near-misses, pre-existing unrelated rot,
unsupported analysis, and semantic errors out of scope. Cases the checker cannot detect
set `in_scope = false` and are reported separately. Not every case favours stilltrue:
in several the checker is expected to add nothing, and in two it cannot help at all.

The split is frozen in `bench/frozen.toml`: docsite is held out whole, and a task whose
split disagrees with its family's entry fails validation. Running held-out tasks needs
`--holdout-use exploratory` or `--holdout-use confirmatory`, and the choice is recorded.

## Arms

| Arm | What the session gets |
|---|---|
| A | The normal harness. No hooks, no instruction, and no stilltrue on the agent's PATH |
| B | stilltrue on PATH, a standing instruction to run it before finishing, and on Claude Code the repair skill — no hooks |
| C | The adapters' hooks at session start and completion, advisory |
| D | The same hooks with bounded blocking: at most one continuation per user turn |

On Claude Code, C and D load a per-run copy of `integrations/claude-code/` with
`--plugin-dir`, and D sets `STILLTRUE_HOOK_MODE` to `block`; B loads the same plugin
without its hooks. On Codex, C and D write `integrations/codex/hooks.json` into the
worktree's Codex directory (with `--block` for D), and B puts the instruction in the
run's own Codex home. `python3 bench/arms.py` prints exactly what each arm installs.

Every session has two user turns: the task prompt, then a fixed review prompt that is
identical in every arm. The second turn matters because the Claude Code adapter delivers
completion advice as context at the *next* prompt; with a single turn, arm C could never
act on it. Codex has no such channel, so there its completion advice reaches only the
user — the harness reproduces that rather than papering over it.

## The scripted agent

The scripted agent runs as its own process in the worktree, like a live agent, and acts
only on what it can observe: the prompt, a context file holding whatever the host
injected, and whether stilltrue is on its PATH. The policy in `bench/scripted_kit.py`
never looks at the arm. It always makes the task's code change; it updates documents
when its case says it remembers to, when a stilltrue finding in its context names a
document the repair touches, or — told to run the checker and able to — when its own run
names one. Some cases model an agent that forgets documents, some one that does not.

For the scripted agent, `bench/hostsim.py` plays the host: it sends the payload shapes
the adapter's replay tests use (`tests/hook/payloads/`), runs each hook command from the
arm's installed configuration against the real `stilltrue-hook` and `stilltrue`, feeds
context and block reasons to the agent, gives one more attempt on a block, and sets
`stop_hook_active` on the stop that follows it.

## Run the scripted validation

It costs nothing and needs no model: only git, Python 3.11 or newer, and the two
binaries.

```bash
cargo build
python3 -m unittest discover -s bench -p 'test_*.py'
python3 bench/run.py --agent scripted --host claude-code --arms A,B,C,D \
  --repeats 3 --split development --seed 1 --out /tmp/stilltrue-bench
```

The tests include an end-to-end run of the development split through arms A, C and D,
which takes a few minutes with a debug build; they skip it, saying why, when the
binaries are missing. The runner finds the binaries in Cargo's release or debug
output, and accepts `--stilltrue-bin-dir` for others.

Each run gets its own repository built from its build script, and its own HOME, cache,
temporary and hook-state directories, so no cache, transcript or report crosses from
one run or arm to another. A run directory is deleted after grading unless
`--keep-worktrees` is given; its evidence is kept.

## What a run writes

| In the output directory | Contents |
|---|---|
| manifest.json | Schema version, stilltrue revision and binary digests, host and model (or "scripted"), machine, platform, timestamps, seed, recorded arm order, spending limit or null, split, task digests, the frozen thresholds in force |
| outcomes.jsonl | One line per run: status (completed, task-failure, budget-exhausted, infrastructure-failure, interrupted), durations, checker time, hook calls, continuations, the turn-by-turn trace, grading, and tokens and cost — null when unavailable, never 0 |
| report.json and summary.md | The analysis, per host |
| runs/, one directory per run | The agent's context, prompts, transcript and hook calls, and the adapter's own reports |

An infrastructure failure is recorded and retried once by default (`--retries`); the
retry is a new outcome whose `rerun_of` names the original, and the analysis lists both.

## Grading

`bench/grade.py` grades code and documents separately. Code success means every
`code_checks` command exits 0 in the final worktree. Documentation is correct when every
obligation holds — required strings present, stale ones gone — and every `doc_checks`
command passes. Success needs both, and no forbidden repair: a suppression marker
added, an instruction deleted rather than corrected, stilltrue's configuration changed,
or the hooks disabled or their installed files altered. Each of those fails the run and
is reported as a violation.

## Analysis and gates

`bench/analyze.py` reports each host on its own and never pools them.

- Success per arm with denominators, split into in-scope and out-of-scope cases.
- Paired per-task differences: D−A is primary, C−A and B−A are mechanism comparisons.
  A task's repeats are averaged and stay together; the 95% interval resamples families,
  then tasks within them.
- Unnecessary blocks. An eligible boundary is the first completion check of a user turn
  in which the repository changed, taken from the harness's edit trace rather than the
  adapter's decision; continuations after a block are repair verification and are not
  counted as boundaries. A block is necessary only if a document it names had an unmet
  obligation at that moment. The upper bound is the larger of an exact binomial bound
  and a family-clustered bootstrap bound. The fraction of blocking decisions judged
  unnecessary, event counts, unresolved valid blocks and missed hook invocations are
  reported beside it.
- The promotion gates, per host: every false-block regression case (clean controls,
  near-misses, pre-existing rot) passes without a block; D−A is at least ten points with
  an interval above zero; the unnecessary-block bound is under 5%; and on clean controls
  p95 duration and mean tokens rise no more than 10% over A. Missing tokens make that
  last gate "unavailable, not passed".

Anything below the frozen thresholds — four families, twelve paired tasks, three repeats
per task per arm — is reported as inconclusive rather than estimated. With three
development families and one held-out family, this task set can never be conclusive on
one split: it validates the harness, and a confirmatory run needs more families.

## Live runs

A live run spends money on the operator's model access, and `bench/run.py` refuses to
start one unless it is given both `--authorized` and `--spending-limit-usd` with a
positive amount. The refusal happens before any directory is created or any host is
started, and the tests hold it to that. The limit is recorded in the run manifest.

For Claude Code the limit is divided across every planned invocation and passed to each
as its own budget, and the runner stops scheduling once reported spend reaches it. Pin
the model with `--model`; otherwise the manifest records it as unpinned. Credentials
reach the otherwise isolated session only through the API-key and OAuth-token variables
named in `bench/live.py`.

Live Codex runs are refused: Codex reports no billed cost, so a spending limit could not
be enforced. Its arms still run under the scripted agent.

The live driver has never been executed. Treat its first authorized run as a pilot of
the harness itself, and inspect the outcomes before trusting them. The cheapest
disconfirming run is single-host A against C.

## Reproducing a run

Build scripts use fixed commit dates, so the same script builds the same commit ids on
any machine; each outcome records its starting commit. The manifest records the seed,
the arm order it produced, a digest of every task's files and build script, the frozen
thresholds and the binaries' digests. `python3 bench/analyze.py` re-derives the report
from a run directory, with the same bootstrap seed unless told otherwise.

## Known limitations

- The scripted agent's behaviour is chosen, not learned. It proves the pipeline — arms,
  hooks, agent, grading, analysis — and nothing about agents.
- Necessity of a block is judged against the case's labelled obligations; there is no
  blinded human review or adjudication step yet.
- Human interventions, failed commands attributable to stale instructions, and false
  alerts outside blocking are not recorded.
- Every run starts with a cold cache; warm-cache experiments, a release-to-release
  comparison and a competing checker are not implemented.
- Transcripts stay on the machine that ran them; outcomes carry their SHA-256.
