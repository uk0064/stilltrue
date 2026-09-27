# stilltrue

A linter that binds claims made in Markdown to the real repository. This glossary is
the project's ubiquitous language: the words below mean exactly this, in code, in
docs, and in output.

## Language

### The subject matter

**Document**:
A Markdown file under lint. The default set is listed in the README and defined in
`src/config.rs`; it is overridable but never empty.
_Avoid_: doc file, source doc, target file

**Claim**:
A statement a document makes about the repository, extracted with an exact byte span.
Every claim is one of six types and is either true or broken.
_Avoid_: assertion, reference, annotation

**Path**, **Command**, **EnvVar**, **Symbol**, **Version**, **Link**:
The six claim types. A claim's type is decided once, in `extract`, by first-match
precedence, and never revisited.

**Runner**:
The executable a `Command` claim invokes — `make`, `npm`, `cargo`, `just`. Only
runners on the allowlist produce claims.
_Avoid_: tool, executor, binary

**Manifest**:
The file that declares a runner's available targets: `Makefile`, `package.json`,
`justfile`. A command claim is checked against a manifest, never against the
filesystem.
_Avoid_: config, build file

### Truth

**True**:
The claim still describes the repository.
_Avoid_: resolved, valid, passing, green

**Broken**:
The claim provably does not describe the repository. Says nothing yet about whether a
human should be told.
_Avoid_: failed, stale, invalid, drifted

**Evidence**:
What a resolver attaches to a broken claim so that history can be searched without
re-reading the claim text: a needle, a scope, and a candidate set.

**Needle**:
The exact pattern history is searched for, to establish whether the claim was ever
true. Supplied by the resolver that failed, never derived from claim text.
_Avoid_: search string, query, term

**Scope**:
The path restriction a needle is searched within. `demo:` means nothing repo-wide; it
means something in `Makefile`.

**Candidate**:
A real target the claim could plausibly have meant, used for suggestions. `make seed`
is a candidate for a broken `make demo`.

### Judgement

**Tier**:
How much confidence a broken claim carries. A, B, or C — not a severity, a confidence
class.

**Rot** (Tier A):
Broken, and history proves it was once true. The code moved and the document did not.
Reported by default. This is the product.
_Avoid_: drift, staleness

**Lie** (Tier B):
Broken, with no evidence it was ever true. A different bug with a different cause —
usually a document written from imagination. Requires `--strict`.

**Ambiguous** (Tier C):
Broken, but the tool cannot honestly say whose repository the claim was about, or its
understanding of this one is too incomplete to judge. Never reported, at any flag.
Reached by classification, not by configuration: a resolver with no definition-shaped
needle, a symbol in a repository that is not wholly supported, a bare ecosystem filename
such as `requirements.txt`, an extensionless link target that cannot be located, and a
dotted symbol whose namespace is neither a module of this repository nor a class.

**Suppression**:
An author's explicit statement that a claim should not be judged.

**Degraded run**:
A run in which git history is unavailable — no repository, or a shallow or partial
clone. No claim can reach Tier A in a degraded run, so the tool must say so out loud.

### Accounting

**Status**:
Whether a run did what it set out to: **complete**, **incomplete** (a read, parse, walk
or history search failed, or history was degraded) or **no-input** (nothing matched).
Separate from coverage. A quiet incomplete run is not a clean one.
_Avoid_: success, passed, verified

**Coverage limitation**:
Something a complete run did not judge, for an expected reason — an unsupported
language, an absent manifest. Named, counted, never a failure.

**Reason code**:
The stable, kebab-case name for why a claim was skipped or ambiguous:
`no-manifest`, `manifest-unparseable`, `unsupported-language`. A public interface, like
a rule id.

**Diagnostic**:
An operational issue a run met. Not a finding, and never a way to report an ambiguous
claim.

**Run report**:
The versioned JSON envelope `--report-file` writes: status, coverage, diagnostics, and
the same findings as the output, with their fingerprints.

**Edit plan**:
What `--fix` and `--fix-dry-run` share: each edit's span, the text it must still hold,
its replacement, and the document it was read from.

### Sessions

**Session**:
One agent conversation in one worktree, as a hook adapter sees it. Its state lives
outside the repository.

**Starting point**:
The findings a session's first complete scan found. What "new" is measured against.
_Avoid_: baseline, which is the `--baseline` file

**Comparison epoch**:
The span over which one starting point holds. A new session, or a change of tool,
configuration or branch, begins another, and records why.

**Newly observed**:
Present at a completion check and absent from the starting point. Never "introduced by
the agent" — someone else may have changed the repository.

### Testing

**Near-miss**:
A fixture built to look like a finding and be silent. Near-misses are the precision
regression suite; deleting one is a serious change.
_Avoid_: negative test, false-positive test
