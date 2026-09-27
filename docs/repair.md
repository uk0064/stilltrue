# Repairing what stilltrue finds

A recipe for finishing a change — by hand, in an editor, or by a coding agent — with the
command-line tool alone. No daemon, server or plugin is involved.

## The recipe

1. **Finish the edits.** Checking half a change reports what the other half will fix.
2. **Check the whole repository**, not the files you touched. A code change is what
   breaks a document nobody opened:

   ```bash
   stilltrue --summary
   ```

3. **Read the evidence, and the coverage.** Each finding names the claim, the commit
   that likely broke it, and — when there are five or fewer — every candidate it could
   have meant. `git show` on that commit is usually the fastest way to see what moved.
   Then read the summary's status line: a quiet run is only a clean one when it says
   `complete`.
4. **Preview the edits the tool can make safely:**

   ```bash
   stilltrue --fix-dry-run
   ```

   Only a claim with exactly one candidate is rewritten. Two candidates is a choice, and
   the preview leaves it for you.
5. **Apply the edits you accept.** `stilltrue --fix` writes exactly what the preview
   showed, and refuses to write a document that changed after it was read. Make the
   remaining changes yourself: update the document to what the repository now says, or
   restore what it names if the removal was a mistake.
6. **Run the check again** and look for a complete, quiet run.

Do not make a finding go away by deleting the instruction, adding a suppression marker,
or turning the check off. Those make the run quiet without making the document true, and
the next reader follows the same stale instruction.

For tools rather than people, add `--report-file report.json` to steps 2 and 6: it
carries the same findings with stable fingerprints, the run's `status`, and — with
`--fix` or `--fix-dry-run` — every edit, as eligible, applied or rejected with a reason.

## Before each commit, with pre-commit

This repository publishes two [pre-commit](https://pre-commit.com) hooks. Both check the
whole repository on every commit, take no file list, and run whether or not a document
was staged — a filter on changed files would skip exactly the commits that break an
untouched document.

- `stilltrue` builds the tool from source with pre-commit's Rust support. The first run
  compiles it, which takes a minute or two.
- `stilltrue-system` runs a `stilltrue` already on your PATH, and builds nothing.

Nothing installs them for you, and nothing here edits your hook configuration. Add one
to your `.pre-commit-config.yaml` yourself:

```yaml
repos:
  - repo: https://github.com/uk0064/stilltrue
    rev: vX.Y.Z
    hooks:
      - id: stilltrue-system
```

`vX.Y.Z` is a placeholder until a release is published and verified; see the
quickstart.

One behaviour to know: pre-commit sets unstaged changes aside while hooks run, so the
hook checks what you *staged*. A document you fixed but have not added is not seen, and
the hook still fails; a direct `stilltrue` run reads the working tree and passes. Stage
the fix. `tests/product/precommit.sh` pins both behaviours, along with a code-only commit
that breaks an untouched document.

The hook is a convenience, not the gate. Keep the check in CI as well: it examines the
repository as merged, which no local hook can promise.
