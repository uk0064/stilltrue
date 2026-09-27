# stilltrue

A linter for the instructions you give your coding agent — and for your README.
It binds claims made in Markdown to the real repository and fails CI when the code
moved and the document didn't.

## Status

**Implemented and validated against real repositories.** All four stages ship —
`extract`, `resolve`, `gate`, `report` — covering paths, commands, env vars, versions,
links, and Python and TypeScript symbols, in Markdown and reStructuredText, with the
fixture matrix and a composite GitHub Action. `--baseline` for adoption, `--fix` for
claims that can only mean one thing. `cargo test` is green and `cargo run` on this
repository reports nothing, which is the self-referential rule below turned into a test
rather than a promise.

**Precision is measured, not asserted.** A corpus of fifteen third-party repositories
took the default run from 317 findings to 6, twelve of the fifteen to silence, and the
repository that once took over three minutes — poetry, 1,022 files — to half a second
warm. The slowest of the fifteen is now black at 1.0s, and all fifteen together take
under 4 seconds (measured 2026-09-26, macOS arm64, warm cache, best of three, at the
commits pinned in `tests/corpus/pinned.tsv`). Most of the last halving came from
compiling each tree-sitter query once per grammar instead of once per source file,
found by profiling the 500-document benchmark tree that `tests/perf/measure.py` builds.

Those twelve are silence rather than breakage: the runner records each repository's exit
status and stderr and reports degraded and failed scans separately, because a crashed
scan also prints no findings. The last run had none of either. Five of those six
findings are confirmed true positives; the sixth is an entry-point group name that is
spelled exactly like an attribute. Every fix it produced is in `docs/design.md`. Re-run it
before believing any precision claim: this repository was written to satisfy the tool,
so it cannot falsify it. Nor can it check the timings above — they are prose about
measurement, which is outside what this tool reads.

**Three things find what `cargo test` does not.** `cargo mutants` found that
`str::parse::<toml::Value>` rejects what `toml::from_str` accepts — every `cargo <alias>`
was reported broken and no Rust version pin had ever been read. `tests/product/run.sh`
drives every shipped feature through the release binary, against repositories with real
history, and found that `[ignore] symbols` could not be used the way the README
documents it. `tests/fuzz.rs` throws pathological input at both extractors. Run all
three before believing the suite constrains the code.

**A test that passes is not a test that holds.** Most of the gaps mutation testing found
here were tests whose inputs tripped several rules at once, so breaking any one of them
left the test green: `<name>` has both angle brackets, every version test used a single
space, and every suggestion test used a four-character name that never reached the
long-name branch. When adding a case, make it the only thing standing between the claim
and the wrong answer.

`tests/corpus/run.sh` is that corpus. It clones the fifteen repositories named in
`tests/corpus/repositories.txt` and lints each one. It is not a test — it fetches about
a gigabyte and the repositories move under it, so the counts drift — but every precision
number in this file and in the README came from it, and can be checked.
`tests/corpus/pinned.py` does the same at fixed commits, with reviewed labels, and is
the one to cite.

**The adoption work is built, and says what it has not shown.** Every run can report
what it examined (`--summary`, `--report-file`), and a quiet run over unread documents
or a shallow clone says `incomplete` rather than passing for clean. `stilltrue-hook`
runs the check at an agent's session start and completion, for Claude Code and Codex;
it must always exit 0, because a Claude Code Stop hook reads 2 as "keep going". Nothing
here shows that the adapters improve an agent's work: `bench/` is the harness that would,
and no live run has been made. Say what a number does not show, and keep this file and
the README honest when one changes.

`docs/design.md` is the source of truth — the architecture, and every decision behind
it — and `CONTEXT.md` is the vocabulary. Read both before proposing
anything. If you want to change a decision, change the design document first and say
so — don't diverge silently in code.

## The thesis

**Precision is the product.** Every doc linter that has ever been uninstalled was
uninstalled for false positives, not for missing something. The `gate` stage exists as
its own pipeline stage specifically so that "is this claim false?" and "is this worth
a human's attention?" stay separate questions. Competitors fold them together, which is
why they are noisy.

When in doubt, **stay silent**. A missed finding costs nothing. A false finding costs
the whole tool.

## Decisions already made — do not relitigate

- **Rust.** Single static binary, no runtime in CI. This is what won the linter era
  (ruff, biome, oxc).
- **tree-sitter on both sides** — `tree-sitter-md` to extract claims with exact spans,
  `tree-sitter-<lang>` to resolve them.
- **Polyglot architecture.** Python and TypeScript ship behind the `LanguageResolver`
  trait, which now carries the file extensions, the grammar, the definition query and
  the history needle for each. **The query and the needle must describe the same set** —
  when the index was the narrower of the pair, every name in the gap resolved Broken and
  was then promoted to Tier A by its own history. Adding a language means adding one
  impl and its pair of tests.
- **Three gating tiers.** Tier A (rot: provably broken *and* git history proves it once
  resolved) is on by default. Tier B (lie: never existed) needs `--strict`. Tier C
  (ambiguous) is never reported and no flag reaches it — but classification rules do,
  and adding one is a design change, written into `docs/design.md` with its reasoning
  before the code, the way ADR-0011 added one.
- **Git as a subprocess** behind a trait, not `gix`. Git is in every CI image.

## Non-goals — say no to these

- **No LLM anywhere in the pipeline.** This is the constraint most likely to be
  violated by accident. The tool is static analysis. If a check needs a model, it
  doesn't belong in v1.
- No prose or style linting — that's `vale`.
- No OpenAPI conformance — that's DriftLinter.
- No external URL checking — that's `lychee`.
- Autofix rewrites only what it can enumerate — one candidate, never a choice between
  two. See ADR-0015 in `docs/design.md`.
- No *third* language resolver in v1. Python and TypeScript ship; shipping the second
  is what proved the seam, and what caught the query/needle asymmetry a single resolver
  hid. A third is cheap to add and is still a new way to be wrong about a symbol.

## Conventions

How work is done here — verification, errors, tests, documents and commits — is in
@CONVENTIONS.md

## Self-referential rule

This project lints `CLAUDE.md` files. Once the tool runs, this file must pass its own
lint, with no suppressions. Every path, command, and version claimed here has to be
real. If that becomes annoying, that's the tool telling you something true about the
product.
