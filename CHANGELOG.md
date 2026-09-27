# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `--summary` and `--report-file FILE`: what a run examined and what it did not. Every
  claim is accounted for — ignored, true, broken, skipped or ambiguous, with a stable
  reason code — and the run is `complete`, `incomplete` or `no-input`, so a quiet run
  over half-read documents or a shallow clone no longer looks like a clean one. The
  report is a versioned JSON envelope with the same findings as stdout and their
  fingerprints. See ADR-0019.
- `--sarif-file FILE`, so SARIF comes from the same analysis as any other format. The
  Action now scans once, writes the summary to the job summary, and exposes the report's
  path as its `report-file` output. See ADR-0021.
- `--fix-dry-run`, a unified diff of exactly what `--fix` would write. Both share one
  plan; a document that changed in between is not overwritten, and each file is replaced
  atomically. See ADR-0022.
- `stilltrue-hook`, adapters for Claude Code and Codex that run the check at session
  start and at each completion, and report findings newly observed since the session
  began. Advisory by default; blocking is opt-in and bounded. A Claude Code plugin and a
  Codex hooks file ship in `integrations/`. See ADR-0023 and `docs/agents.md`.
- Two pre-commit hooks, `stilltrue` and `stilltrue-system`, which check the whole
  repository on every commit.
- A quickstart and a repair recipe; a controlled-edit suite with a held-out family; the
  corpus pinned to exact commits with reviewed labels; and a measured performance budget.

- Colour in human output when stdout is a terminal, and never otherwise. `NO_COLOR` is
  honoured whatever its value. No machine format is ever coloured.
- `--baseline FILE` and `--write-baseline FILE`, so a repository with an existing
  backlog can adopt the tool without starting red. Findings are matched by fingerprint,
  never by line number, so reflowing a document does not un-suppress it.
- `--fix`, which rewrites a claim only where there is exactly one thing it could have
  meant, and prints every change. The exit code still reflects what was found.
- A TypeScript and JavaScript resolver — the second language behind the
  `LanguageResolver` trait, which existed from v1 to prove the seam.
- reStructuredText documents, for the Sphinx ecosystem. pytest previously yielded zero
  documents.
- Same-document anchors: `[setup](#setup)` is a claim. This found three broken
  table-of-contents links in fzf's README.
- `deno task` targets.
- Unused `<!-- stilltrue:ignore -->` markers, reported under `--strict`.

### Fixed

- A manifest that did not parse was read as having no targets, so every target it named
  was reported as rot. It is now skipped, and the run says it is incomplete.
- A git search that failed was indistinguishable from one that found nothing: under
  `--strict` it was a lie, and the cache stored it as proof of absence. A commit subject
  that was not UTF-8 lost its commit the same way. See ADR-0020.
- A cached positive reused after HEAD moved kept its old attribution even when newer
  history held a later match; a target restored and removed again was blamed on the
  first removal. The cache now searches the history added since, is versioned, and is
  replaced atomically, and a cached run is tested against an uncached one across branch
  switches, rebases, collected objects and concurrent runs.
- `--write-baseline` ran before `--require-history` was checked, so a baseline could be
  recorded from a run that could not see history.
- The definition index compiled its tree-sitter query once per source file. On the
  500-document, 5,000-file benchmark tree a warm run took 8.5 seconds; it takes 0.3.
- The README pinned the Action to `@main`, a branch that moves under every consumer. It
  now shows a placeholder until a release is verified from its published archives.
- "Rust 2024" — the edition — was read as a Rust version, compared with the toolchain
  pin, and reported as rot in a default run. An edition is no longer a version claim.
  See ADR-0024.
- The walk descended into nested repositories: a submodule, a nested clone, or an
  agent's worktree kept inside the checkout. Their documents were linted as this
  repository's, and their files counted as evidence here — enough to silence real rot.
  See ADR-0025.
- The history cache grew one directory per repository path forever, so every
  throwaway checkout left one behind; tests and a mutation sweep had left 107,818 on one
  machine. It now removes entries unused for thirty days and keeps at most 256, and
  `STILLTRUE_CACHE_DIR` relocates it. Tests keep theirs in `target/test-cache`.
- `stilltrue-hook` killed only the checker when a scan overran its budget, leaving the
  processes the checker had started — git, in a real scan — running after the adapter
  had answered. It now runs the checker in its own process group and kills the whole
  group, on a timeout and when the adapter itself is terminated.
- The diff `--fix-dry-run` prints writes a one-line range as `@@ -1 +1 @@`, as `diff -u`
  does, and is checked line for line against Python's difflib.

- `cargo <alias>` was reported broken in every repository, and Rust version pins were
  never read at all: both used `str::parse` where the TOML library means
  `toml::from_str`, and both swallowed the error.
- A name a repository imports, and a name its language provides, now count as defined.
  `Path()` and `print()` were being reported as rot.
- `[ignore] symbols` matched the raw claim text, so it could not be used the way the
  documentation showed. `symbols = ["fold_events"]` did not silence `fold_events()`,
  and the documented example of a bare name could never match anything, because a
  symbol claim always carries a `()` or a dot. It now matches the symbol's name.
- `self` and `cls` were offered as suggestions on a broken Python method. They are
  parameters on every method in a repository, so they belong in the definition index
  and never in `did you mean`.
- Four classes of false positive found by re-running the validation corpus, which went
  from 11 findings to 6 and from nine silent repositories to twelve, with all five true
  positives untouched:
  - A CamelCase namespace this repository never defines is no longer assumed to be a
    class it can read. `Object.keys`, `JSON.parse` and `Math.floor` were judged against
    a Python index. See ADR-0013.
  - A module that serves its names from a lazy `__getattr__` computes its attributes,
    so nothing static can read them. `click.get_text_stream` is one.
  - An environment variable whose family the code assembles is no longer looked up.
    poetry builds every one of its names, so the literal appears nowhere. See ADR-0018.
  - A builtin of any language this tool reads is missing from nothing, whether or not
    that resolver has files here — a document never says what language a symbol is in.
    `fetch()` in a Python project's JavaScript page was reported as rot.
- A version written as a family — `Python 2.x`, `Node 18.*` — cannot contradict a pin,
  by the same reasoning that makes a glob no claim about a path.
- `stilltrue.toml` counted as evidence for an environment variable, so naming one in
  `[ignore]` made it resolve as read. The configuration satisfied its own claims, the
  way a document did before the prose exclusion, and silenced findings by the wrong
  mechanism. The history needle carries the same exclusion.
- A pin file naming a channel rather than a version was read as a pin, so every Rust
  version stated in a README contradicted `channel = "stable"` — the commonest
  `rust-toolchain.toml` there is — and was reported as rot, blamed on the commit that
  created the file. `lts/*`, `lts/iron`, `nightly-2026-01-01`, `system` and
  `pypy3.10-7.3.15` all did the same. A channel names whatever that stream points at
  today, so prose stating a number cannot contradict one. See ADR-0003.
- A suggestion for a broken link was spelled from the repository root, when a link is
  relative to its own document. A document in `docs/` was offered
  `docs/new/x.md` for a file it has to reach as `new/x.md`.
- `--fix` wrote text that did not resolve for every claim type except a command, which
  was the only one it was tested on:
  - A version lost its tool name: `Needs Node 20` became `Needs 22.3.0`.
  - An anchor replaced the whole destination: `[x](guide.md#install)` became
    `[x](setup)`, a link to a file that does not exist.
  - A link took the repository-relative spelling above, so it stayed broken — and,
    having no history any more, fell out of a default run. A reported finding became a
    silent one, which is the worst direction this tool can move in.
  - The rewrite log went to stdout, after the report, so `--format json --fix` and
    `--format sarif --fix` emitted two documents and parsed as neither. It now goes to
    stderr, like the tally beside it.

### Changed

- A `Symbol` claim needs a call or a namespace. The bare-`snake_case` rule produced six
  false positives across the validation corpus and no true ones.
- `tests/product/run.sh` drives every shipped feature through the release binary
  against repositories with real git history — 162 assertions over several fixtures,
  running on Linux and macOS in CI. It is how the `[ignore]` and suggestion fixes above
  were found.

## [0.1.0] - 2026-09-18

First release. A linter for the instructions you give your coding agent, and for your
README: it binds claims made in Markdown to the real repository and fails CI when the
code moved and the document didn't.

### Added

- Four pipeline stages — `extract`, `resolve`, `gate`, `report` — with `gate` as its own
  stage so that "is this claim false?" and "is this worth a human's attention?" stay
  separate questions.
- Six claim types: paths, commands, environment variables, pinned versions, links, and
  Python symbols. Commands resolve `make` targets, `just` recipes, `npm`/`pnpm`/`yarn`
  scripts, and `cargo` aliases against the nearest anchoring manifest.
- Three gating tiers. Tier A (rot: broken, and git history proves it once resolved) is
  reported by default and names the commit that likely broke it. Tier B (lie) requires
  `--strict`. Tier C (ambiguous) is never reported.
- Broken link anchors are reported, not just missing link targets: retitling a heading
  is the commonest documentation refactor there is. Explicit `{#id}` anchors, mkdocs'
  `[](){#id}` idiom, and emphasis inside a heading are all understood.
- Documents are read at any depth, so a monorepo package's own README is linted against
  its own manifest.
- Human, JSON, SARIF 2.1.0, and GitHub workflow-command output, all obeying the same
  gate. Columns are Unicode code points, SARIF URIs are percent-encoded, and result
  fingerprints survive a documentation reflow without colliding between two occurrences
  of one claim.
- A composite GitHub Action that installs a prebuilt binary, repairs a shallow clone,
  and annotates by default. It carries no runtime: the binary renders the annotations
  itself, so the escaping workflow commands require happens once, in Rust. `sarif-file`
  writes SARIF for a later upload step.
- `stilltrue.toml` configuration, block-anchored `<!-- stilltrue:ignore -->`
  suppressions, and a cache outside the repository.
- Prebuilt binaries for Linux and macOS on both architectures, plus Windows x64.

### Notes

Precision is the product, so it is measured rather than asserted. Validating against a
corpus of fifteen third-party repositories took the default run from 317 findings to 11
and nine of the fifteen to silence. Every rule that came out of it is recorded in
`docs/design.md`.

Documentation that describes the reader's files rather than its own — a bare
`requirements.txt` — is deliberately not judged. Environment variables read through a
dependency or assembled from config keys, symbols reached through a lazy `__getattr__`,
commands in untagged fences, and bare directory references are known blind spots.
`tests/corpus/run.sh` reproduces the measurement.

[0.1.0]: https://github.com/uk0064/stilltrue/releases/tag/v0.1.0
