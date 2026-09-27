# Quickstart

From nothing to a check in CI, in six steps. No account, API key or model is involved:
stilltrue is a static checker, and everything below runs on your machine or your CI
runner.

## 1. Install

Prebuilt archives, `cargo install stilltrue` and a pinned Action version arrive with the
first published release, and none has been verified yet. Until one is, install from
source — Rust 1.98 and git are the only requirements:

```bash
cargo install --git https://github.com/uk0064/stilltrue
```

From a checkout of this repository, `cargo install --path .` does the same.

## 2. Run it

In the repository whose documents you want checked:

```bash
stilltrue --summary
```

With no configuration it reads the README, `docs/`, CLAUDE.md, AGENTS.md, agent rule
files and skills, and skips changelogs. `--summary` adds a few lines on stderr saying
what that meant for your repository:

```text
stilltrue: summary (complete)
  documents   18 matched · 18 read · 0 unreadable
  claims      240 extracted · 0 ignored · 212 true · 1 broken · 22 skipped · 5 ambiguous
  gate        1 rot · 0 lie reported · 0 lie not reported · 0 abstained · 0 history failed
  baseline    0 suppressed
  history     complete · 1 searches · 5 git processes
  skipped     command/no-manifest 14 · version/no-pin 8
  ambiguous   symbol/unsupported-language 5
stilltrue: 1 finding(s) reported (1 rot, 0 lie). Checked 18 document(s); …
```

## 3. What a quiet run means

`stilltrue: no findings` on its own says less than it seems to. Read the last line of
the summary instead, and its status:

- **complete** — every document was read, every manifest parsed, and history was
  available. A quiet complete run means no claim the tool can check has rotted.
- **incomplete** — something that should have been read was not: an unreadable file, a
  manifest that did not parse, a shallow clone. The summary names each one. A quiet
  incomplete run is not a clean one.
- **no-input** — no document matched, usually a mistyped path or an `include` in
  stilltrue.toml that is too narrow. Nothing was examined.

Even a complete run checks only what it can extract: paths, commands, environment
variables, pinned versions, links and — in wholly Python or TypeScript repositories —
symbols. The summary lists what it skipped or could not judge, and why. It never says
your documentation is correct, because nothing here can know that.

Tools and agents should not parse this text. `--report-file report.json` writes the
same information as a versioned JSON envelope; read its `status` field.

## 4. History, and what it costs

A claim is reported as rot only when git history proves it was once true, so the tool
needs history. In a shallow clone it says so loudly, reports nothing as rot, and marks
the run incomplete. Locally, `git fetch --unshallow` repairs one.

In CI the Action deepens a shallow checkout for you. That fetch is the largest cost of
running the check: it downloads the repository's whole history, once per job. For most
repositories that is seconds; on a very large one, see what a full clone weighs with
`git count-objects -vH`. Set `unshallow: false` to skip it, knowing that nothing can
then be reported as rot.

## 5. Fix a finding

A finding names the claim, the commit that likely broke it, and what it may have meant:

```text
CLAUDE.md:14:8  rot  command `make demo` has no target in Makefile
                     likely broke in a3f21c9 "split demo into seed+serve" · 4 months ago
                     did you mean: make seed, make serve?
```

Two candidates is a choice, and a choice is yours: edit the document, then run the check
again and look for a complete, quiet run. When a finding has exactly one candidate,
`stilltrue --fix-dry-run` shows the edit as a unified diff and writes nothing, and
`stilltrue --fix` makes exactly that edit — the two share one plan, and a document that
changed in between is left alone. Either way the exit code reflects what was found, so a
CI job cannot turn green by rewriting the repository.

[The repair recipe](repair.md) is the same loop for any change, with an optional
pre-commit hook that runs it before each commit.

This walkthrough is not just an illustration: `tests/demo.rs` builds that repository,
removes the target, checks the finding matches the README word for word, corrects the
document and checks the rerun is complete and clean.

## 6. Turn it on in CI

Start advisory, so the first run informs rather than blocks:

```yaml
on:
  pull_request:
  push:
    branches: [main]

jobs:
  stilltrue:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: uk0064/stilltrue@vX.Y.Z
        with:
          version: X.Y.Z
          fail-on: none
```

`vX.Y.Z` is a placeholder: pin the release you installed, once one is published and
verified. Do not add a `paths:` filter that limits the job to Markdown files. A code
change is what breaks an untouched document, so that filter skips exactly the pull
requests the check exists for.

If the first run reports a backlog you are not ready to fix, record it and gate only on
new rot:

```bash
stilltrue --write-baseline .stilltrue-baseline
git add .stilltrue-baseline
```

Then pass `args: --baseline .stilltrue-baseline` to the Action and remove `fail-on:
none`. Delete a line from the baseline to start judging that finding again.

With a coding agent, the same check can run when a session starts and each time the
agent finishes, so stale instructions are caught while the agent can still fix them. See
[the agent adapters](agents.md) for Claude Code and Codex.

Configuration is optional and best left alone at first. When you need it, see
stilltrue.toml in the README — and note that `include` there replaces the default
document set rather than adding to it.
