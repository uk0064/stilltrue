# stilltrue

A linter for the instructions you give your coding agent — and for your README.

`stilltrue` binds claims made in Markdown to the real repository and fails CI when the
code moved and the document didn't. It is static analysis: there is no model anywhere
in the pipeline.

```
CLAUDE.md:14:8  rot  command `make demo` has no target in Makefile
                     likely broke in a3f21c9 "split demo into seed+serve" · 4 months ago
                     did you mean: make seed, make serve?
```

## Why

Nobody lints agent configuration artifacts. In 2026 those files are not documentation —
they are instructions a machine executes against. When they rot, the agent silently
takes wrong actions and burns tokens, and no signal is emitted. Ordinary READMEs are
covered by the identical checks, so they come for free.

## Precision is the product

Every doc linter that has ever been uninstalled was uninstalled for false positives.
So "is this claim false?" and "is this worth a human's attention?" are kept as separate
questions, answered by separate stages:

- **rot** — broken, *and* git history proves it was once true. The code moved and the
  document didn't. Reported by default.
- **lie** — broken, with no evidence it was ever true. A different bug with a different
  cause. Needs `--strict`.
- **ambiguous** — broken, but we cannot honestly say whose repository the claim was
  about, or our reading of this one is too incomplete to judge. Never reported, and no
  flag reaches it.

When in doubt, stay silent. A missed finding costs nothing. A false finding costs the
whole tool.

This is measured, not asserted. Against fifteen third-party repositories the default run
produces 6 findings, with twelve of the fifteen clean — down from 317 before that corpus
was run. Five of the six are confirmed true positives. Every rule it produced is recorded
in [the design document](docs/design.md). It is fast enough to run on every commit: all
fifteen scan in under four seconds together, the largest in about one, with a warm
cache on a laptop.

Clean, not merely quiet: the runner records each repository's exit status and stderr and
reports degraded and failed scans separately, because a scan that crashes also prints no
findings. The last run had none of either. Until that was fixed the count could not have
told you the difference.

## What it checks

| Claim | True when |
|---|---|
| Path | the file exists, relative to the document or the repository root |
| Command | the runner's target exists in the nearest ancestor manifest |
| EnvVar | the name is read somewhere in code or config — never in prose |
| Version | the version matches an exact pin, such as .nvmrc or rust-toolchain.toml |
| Link | the relative target exists and any anchor matches a real heading |
| Symbol | a definition exists, in repositories every resolver supports |

Symbols are read by Python and TypeScript resolvers; a repository containing a language
neither can read gets no symbol checking at all, deliberately. A symbol claim needs a
call or a namespace — `fold_events()`, `llm.get_model` — because a bare word with an
underscore in it is a word, not a reference to code.

Claims are read from inline code spans, link destinations, and shell fences. Nothing
else inside a fenced block is ever a claim, because fences are where placeholders and
other repositories' commands live. Markdown and reStructuredText both, so a Sphinx
project is linted like any other.

## Install

No release has been published yet. Until one is, install from source — Rust 1.98 and git
are the only requirements:

```bash
cargo install --git https://github.com/uk0064/stilltrue
```

That installs `stilltrue` and `stilltrue-hook`, the agent adapter.

## Usage

New here? [The quickstart](docs/quickstart.md) goes from install to CI in six steps,
including what a quiet run does and does not mean. Finishing a change, or wiring the
check into pre-commit? See [the repair recipe](docs/repair.md). Using Claude Code or
Codex? [The agent adapters](docs/agents.md) run the check when a session starts and each
time the agent finishes, and tell it what they found.

```bash
stilltrue                      # Tier A across the default document set
stilltrue --summary            # say what was examined and what was not, on stderr
stilltrue --strict             # also report Tier B
stilltrue docs/ CLAUDE.md      # filter the set; excludes still apply
stilltrue --format sarif       # or json, or github; same gate, same findings
stilltrue --fail-on any        # or none; decides the exit code only
stilltrue --require-history    # exit 2 rather than run blind
stilltrue --no-cache           # history results are cached per repository; see below
stilltrue --fix                # rewrite claims that can only mean one thing
stilltrue --fix-dry-run        # show those rewrites as a diff, and write nothing
stilltrue --report-file r.json # a versioned run report, for agents and tooling
stilltrue --sarif-file s.sarif # SARIF beside any other format, from the same scan
```

A quiet run is only a clean one when it was complete. The summary and the run report
say which: `complete`, `incomplete` — an unreadable document, a manifest that did not
parse, a shallow clone — or `no-input`, when nothing matched. They also name what was
not judged and why, so silence has a stated meaning rather than an assumed one.

Adopting it on a repository that already has findings:

```bash
stilltrue --write-baseline .stilltrue-baseline   # record today's backlog
stilltrue --baseline .stilltrue-baseline         # from now on, only new rot
```

Zero config is the intended way to run it. By default it reads every README, everything
under `docs/`, CLAUDE.md and AGENTS.md files, Cursor rules, Copilot instructions and
agent skills, and leaves out changelogs and release notes. `stilltrue.toml` in the
repository root, if you want one:

```toml
include = ["README*", "docs/**"]   # replaces the defaults; it does not extend them
exclude = ["docs/legacy/**"]       # extends the defaults, which already drop changelogs
strict  = false

[ignore]
symbols = ["LegacyThing"]
env     = ["CI"]
```

History results are cached outside the repository, in the platform cache directory, or
wherever `STILLTRUE_CACHE_DIR` points. The cache keeps itself bounded: a repository
unused for thirty days is dropped, and at most 256 are kept.

Exit codes: `0` nothing fatal, `1` fatal findings, `2` internal error or, under
`--require-history`, unavailable history.

Suppress a block with an HTML comment on the line before it:

```markdown
<!-- stilltrue:ignore this fixture is broken on purpose -->
```

## What it won't tell you

Silence is a design position, so it is worth knowing where it is deliberate:

- A bare manifest name — `requirements.txt`, `package.json`, `poetry.toml` — is never
  judged. A tool whose documentation describes the reader's files writes the same token
  as one describing its own. Say `./requirements.txt` to opt back in.
- Changelogs and release notes are not linted. Naming something that has since moved is
  what that genre is for.
- An environment variable read through a dependency looks unread. One whose family the
  repository assembles — `"POETRY_" + key.upper()` — is not judged at all, so a real
  rot inside such a family goes unreported too.
- Any attribute on an instance is out of reach: resolving it is type inference, and
  there is none here. A module that serves its names from a lazy `__getattr__` is
  treated the same way — it computes its attributes, so nothing static can read them.
- External URLs are not checked. That is [lychee](https://github.com/lycheeverse/lychee).
- A bare `download_file` in prose is not judged; `download_file()` is.
- A version written as a family — `Python 2.x`, `Node 18.*` — is not judged, because a
  family cannot contradict a pin.
- A pin file naming a channel rather than a version — `channel = "stable"`, `lts/*`,
  `system` — pins nothing a number can contradict, so a repository whose toolchain
  moves with a stream gets no version checking at all.
- Commands in an untagged fence are not read, because a fence that does not say it is
  shell is as likely to be a config sample as a command.
- A nested repository — a submodule, a clone inside the checkout, or a worktree an agent
  keeps there — is not read at all. Its documents describe their own repository, and its
  files are not evidence about this one. Run the tool inside it to lint it.
- A Rust edition is not a Rust version: "Rust 2024" is never compared with a toolchain
  pin.
- A symbolic link is not followed, as a document or as source. Following them invites
  walking the same file twice and, with a cycle, forever. A symlinked document is not
  linted, and a symlinked source file is not indexed — which costs a finding rather
  than inventing one, since a definition the index cannot see has no history to promote
  it out of silence either.

## History is not optional

Without git history nothing can be reported as rot, and GitHub's checkout action is
shallow by default — so a naive setup runs green on a rotting repository. `stilltrue`
says so loudly rather than quietly, and the bundled action repairs the clone itself
rather than asking you to edit your checkout step:

```yaml
- uses: actions/checkout@v4
- uses: uk0064/stilltrue@vX.Y.Z
  with:
    version: X.Y.Z
```

No release has been verified from its published archives yet, so this shows a
placeholder rather than a version that may not exist or a branch that moves under you.
Once `tests/release/verify-published.sh` passes for a release on every advertised
platform, that release is the one to pin, in both places.

Findings arrive as PR annotations, which need no token permissions. The binary renders
them itself under `--format github`, so the Action carries no runtime. The summary goes
to the job summary, and the run report's path is the step's `report-file` output. To
feed code scanning as well, ask for the file and upload it yourself — the same scan
writes both:

```yaml
- uses: uk0064/stilltrue@vX.Y.Z
  with:
    version: X.Y.Z
    sarif-file: ${{ runner.temp }}/stilltrue.sarif
- uses: github/codeql-action/upload-sarif@v3
  # `if: always()`, because findings make the step above exit 1 — and that is exactly
  # when there is something to upload. Writing outside the checkout keeps a later
  # `git diff --exit-code` honest.
  if: always()
  with:
    sarif_file: ${{ runner.temp }}/stilltrue.sarif
```

Run it on every pull request, not only those that touch documents: a code change is
what breaks an untouched document, so a Markdown-only `paths:` filter skips exactly the
changes this exists for.

`version` is required and has no default, because it names a published release: until
one exists the action cannot install anything, and a default pointing at a release that
is not there would fail with a download error instead of saying so.

## Building

Rust 1.98 and git are the only requirements.

```bash
cargo build
cargo test
```

Fixtures are real git repositories, because history is part of the logic. Every gating
rule ships two: the finding it must report, and the near-miss it must stay silent on.
The near-miss fixtures are the regression suite that protects precision — treat
deleting one as a serious change.

Whether the agent adapters improve an agent's work is what
[the benchmark harness](bench/README.md) measures; no live run has been made yet.

## Design and contributing

How it works, and every decision behind it, is in [the design document](docs/design.md).
The vocabulary is in [CONTEXT.md](CONTEXT.md), and how work is done here — tests,
verification, documents and commits — is in [CONVENTIONS.md](CONVENTIONS.md).
Maintainers: see [the release procedure](docs/releasing.md) for binary packaging,
release validation, and crates.io publication.

## License

[MIT](LICENSE).
