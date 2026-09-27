# Conventions

How work is done in this repository, for people and for coding agents alike. The
architecture and every decision behind it are in [the design document](docs/design.md),
and the vocabulary is in [CONTEXT.md](CONTEXT.md). Read both before proposing a change.

## Decisions

- The design document is the source of truth. To change a decision, change the document
  first, in the same change as the code, and say so. Never diverge silently in code.
- A new decision takes the next free ADR number and goes in the section of the design
  document it belongs to, with its context, the decision and its consequences, plus a
  row in the index. Cite it by number from the code, as the comments already do.
- A new classification rule — anything that makes a broken claim ambiguous — is always
  a decision, and always gets an ADR.
- There is no model anywhere in the pipeline. If a check needs one, it does not belong
  here.

## Code

- Rust, on the toolchain pinned in `rust-toolchain.toml`. CI runs
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`;
  run all three before you push.
- Errors never fail the build. Unparseable input, missing manifests and git failures
  warn to stderr and skip that unit. Exit `2` is reserved for unrecoverable faults.
- `resolve` never invokes git. Git is touched only in `gate`, so a repository with no
  broken claims spawns no git processes at all. Git is a subprocess behind a trait.
- History needles are POSIX ERE, not PCRE (ADR-0010). A language resolver's definition
  query and its history needle must describe the same set of names.
- Comments say why, in plain sentences. Cite a decision by its number rather than
  restating it.
- Python in `bench/` and `tests/` uses the standard library only.

## Tests

- Fixtures are real git repositories built by `tests/fixtures/<name>/build.sh`,
  because history is part of the logic. Snapshots use `insta`.
- Every gating rule ships two fixtures: the true positive it must report, and the
  near-miss it must stay silent on. The near-miss fixtures are the precision regression
  suite — treat deleting one as a serious change.
- Make each new case the only thing standing between the claim and the wrong answer. A
  case whose input trips several rules at once stays green when any one of them breaks.
- Tests keep their history caches in `target/test-cache` through `.cargo/config.toml`;
  a test that writes to the real cache directory is a bug.
- `cargo test` is not the whole story. `tests/product/run.sh` drives the release binary
  against repositories with real history, `tests/fuzz.rs` throws pathological input at
  both extractors, and `tests/corpus/pinned.py` re-runs the third-party corpus at fixed
  commits. Cite a precision number only if one of these produced it.

## Heavy verification

Run it gently, and clean up after it.

- Run `cargo mutants` with one job, scoped with `--in-diff` and `--file` to what
  changed, passing only the test binaries that exercise those files after `--`. Each
  extra job builds a full copy of the crate, and parallel sweeps have exhausted the
  maintainer's memory more than once.
- When a sweep, a corpus run or a live host test ends or is interrupted, stop every
  process it started — not just the task that launched it — and remove leftover copies
  under the temporary directory.

## Documents

- This repository passes its own lint with no suppressions: `cargo run` reports
  nothing, and CI fails if it does. Every path, command and version written in a code
  span has to be real. For an example that is not, use an untagged fence or an
  angle-bracket placeholder such as `<name>`.
- Setup examples pin a release verified from its published archives, or show the
  `vX.Y.Z` placeholder — never a branch. `tests/docs.rs` enforces this.
- Wrap Markdown at about 90 columns.
- Every user-visible change gets an entry under Unreleased in `CHANGELOG.md`, in the
  Keep a Changelog format.

## Pull requests and commits

- Every change reaches `main` through a pull request; `main` accepts nothing else. Every
  CI job must pass, history stays linear, and pull requests are squash-merged.
- The maintainer approves every pull request before it merges, Dependabot's included.
- Coding agents open pull requests and stop there. Approving and merging belong to the
  maintainer, even when the agent is working with credentials that would allow it.
- A commit subject is one plain imperative sentence saying what the change does, without
  a type prefix or a trailing period: "Bound the history cache, and keep tests out of
  the real one". A pull request title follows the same rule, because a squash merge can
  use it as the subject.
- The body, when there is one, says why.
