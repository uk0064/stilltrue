# stilltrue — design

stilltrue binds the claims a document makes — a path, a command, an environment
variable, a version, a link, a symbol — to the repository the document sits in, and
fails CI when the code moved and the document did not. It reads the instruction files
coding agents follow (CLAUDE.md, AGENTS.md, Cursor rules, skills) and ordinary READMEs
with the same checks. It is static analysis: there is no model anywhere in it.

This document explains how it works and why. It starts with the thesis, walks the
pipeline stage by stage, and then covers the surfaces built around it: configuration,
suppression, baselines, autofix, the run report, the GitHub Action and the agent
adapters. It ends with how precision is verified. The vocabulary — claim, true, broken,
rot, lie, needle, scope, tier — is defined in [CONTEXT.md](../CONTEXT.md) and used here
exactly as defined there.

Each design decision is recorded as an ADR in the section it governs, with its context,
the decision and its consequences. Decisions keep their original numbers, so the ADR
citations in the code (ADR-0010 and so on) still find them; the index below lists them
in number order. A new decision takes the next free number and goes in the section it
belongs to, with a row in the index.

## Decision index

| ADR | Decision | Section |
|---|---|---|
| [0001](#adr-0001--resolution-failure-carries-evidence) | Resolution failure carries evidence | [Architecture](#architecture) |
| [0002](#adr-0002--no-claims-inside-non-shell-fences) | No claims inside non-shell fences | [Extraction](#extraction) |
| [0003](#adr-0003--version-claims-resolve-against-exact-pins-only) | Version claims resolve against exact pins only | [Resolution](#resolution) |
| [0004](#adr-0004--symbol-claims-require-a-fully-supported-repository) | Symbol claims require a fully supported repository | [Resolution](#resolution) |
| [0005](#adr-0005--history-needles-are-definition-shaped-and-scoped) | History needles are definition-shaped and scoped | [Gating](#gating) |
| [0006](#adr-0006--degraded-runs-are-loud) | Degraded runs are loud | [Gating](#gating) |
| [0007](#adr-0007--the-cache-lives-outside-the-repository) | The cache lives outside the repository | [The history cache](#the-history-cache) |
| [0008](#adr-0008--machine-formats-obey-the-same-gate) | Machine formats obey the same gate | [Output](#output) |
| [0009](#adr-0009--the-action-repairs-the-clone-and-annotates-by-default) | The Action repairs the clone and annotates by default | [The GitHub Action](#the-github-action) |
| [0010](#adr-0010--history-needles-are-posix-ere) | History needles are POSIX ERE | [Gating](#gating) |
| [0011](#adr-0011--a-bare-ecosystem-filename-is-ambiguous) | A bare ecosystem filename is ambiguous | [Resolution](#resolution) |
| [0012](#adr-0012--historical-records-are-not-documents) | Historical records are not documents | [The document set](#the-document-set) |
| [0013](#adr-0013--a-dotted-symbol-is-judged-only-inside-a-readable-namespace) | A dotted symbol is judged only inside a readable namespace | [Resolution](#resolution) |
| [0014](#adr-0014--a-baseline-is-a-set-of-fingerprints) | A baseline is a set of fingerprints | [Baselines and autofix](#baselines-and-autofix) |
| [0015](#adr-0015--autofix-rewrites-only-what-it-can-enumerate) | Autofix rewrites only what it can enumerate | [Baselines and autofix](#baselines-and-autofix) |
| [0016](#adr-0016--an-unused-suppression-is-reported-under---strict) | An unused suppression is reported under `--strict` | [Configuration and suppression](#configuration-and-suppression) |
| [0017](#adr-0017--a-symbol-claim-needs-a-syntactic-marker) | A symbol claim needs a syntactic marker | [Extraction](#extraction) |
| [0018](#adr-0018--an-assembled-variable-name-is-ambiguous) | An assembled variable name is ambiguous | [Resolution](#resolution) |
| [0019](#adr-0019--a-run-says-whether-it-was-complete) | A run says whether it was complete | [The run report](#the-run-report) |
| [0020](#adr-0020--a-failed-history-search-is-not-an-answer) | A failed history search is not an answer | [Gating](#gating) |
| [0021](#adr-0021--one-analysis-several-outputs) | One analysis, several outputs | [Output](#output) |
| [0022](#adr-0022--fixes-are-planned-then-applied) | Fixes are planned, then applied | [Baselines and autofix](#baselines-and-autofix) |
| [0023](#adr-0023--agent-adapters-advise-at-session-boundaries) | Agent adapters advise at session boundaries | [Agent adapters](#agent-adapters) |
| [0024](#adr-0024--a-rust-edition-is-not-a-rust-version) | A Rust edition is not a Rust version | [Extraction](#extraction) |
| [0025](#adr-0025--a-nested-repository-is-not-this-repository) | A nested repository is not this repository | [The document set](#the-document-set) |

## The problem and the thesis

Agent instruction files are not documentation in the old sense. They are instructions a
machine executes against. When one names a make target that was deleted, a script that
was renamed or a module that moved, the agent silently takes a wrong action, spends
tokens recovering, and nobody is told. READMEs rot the same way and get the same checks
for free.

Every documentation linter that has been uninstalled was uninstalled for false
positives, not for what it missed. So **precision is the product**, and the design
keeps two questions apart that other tools fold together:

- *Is this claim false?* That is `resolve`'s question, and it is answered as strictly
  as the evidence allows.
- *Is this worth a human's attention?* That is `gate`'s question, and it is answered by
  asking git history whether the claim was ever true.

Only a claim that is broken *and* provably once true — rot — is reported by default.
When in doubt, stay silent: a missed finding costs nothing, and a false one costs the
whole tool. Most of the decisions below are this rule applied to a case the corpus
found.

### Non-goals

- **No LLM anywhere in the pipeline.** Every check is static analysis. A check that
  would need a model does not belong here.
- No prose or style linting; that is vale.
- No OpenAPI conformance; that is DriftLinter.
- No external URL checking; that is [lychee](https://github.com/lycheeverse/lychee).
- No rewrite that chooses between candidates. Autofix changes only what has exactly one
  possible meaning ([ADR-0015](#adr-0015--autofix-rewrites-only-what-it-can-enumerate)).
- No third language resolver yet. Python and TypeScript ship; the second is what proved
  the seam, and a third is cheap to add but is still a new way to be wrong about a
  symbol.

## Architecture

One crate, two binaries: `stilltrue`, the checker, and `stilltrue-hook`, the agent
adapter, which runs the checker as a child process. The checker is a four-stage
pipeline:

```
 documents ──▶ extract ──▶ resolve ──▶ gate ──▶ report
               typed        true,       rot,     human, JSON, SARIF,
               claims       broken,     lie,     GitHub annotations,
               with spans   skipped,    abstain  run report, summary
                            ambiguous
```

- **extract** reads each document and produces typed claims, each with its file, line,
  column and exact byte span. The kind of a claim is decided here, once.
- **resolve** decides whether each claim still describes the repository. A broken claim
  carries its own evidence ([ADR-0001](#adr-0001--resolution-failure-carries-evidence)).
- **gate** decides whether a broken claim deserves a human's attention, by searching
  history with the evidence the resolver supplied. It does not know what a Makefile is.
- **report** renders the findings, identically gated, in every format.

`gate` existing as a stage of its own is the thesis in code. It lets a resolver be
strict about truth without that strictness reaching the reader, because the gate decides
separately what is worth saying.

**`resolve` never invokes git.** Git is touched only in `gate`, and only for claims that
are already broken, so a repository whose documents are all true spawns no git process
beyond a handful of fixed probes at start-up — whether this is a repository, whether the
clone is shallow or partial, and where HEAD is.

| Module | Role |
|---|---|
| `src/pipeline.rs` | Wires the stages together, selects documents, tallies coverage |
| `src/extract.rs`, `src/rst.rs` | Extraction from Markdown and from reStructuredText |
| `src/resolve/universal.rs` | Paths, links, environment variables, versions |
| `src/resolve/command.rs` | Commands, one manifest format per runner |
| `src/resolve/lang.rs`, `src/resolve/symbol.rs` | The language resolvers and the symbol resolver |
| `src/resolution.rs` | What `resolve` hands to `gate`: outcomes, evidence, needles, reason codes |
| `src/gate.rs`, `src/git.rs` | Tiers, the history search, suggestions; git behind a trait |
| `src/cache.rs` | The history cache |
| `src/report.rs`, `src/runreport.rs`, `src/coverage.rs` | Output formats, the run report, accounting |
| `src/config.rs`, `src/baseline.rs`, `src/fix.rs` | Configuration, baselines, autofix |
| `src/repo.rs` | The repository: its walk and the indexes built from it |
| `src/main.rs`, `src/hook/` | The command line, and the agent adapters |

### Execution model

The repository is walked once. Extraction and resolution run in parallel across
documents, and every index a resolver needs — the file list, the environment-variable
names, the symbol definitions and namespaces — is built once on the shared repository
value behind a `OnceLock`. Each document produces its own tally of claims, coverage
counts and diagnostics, which are merged after the parallel stage, so no claim waits on
a lock and no true claim outlives its document.

History checks are different: each is a git process, and unbounded parallelism over
process spawns is a fork bomb on a large repository. They run in a pool of at most
eight threads, or fewer on a machine with fewer cores.

Determinism comes from one rule: findings are collected, then sorted by path, line,
column and rule id before anything is rendered. Snapshot tests depend on nothing else.

The performance budget is a warm run under one second over 500 documents in a
5,000-file tree. It shaped two choices. The indexes are built once per run, not per
claim; and each language's tree-sitter query is compiled once per grammar, not once per
source file — compiling it per file is what once made that benchmark take eight
seconds warm.

### Errors never fail the build

A linter that fails a build because it choked gets removed. An unreadable document, a
manifest that does not parse, a bad glob or a git failure each warn on stderr and skip
that unit. Skipping is never silent: every such failure is also a diagnostic that marks
the run incomplete ([ADR-0019](#adr-0019--a-run-says-whether-it-was-complete)). Exit
status `2` is reserved for faults the run cannot recover from, for outputs that were
asked for and could not be written, and for `--require-history` in a run without
history.

Git is invoked as a subprocess behind the `Git` trait rather than linked as a library.
Git is in every CI image, a subprocess is easy to reason about, tests can substitute a
repository that answers whatever they need, and the trait keeps a linked library
available as a later optimisation.

### ADR-0001 · Resolution failure carries evidence

**Context.** A document says "make demo", and the Makefile has no `demo` target. The
string that proves the claim was once true is not the claim's text — "make demo" never
appears in a Makefile. It is the target definition, `demo:`, searched for in the
Makefile the claim was checked against. Only the resolver that just failed knows that.

**Decision.** `resolve` does not return a boolean for `gate` to reverse-engineer. A
broken claim carries evidence: the *needle* history should be searched for, the *scope*
to search it in, the *candidates* the claim might have meant, and a message. The gate
performs the same search for every claim kind and never derives a search string from
claim text.

**Consequences.** Adding a claim kind or a runner means teaching one resolver to
describe its own failure. It never means touching `gate`, which stays ignorant of every
manifest format. The flagship finding — a broken command with the commit that likely
broke it — is reachable only because of this.

## The document set

A document is a Markdown or reStructuredText file (`.md`, `.markdown`, `.mdc`, `.mdx`,
`.rst`) that the include globs select and the exclude globs do not. The defaults are:

```
include:  README*  docs/**  CLAUDE.md  AGENTS.md  .cursor/rules/**  **/SKILL.md
          .github/copilot-instructions.md
          and README*, CLAUDE.md and AGENTS.md at any depth
exclude:  CHANGELOG*  CHANGES*  HISTORY*  NEWS*  RELEASE-NOTES*  RELEASE_NOTES*
          RELEASES*, at any depth and in any case
```

Nested READMEs are included because a monorepo documents a package's commands in the
package's own README, which is exactly the case the nearest-manifest rule for commands
exists for. Zero configuration is the intended way to run the tool, and it must produce
useful output on a repository that has never heard of it.

The walk is gitignore-aware and includes hidden files, so `.cursor/rules/**` and
untracked documents are seen. It does not follow symbolic links, as documents or as
source: following them invites reading one file twice and, with a cycle, forever. The
cost is a missed finding rather than an invented one — a definition the index cannot
see has no history to promote a claim out of silence either. The walk does not enter
another repository nested inside this one
([ADR-0025](#adr-0025--a-nested-repository-is-not-this-repository)).

Positional paths on the command line filter the selected set rather than replacing it,
so excludes still apply. They are taken relative to the working directory and rebased
onto the repository root; without that, an absolute path or one typed from a
subdirectory matches nothing and the run passes having read nothing. A path outside the
repository is dropped with a diagnostic, and dropping every path lints nothing rather
than everything.

### ADR-0012 · Historical records are not documents

**Context.** A changelog records what was true at a release; its entries are supposed
to name things that have since moved. Linting one inverts the tool: every entry that
describes a rename becomes a finding precisely because the rename happened. On the
corpus, 27 of 80 remaining findings sat in changelogs and release notes — typer's
release notes were reported for naming the Python version a past release supported.

**Decision.** Historical records are excluded by default at any depth, matched
case-insensitively on the file name: the seven patterns in the exclude list above.
`exclude` in the configuration file extends this list rather than replacing it, so
configuring one exclusion cannot re-admit every changelog. The default excludes apply
whatever `include` says, so a changelog cannot be opted back in. The same argument
applies inside reStructuredText, whose `versionadded`, `versionchanged`,
`versionremoved` and `deprecated` directives are records of the past inside a live
document: the literals, links and commands in their bodies are not read.

**Consequences.** This is a statement about genre made from a file name, the only
signal available without reading prose. A changelog named something else is linted, and
a design document named HISTORY.md is skipped. Both failures are quiet ones, which is
the direction this tool errs in.

### ADR-0025 · A nested repository is not this repository

**Context.** An agent's worktree kept inside the checkout, under .claude/worktrees, was
walked as part of this repository. Its documents were linted as this repository's, and
its files counted as evidence here: a worktree checked out before a rename still
defined the old name, so the definition index held it and the rot the rename caused in
this repository's own CLAUDE.md was silenced. For the agent adapters it was worse,
because every edit in the worktree changed the main session's content identity.

**Decision.** A directory below the root that contains a `.git` — a directory for a
submodule or a nested clone, a file for a worktree — is another repository, and the walk
does not enter it. None of its documents is linted and none of its files is evidence.
This is what git itself does. The `nested-worktree` fixture reproduces the silenced
rot.

**Consequences.** A path or link in this repository's documents that names a file inside
a submodule is still checked against the file on disk. Symbol, environment-variable and
command evidence inside the nested repository is no longer read. To lint a submodule,
run stilltrue inside it.

## Extraction

Extraction turns a document into claims. Markdown is parsed with `tree-sitter-md`,
which gives block and inline trees with exact byte ranges; reStructuredText is read by a
deliberately small line-oriented reader. Both produce the same claim model, so
everything downstream is shared. Every claim records its file, 1-based line and column —
counted in Unicode code points, since byte columns misplace every annotation on a line
with non-ASCII text — its end position, and its byte span.

### Claim kinds and where they come from

| Kind | Read from | True when |
|---|---|---|
| Path | inline code spans | the file exists, relative to the document or to the repository root |
| Command | inline code spans, shell fences | the runner's target exists in the nearest ancestor manifest |
| EnvVar | inline code spans | the name is read somewhere in code or configuration, never in prose |
| Symbol | inline code spans | a definition exists, in a repository every resolver can read |
| Version | prose, code spans and every fence | it agrees with an exact version pin |
| Link | Markdown link destinations | the relative target exists, and any anchor names a real heading |

A claim is read from exactly four surfaces: inline code spans, link destinations,
fences tagged as shell, and — for versions only — any text at all. Nothing else inside a
fenced block is ever a claim ([ADR-0002](#adr-0002--no-claims-inside-non-shell-fences)).
A seventh kind, Suppression, is not a claim about the repository; it exists only so an
unused suppression marker can be a finding
([ADR-0016](#adr-0016--an-unused-suppression-is-reported-under---strict)).

### Classifying a code span

An inline code span is classified by first-match precedence, and its kind never changes
afterwards. A code span the author wrapped across lines is unwrapped first, as
CommonMark renders it.

1. **Placeholder, and so nothing.** A span containing `<` or `>`, an ellipsis, a braced
   group, `://`, a glob metacharacter (`*`, `?`, or a bracket pair), or a segment
   written as path/to, your-, YOUR_ or my- is not a claim. This rule runs first because
   a placeholder path contains a slash and would otherwise be a Path. A glob names a set
   of paths, not a path, and an external URL is lychee's business. The rule is
   mechanical on purpose: there is no list of metasyntactic words, because foo and bar
   are real identifiers in real repositories.
2. **Path.** A single token containing an alphanumeric character, which contains `/` or
   ends in an extension on a bounded list (md, rst, toml, json, yaml, yml, py, ts, rs,
   go, sh, txt, lock, cfg, ini). A *bare reference* — a slash-separated name with no
   extension, no leading `./`, `../` or `/`, and no trailing `/` — is not a claim:
   `actions/checkout`, an npm scope and this tool's own rule ids have the shape of a path
   and none of the substance. Writing `docs/` or `./docs` says a path is meant. The
   extension list is what keeps a dotted symbol from being read as a file.
3. **Command.** The first token is a runner on the allowlist: make, npm, pnpm, yarn,
   cargo, go, uv, uvx, just, docker, python and deno. Any other span of more than one
   token is not a claim.
4. **EnvVar.** Upper-case letters, digits and underscores, at least three characters,
   starting with a letter — *and* containing an underscore. Without the underscore the
   pattern matches README, TODO, API and JSON, and an all-caps word is as likely an
   acronym as a variable. PATH and CI are lost, which is the right trade.
5. **Symbol.** Every segment spellable as an identifier, and a syntactic marker: a call
   or a namespace ([ADR-0017](#adr-0017--a-symbol-claim-needs-a-syntactic-marker)).
6. **Anything else** — a single lowercase word, a phrase — is not a claim. This rule
   alone removes most of the potential noise.

### Shell fences

A fence whose info string starts with bash, sh, shell, console or zsh yields Command
claims. A shell fence is not one command, and segmenting it is deliberately shallow —
never a real shell parse:

1. Drop comment lines and join `\` continuations.
2. Split on `;`, `&&`, `||` and `|`.
3. In each segment, drop a leading prompt (`$`, `%`, `>`), then leading `VAR=value`
   assignments and `sudo`.
4. The first remaining token is the runner. A runner not on the allowlist yields no
   claim.
5. If the runner or its target contains `$` or a backtick, there is no claim: a name
   computed at runtime cannot be provably broken. A computed flag beside a real target
   says nothing about the target, so only those two tokens are tested.

### Version claims

A version claim is a tool name — Node.js, Node, Python, Rust, Go, pnpm, npm, yarn or
uv — followed by spaces or tabs, an optional `v`, and a dotted number. It is read from
anywhere in the document, because versions are stated in prose. The separator excludes
line breaks: a tool and a number split across a line are more often two sentences than
one claim.

A trailing wildcard where a number would stand, as in `2.x` or `18.*`, names a family,
and a family cannot contradict a pin — the same reasoning as the glob clause above. A
Rust edition is not a version at all
([ADR-0024](#adr-0024--a-rust-edition-is-not-a-rust-version)).

### reStructuredText

A whole documentation ecosystem never adopted Markdown, and a repository whose docs are
`.rst` got nothing from this tool until it read them. The reader is not a parser: only
three reStructuredText surfaces carry claims, and a line reader that understands them is
far less likely to invent a claim than a half-finished grammar. It reads double-backtick
inline literals, classified exactly as code spans are; `code-block`, `sourcecode` and
`code` directives whose language is shell (including `shell-session`), segmented as
shell fences are; and inline links written as `` `text <target>`_ ``. Versions are read
from the whole text, as in Markdown. Everything else in the bodies of historical
directives is skipped ([ADR-0012](#adr-0012--historical-records-are-not-documents)).

### ADR-0002 · No claims inside non-shell fences

**Context.** Fenced code blocks are where placeholders, other repositories' commands and
illustrative configuration live. They are the largest single source of false positives
available to a tool like this.

**Decision.** Only fences tagged `bash`, `sh`, `shell`, `console` or `zsh` produce
claims, and only Command claims. Every other fence, including an untagged one,
contributes nothing but versions. Paths, symbols and environment variables come from
inline code spans in prose. In Markdown this is enforced structurally: fenced content
has no inline tree, so there is nothing for the code-span rules to run on, rather than a
filter someone could forget.

**Consequences.** A genuinely broken path inside a Python fence is missed. That is
accepted and deliberate — not an oversight to be fixed by widening extraction. An
untagged fence is as likely to be a configuration sample as a command, so it is not read
as shell either.

### ADR-0017 · A symbol claim needs a syntactic marker

**Context.** An earlier rule made any bare multi-segment snake_case or CamelCase span a
Symbol. On the corpus it produced six false positives and no true ones: download_file,
build_only and simple_page in flask's tutorial, long_description in poetry's docs,
part_attachments in llm's. Each is a name a document uses while *explaining* something —
an example view function, a setup field, a database table — not a claim that this
repository defines it. A bare word with an underscore in it is a word with an
underscore in it.

**Decision.** A Symbol needs a marker that the author meant code as code: a call, as in
fold_events(), or a namespace, a `.` or `::` between identifier characters, as in
llm.get_model. Every segment must also be spellable as an identifier, so a
pyproject key with a hyphen in it is never a symbol.

**Consequences.** Documentation that names a function bare goes unchecked. That is the
cost, and six false positives against no true one is not a close call. The call and the
namespace are the forms the fixtures use and the forms the recall audit found real rot
through, so the rule keeps the shapes that were demonstrably working.

### ADR-0024 · A Rust edition is not a Rust version

**Context.** This repository's own lint reported one of its plans for saying the crate
was written in Rust 2024. The version rule read 2024 as a compiler version, compared it
with the pin in `rust-toolchain.toml`, found a contradiction, and history — the commit
that created the pin file — promoted it to rot, in a default run. 2024 is the edition: a
revision of the language that every current toolchain supports, named by year. A pin
says which compiler; an edition says which language.

**Decision.** For Rust, a bare four-digit number beginning with 20 is an edition and is
not extracted as a version at all. The rule is a shape, not a list, so a future edition
needs no change. A dotted number, a four-digit number no edition could be, and every
other tool's numbers stay claims. The `version-edition` fixture holds a stale Rust
version and an edition in one sentence and must report exactly the first.

**Consequences.** A document that states a Rust version as a bare year is no longer
judged; there is no such version, so nothing is lost. The self-referential rule found
this, which is the rule working.

## Resolution

Resolution decides, for each claim, one of four outcomes:

- **True** — the claim still describes the repository.
- **Broken** — it provably does not, and it carries evidence
  ([ADR-0001](#adr-0001--resolution-failure-carries-evidence)).
- **Skipped** — there is nothing to compare it against: no manifest for the runner, no
  pin for the tool, a link that leaves the repository. Usually a coverage limitation;
  when the thing to compare against exists but could not be read or parsed, a failure
  that makes the run incomplete.
- **Ambiguous** — the claim does not describe this repository, but the tool cannot
  honestly say it was meant to. This is Tier C, and it is never reported.

Skips and ambiguities carry stable reason codes — `no-manifest`,
`manifest-unparseable`, `no-pin`, `ecosystem-filename`, `unsupported-language` and so
on — which the run report counts
([ADR-0019](#adr-0019--a-run-says-whether-it-was-complete)).

The repository root is the git top level, or the working directory when there is no
repository. Each resolver anchors its claim differently, and the differences are
deliberate.

### Paths

A Path is true if the file exists relative to the document's directory *or* to the
repository root; a leading `/` means the root. Either reading resolves it, and the
ambiguity is settled in favour of silence. A broken path's needle is the path itself,
searched with `git log --full-history` under a `:(literal)` pathspec, so a `*` in the
claim is never read as a wildcard. Its candidates are files elsewhere with the same
name, which is the shape a moved file takes. A broken bare ecosystem filename is
ambiguous instead ([ADR-0011](#adr-0011--a-bare-ecosystem-filename-is-ambiguous)).

### Links and anchors

A Link is resolved relative to its document only, strictly, because that is how GitHub
renders it: a link written with root-relative intent in a nested document really is
broken for its reader. External URLs, mail links and links starting with `/` are not
claims. A bare `#anchor` is a claim about the document itself — a table of contents is
the commonest link shape in a README.

The target must exist. An extensionless target also resolves through the same name
with `.md` added, or an index.md or README.md inside it, because mkdocs and Docusaurus
drop extensions from their URLs. When none of those exists, an extensionless link is
ambiguous rather than broken: such sites resolve links in URL space, where a page served
as a directory moves what `..` means, and which space was meant depends on configuration
this tool does not read. A target that names a file outright and is missing is broken.

An anchor must match a heading of the target under GitHub's slug rules. Explicit `{#id}`
attributes count wherever they appear, including mkdocs' standalone form, and a heading
is slugged both as written and with emphasis markers removed, since GitHub slugs the
rendered text but an underscore inside a name survives. Accepting more spellings errs
towards silence. A broken anchor has a definition-shaped needle of its own — the heading
it names ([ADR-0005](#adr-0005--history-needles-are-definition-shaped-and-scoped)).

Suggestions for a link are spelled relative to the link's own document, because that is
the only place it resolves; a repository-relative suggestion would name a file that is
not there, and `--fix` would write it in.

### Commands

A Command is checked against the nearest manifest for its runner, found by walking up
from the document's directory to the root. That is what makes monorepos work: a build
script named in a package's README is checked against that package's manifest, not the
root one. Manifest names are matched against the names the walk saw, case included,
because each becomes a git pathspec and git is case-sensitive where macOS is not.

| Runner | Manifest | Targets | Needle |
|---|---|---|---|
| make | Makefile, makefile, GNUmakefile | rule names | `^NAME[[:space:]]*:` |
| just | justfile, Justfile, .justfile | recipes and aliases | `^@?NAME[[:space:]]*:` |
| npm, pnpm, yarn | package.json | `scripts` keys | `"NAME"[[:space:]]*:` |
| deno | deno.json, deno.jsonc | `tasks` keys, after deno task | `"NAME"[[:space:]]*:` |
| cargo | `.cargo/config.toml`, .cargo/config | `alias` keys | `^NAME[[:space:]]*=` |

cargo, the npm family, go and uv have built-in subcommands that always resolve —
building, testing, installing and their kin — and a command naming no target resolves
too. docker and python have no repository-local target list, so their commands always
resolve; go and uv have no manifest format here, so their other targets are skipped. A
Makefile with a `%` pattern rule forwards every target, so nothing in it can be missing.
A manifest that exists and does not parse is skipped as `manifest-unparseable`, never
read as empty: read as empty, one stray comma in a package manifest made every script it
names look missing, and each one's history then promoted it to rot. The candidates of a
broken command are the manifest's real targets, and a suggestion carries its runner.

### Environment variables

An EnvVar is true if the literal name appears in code or configuration anywhere in the
repository. The index is a text scan, not a syntax query, because variables are read
through too many idioms per language to enumerate — Python's environ mapping, Node's
process.env, a shell script, a Dockerfile, a CI matrix. It is built once: every
env-shaped token in every walked file that can serve as evidence.

Prose is never evidence. Files with a prose extension (md, markdown, mdx, mdc, rst,
txt) are left out, or every EnvVar claim would satisfy itself from the very document
making it; the dotenv examples (.env.example, .env.sample, .env.template) count. The
tool's own configuration file is not evidence either: a variable named there is named
so that it can be ignored, and counting it would let the configuration satisfy its own
claims.

The needle is the literal name, and its scope carries the same exclusions as git
pathspecs. The index and the needle must describe one set of files. When the needle was
the broader of the pair, a name that only ever appeared in prose was proved by its own
document's history, and an example variable in click's documentation became Tier A.

A name whose family the repository assembles is ambiguous
([ADR-0018](#adr-0018--an-assembled-variable-name-is-ambiguous)).

### Version pins

A Version is compared only with an exact pin
([ADR-0003](#adr-0003--version-claims-resolve-against-exact-pins-only)). It is true when
the stated version is a prefix of the pin at a component boundary: a stated major
version of 24 agrees with a pin of 24.3.1, and 18 does not. When no pin file names the
tool, the claim is skipped as `no-pin`. A broken version's needle is the pin file
itself, so the commit it is blamed on is the last one that changed the pin, and its one
candidate — the pinned version — is offered with the tool's name in front of it.

### Symbols and the language seam

Symbols are the one claim kind that needs to understand code. Each supported language
is one implementation of the `LanguageResolver` trait, which carries everything
language-specific: the file extensions it claims, the tree-sitter grammar for each (TSX
is a different grammar from TypeScript, not a dialect), a definition query, a history
needle, the language's builtins, and a namespace query. Two ship:

- **Python** reads `.py` and `.pyi`: functions, classes, assignments at any depth,
  attribute assignments, parameters and imports.
- **TypeScript** reads eight extensions, the JavaScript it is a superset of included:
  functions, classes, interfaces, type aliases, enums and their members, variables,
  methods, fields, property signatures, object keys, parameters and imports.

**The definition query and the needle must describe the same set.** This is the
invariant the trait exists to carry, and it was broken three times before it was written
down. When the index is the narrower of the pair, every name in the gap resolves broken
and is then promoted to Tier A by its own history: a query that knew functions but not
class attributes produced 244 false positives in a single corpus run. That is why
parameters are in the index — a formatter that puts each parameter on its own line makes
the needle read it as an assignment — and why enum members are.

A symbol is resolved by its normalised name: the last segment, without `()` and without
Sphinx's `~` prefix. It is true if the index holds that name, or if the name is a builtin
of *any* shipped language. Builtins belong in the index — print is not missing from
anything — but never in the candidate set, where seventy of them would push it past the
size that can be enumerated. They are not scoped to the languages present: flask is
wholly Python and its docs name the browser's fetch, which no Python repository could
define.

A broken symbol's needle is every active resolver's needle joined, searched across the
union of their scopes, because a document never says which language a symbol is in.
Symbol checking applies only in a repository every resolver can read
([ADR-0004](#adr-0004--symbol-claims-require-a-fully-supported-repository)), and a
dotted name only inside a namespace the tool can read
([ADR-0013](#adr-0013--a-dotted-symbol-is-judged-only-inside-a-readable-namespace)).
Adding a language means adding one implementation to the list of shipped resolvers, and
its pair of fixtures, and nothing else.

### ADR-0011 · A bare ecosystem filename is ambiguous

**Context.** The tool assumes a document describes the repository it sits in. That holds
for instruction files and breaks for any tool whose documentation describes the files it
operates on. pip-tools' README names requirements.txt and setup.py about twenty-five
times, meaning the reader's files. Neither exists in pip-tools and both once did, so
Tier A fired, exactly as specified, twenty-five times on a document that is not wrong.
Nothing in the text separates the two readings; the token is identical.

**Decision.** A broken Path that is a bare conventional filename — no directory
separator, and a name on a closed list — is ambiguous. The list holds manifests,
lockfiles and the tool configuration a document tells its reader to create:
requirements.txt, setup.py, pyproject.toml, package.json, Cargo.toml, go.mod,
poetry.toml, tox.ini, ruff.toml, this tool's own configuration file and their kin.
Makefile and Dockerfile are deliberately absent, because documents name those far more
often as the repository's own. Saying a path is meant — ./requirements.txt, or
config/requirements.txt — opts back into judgement.

**Consequences.** Tier C became reachable by a classification rule, not only by a
resolver with no needle. A genuinely deleted setup.py named bare goes unreported: one
missed finding against about twenty-five false ones. The list is closed and lives in one
place, `src/resolve/universal.rs`, so widening it is a visible change with a fixture
attached (`ambiguous-manifest`).

### ADR-0018 · An assembled variable name is ambiguous

**Context.** poetry documents variables such as POETRY_VIRTUALENVS_CREATE but never
writes them down: it builds every name from a configuration key at runtime, by
concatenating a prefix with the upper-cased key, and matches whole families with
patterns. The literal names appear only in the documentation, so the scan found nothing,
the claims resolved broken, and the documentation's own history promoted them to rot —
three of the twelve findings the corpus produced at the time, all wrong about the
documentation.

**Decision.** A name whose family the repository is seen to assemble is ambiguous. The
evidence is already in the index: the scan keeps env-shaped tokens, so the prefix the
code writes down, POETRY_, becomes a token — and a name ending in an underscore is not
a variable, because nothing reads it. It is the fragment of a longer name being built.
So if any prefix of a claimed name, cut at one of its underscores, is itself in the
index, the name belongs to an assembled family and cannot be looked up. The test is
reached only after a claim has failed, so it can turn a finding into silence and never
the reverse. It costs no new scan.

A syntax query for concatenation and interpolation would be more precise. It was
rejected because it contradicts the reason the index is a text scan: it would have to be
written per language and would silently stop working in every language without a
resolver. The prefix test works wherever variables are spelled in capitals and
underscores.

**Consequences.** Real rot inside an assembled family goes unreported — in poetry, every
variable it documents. A repository that writes its variables out in full loses nothing,
because a complete name is not a prefix of itself. The corpus went from twelve findings
to eight and from nine quiet repositories to eleven, with its five true positives
untouched.

### ADR-0003 · Version claims resolve against exact pins only

**Context.** Version is the only claim kind whose truth depends on the intent of prose.
A sentence naming a Node version can mean the project requires it, is tested on it,
ships with it or will soon move to it, and no static analysis separates them. Comparing
prose with a range — an engines field, a requires-python constraint — adds semver
algebra on top of an intent that cannot be recovered.

**Decision.** Ranges are ignored entirely. A version is compared only with a file that
pins exactly one version: .nvmrc or .node-version for Node, .python-version for Python,
`rust-toolchain.toml` or a bare rust-toolchain file for Rust, and the go directive of
go.mod for Go. A pin is consulted only when what it holds *is* a version number. A
channel (stable, a dated nightly), an alias (an lts/ name, system) and an implementation
(a PyPy build) each name a moving target, so a stated number cannot contradict them.
Reading one as a pin made every Rust version in a README rot against the commonest
toolchain file there is, a stable channel, blamed on the commit that created it. The
file existing is a different question from the file pinning something.

**Consequences.** No semver dependency and no range comparison. The history check needs
no pattern: the pin file is its own needle. A repository that states its toolchain only
as a range, or pins a channel, gets no version checking, which is the right amount of
checking for a claim that cannot be interpreted.

### ADR-0004 · Symbol claims require a fully supported repository

**Context.** A document never says what language a symbol is in. In a repository that
also contains a language no resolver reads, every symbol defined in that language would
resolve "no definition found" — the strongest false-positive generator available,
aimed at the highest-volume claim kind.

**Decision.** Symbol claims are judged only when every code language present has a
shipped resolver. Languages are detected by file extension, and a language counts as
present from two files, so a stray script does not disqualify a repository. There must
also be at least one file a resolver can read: "every language present is supported" is
vacuously true of a repository with no code, and an empty index would make every symbol
broken. Otherwise every symbol claim is ambiguous (`unsupported-language`), and the run
report names that as a coverage limitation.

**Consequences.** Symbol checking is off in any repository carrying a language neither
resolver reads — including this one, which is written in Rust. Its self-lint is real
for paths, commands, links, environment variables and versions, and vacuous for
symbols. That is a known and accepted gap, not a bug. Mixed-language repositories are
the obvious place a future design would scope symbols per package; that is not built.

### ADR-0013 · A dotted symbol is judged only inside a readable namespace

**Context.** response.next, netrc.netrc() and project.scripts were all reported as rot
on the corpus, and none was wrong. The resolver looked up the last segment and threw
away the only part that said what the name was about. A dotted name is a name inside a
namespace, and judging it means being able to read that namespace. A module of this
repository can be read, and so can a class the index found. An instance cannot — what
`response` is at that point is a question for type inference, which this tool does
not do — and neither can a namespace from outside the repository: netrc is the standard
library, and project is a pyproject table.

**Decision.** A dotted symbol whose first segment is neither a module of this
repository nor a namespace a resolver found is ambiguous (`unreadable-namespace`). A
module is a top-level source file or package at the root or directly under `src` or
`lib`, because only the top level is what an import finds. Namespaces come from a second
query per resolver whose captures are names that can hold other names: Python classes;
TypeScript classes, interfaces, enums, type aliases and namespaces. A module that
defines a module-level `__getattr__` computes its attributes, so it is no more readable
than an instance and is ambiguous too (`dynamic-module`).

This first shipped with capitalisation standing in for "a class", and failed the way
the snake_case rule had: Object, JSON, Array, Math and Decimal are capitalised
namespaces no Python repository defines, and documentation names them as soon as it
mentions a browser or a standard library. Asking the index is not a convention; it is
the question itself. The namespace query is exempt from the query/needle symmetry rule,
because nothing is resolved against it and no history is searched with it. The
definition index cannot serve instead: it holds parameters and locals, as symmetry
requires, so `response` is in it.

**Consequences.** Real rot inside a namespace this repository does not define — a class
imported from a dependency — goes unreported, the same trade as
[ADR-0004](#adr-0004--symbol-claims-require-a-fully-supported-repository). A capitalised
module-level variable is no longer mistaken for a class.

## Gating

Only claims that have already failed resolution reach the gate, so the set is small,
and each arrives carrying the needle and scope its resolver supplied. The gate asks one
question of git — was this ever true? — and assigns a tier.

### Three tiers

- **Tier A, rot.** Broken, and history holds a commit matching the needle: it was true,
  and the code moved. Reported by default. This is the product.
- **Tier B, lie.** Broken, and history has never known the needle. A different bug with
  a different cause — usually a document written from imagination. Reported only under
  `--strict`, or with `strict = true` in the configuration file.
- **Tier C, ambiguous.** Never reported, at any flag, in any format. It is reached by
  classification, never by configuration.

A tier is a confidence class, not a severity. Tier C has seven reasons, each a rule with
its own decision. Six are decided by a resolver: `unsupported-language` (ADR-0004),
`unreadable-namespace` and `dynamic-module` (ADR-0013), `ecosystem-filename`
(ADR-0011), `assembled-name` (ADR-0018), and `site-relative-link` (see
[Links and anchors](#links-and-anchors)). The seventh, `no-history-needle`, is the gate's:
a broken claim whose resolver cannot express a definition-shaped needle
([ADR-0005](#adr-0005--history-needles-are-definition-shaped-and-scoped)). Every shipped
resolver supplies a needle today, so that rule guards the next one written.

The gate records one of five outcomes for every broken claim — rot reported, lie
reported, lie not reported, abstained, or history failed — so the run report can account
for each ([ADR-0019](#adr-0019--a-run-says-whether-it-was-complete)).

### The history search

Every search is a single `git log -1` over HEAD's history, reporting the most recent
matching commit — for a pattern, one that changed how often it occurs; for a path, one
that touched it:

- **Symbol, Python:** `^[[:space:]]*(def|class)[[:space:]]+NAME[^A-Za-z0-9_]` or
  `^[[:space:]]*NAME[[:space:]]*=`, within the Python extensions.
- **Symbol, TypeScript:** a declaration keyword (optionally exported, default, async)
  followed by the name, or the name followed by `:`, `(` or `=`, within all eight
  extensions.
- **Command:** the runner's pattern from the table in [Commands](#commands), within the
  manifest the claim was checked against.
- **EnvVar:** the literal name, no regex, excluding prose and the configuration file.
- **Path, Link, Version:** no pattern; `git log --full-history` on the path under a
  `:(literal)` pathspec — the claimed file, or the pin file.
- **Anchor:** `^#+` followed by the slug's parts joined by runs of non-identifier
  characters, then nothing but spaces, an attribute block or closing hashes to the end
  of the line, matched case-insensitively within the linked document. The end matters:
  without it an anchor for "install" matches a heading "Installation" and is blamed on
  the commit that added the wrong heading.

Only HEAD's history is searched, never every ref. A branch nobody merged must not prove
a claim, the answer must not depend on which refs a CI checkout happened to fetch, and
searching every ref is what made a repository with 172 refs take three minutes.

Pickaxe with a pathspec does not follow renames, so a renamed manifest cuts the trail
and the finding falls to Tier B — silent by default — rather than being mislabelled as
rot. The `history-cut-rename` fixture holds this.

Detection is exact; attribution is a labelled heuristic. The reported commit is the
most recent one matching the needle, which is usually but not always the one that broke
the claim — squashes and merges muddy it. Output always says "likely broke in".

### Degraded runs

A run is *degraded* when history is unavailable: there is no repository, the clone is
shallow, or it is partial (blobs fetched on demand, so a pickaxe reads an incomplete
tree). Detection costs three probes at start-up. In a degraded run no claim can reach
Tier A, so every would-be finding falls to Tier B, which is off by default — the tool
would pass a rotting repository in silence. It is therefore loud
([ADR-0006](#adr-0006--degraded-runs-are-loud)).

### ADR-0005 · History needles are definition-shaped and scoped

**Context.** `git log -S` is a substring search. Normalising graph.query() to the needle
"query" and searching the whole tree matches comments, test names and changelogs, so
nearly every broken symbol would find some commit and land in Tier A. That would make
Tier A the catch-all and leave `--strict` guarding nothing.

**Decision.** Every needle is a pickaxe regular expression shaped like a *definition* of
the thing, and is always restricted to a path scope. A resolver that cannot express a
definition-shaped needle yields Tier C, not Tier A. That rule is what keeps rot meaning
what the glossary says it means.

Headings were first treated as having no such needle. That was wrong: a heading line is
exactly a definition shape, and a slug's parts rejoined with runs of non-identifier
characters reconstruct it closely enough. The approximation only ever errs by matching a
heading that would have slugged to the same anchor, which is the thing being asked
about. It mattered, because retitling a heading is the commonest documentation refactor
there is, and 41% of the relative links in the validation corpus carried an anchor.

**Consequences.** A renamed file cuts the history trail and the finding degrades to a
silent lie; silence on a rename is the correct trade. Each of the six claim kinds now
reaches Tier A through a needle of its own.

### ADR-0010 · History needles are POSIX ERE

**Context.** Git's pickaxe compiles POSIX extended regular expressions, not PCRE. In
that dialect `\b` matches nothing at all and `\s` is silently read as a literal `s`. A
needle written in PCRE habits does not error: it finds no history, and every would-be
rot degrades to a lie that the default run hides.

**Decision.** Needles use `[[:space:]]`, and an explicit non-identifier bracket
expression where a word boundary is wanted — a definition line always has a delimiter
after the name. Target names are escaped before interpolation, for exactly the POSIX
metacharacters and nothing else. The environment-variable needle is a literal, because
POSIX has no portable word boundary and an all-caps underscored name is distinctive
without one. Path needles carry the `:(literal)` pathspec prefix for the same family of
reason: without it a `*` in a claim is a glob and matches unrelated commits.

**Consequences.** Do not tidy `[[:space:]]` into `\s`. The make and npm needles once
survived `\s*` only because it degrades to `s*`, which still matches zero characters;
the Python needle, which used `\s+`, did not. The regression test is in `tests/gate.rs`,
and it drives needles produced by the *resolvers* against a real git repository —
needles written by hand in a test are written in the same dialect as the code and cannot
catch the mismatch.

### ADR-0020 · A failed history search is not an answer

**Context.** A pickaxe that finds nothing exits 0 with no output, and means the needle
never existed. Git failing — a bad pattern, a corrupt object, no git at all — used to
return the same empty answer. Under `--strict` a claim whose search had failed was
reported as a lie, when in truth nobody had looked; and the cache stored the failure as
a negative, so every later run at that HEAD read "never existed" without asking git
again. Git's output was also decoded strictly as UTF-8, so a commit with a Latin-1
subject lost the line that proved its claim, and rot fell to a lie.

**Decision.** A search returns found, not found, or failed. A failed search is never
cached, never becomes a finding at any tier, and makes the run incomplete with a
diagnostic counting the failures. Git's output is decoded lossily. `Git::try_search`
carries the distinction; its default wraps the infallible search, so a test double need
not care, while the subprocess implementation overrides it, and the gate and the cache
call only it.

**Consequences.** A git failure no longer changes what is reported, only what is known:
the claim is counted as history-failed, the run says it is incomplete, and the next run
searches again.

### ADR-0006 · Degraded runs are loud

**Context.** GitHub's checkout action fetches one commit by default, so a shallow clone
is the normal state of this tool's main environment. Without history nothing is rot,
Tier B is off, and a naive setup runs green on a rotting repository with nobody learning
why.

**Decision.** A degraded run announces itself. A banner on stderr, for every format,
says history is unavailable and names the fix verbatim — `fetch-depth: 0`, or checking
out without a filter for a partial clone. Under `--format github` it is also a warning
annotation, because stderr in an Action is a collapsed footnote. The run report's
status is `incomplete`. `--require-history` exits 2 instead, for CI that would rather
fail than run blind, and the bundled Action repairs the clone itself
([ADR-0009](#adr-0009--the-action-repairs-the-clone-and-annotates-by-default)). A
degraded run writes nothing to the history cache, because every answer it can give is
"never existed".

**Consequences.** A degraded run still reports nothing. Reporting Tier B when history is
missing was considered and rejected: buying coverage by loosening tiers is the failure
this project exists to avoid. The fix is to make the silence impossible to miss, not to
fill it with guesses.

## The history cache

History searches are the expensive part of a run, so their answers are cached. The
cache is acceleration and nothing else: a cached and an uncached run over the same
repository state must report the same findings with the same attribution.

It lives outside the repository, at
`<cache root>/<hash of the repository root>/history.json`. The cache root is
`STILLTRUE_CACHE_DIR` when that is set, and otherwise `stilltrue` in the platform cache
directory. The file is a versioned envelope: a schema number and an analysis version —
the tool's version plus a needle-format number — around entries keyed by needle and
scope. Each entry stores the HEAD it was computed at and the commit found, if any. The
commit keeps its Unix timestamp rather than a relative date, which is computed when a
finding is rendered; a stored "4 months ago" would be wrong forever.

A run uses the cache in one of three modes: read and write; rewrite, under `--no-cache`,
which ignores what is stored and replaces it, because a bad cache is the reason to reach
for that flag; and off, in a degraded run.

### ADR-0007 · The cache lives outside the repository

**Context.** A linter that writes a cache into the working tree dirties CI diffs and gets
cleaned away. Keying the whole cache on HEAD would discard it on every local commit,
which is exactly the pre-commit workflow where process spawns hurt most. But history
can be rewritten: after an amend, a rebase or a squash-merge, a stored commit may no
longer exist, and the stored commit *is* the attribution. A cached run once reported
"likely broke in" a SHA the reader could not find, with its pre-rewrite subject —
evidence that contradicted the history in front of them. Nothing told the reader it was
stale, so `--no-cache` was no remedy.

**Decision.**

- At the HEAD it was computed at, any stored answer is exactly what a search would say.
- A negative answer is trusted only at that HEAD, and searched again once HEAD moves.
- A positive answer survives HEAD moving only forward. If the stored HEAD is an ancestor
  of the analyzed one, the history it was computed over is intact, so the stored commit
  is still *an* answer. It may not be *the* answer: the search reports the latest
  matching commit, and a target restored and removed again since would be blamed on the
  second removal by an uncached run. So the new range, stored HEAD to HEAD, is searched
  too, and the later of the two commits is reported. After a rewrite, a branch switch or
  a collected object, everything is searched again. The ancestry check costs one
  `git merge-base --is-ancestor` per distinct stored HEAD, memoized for the run.
- A failed search is never stored
  ([ADR-0020](#adr-0020--a-failed-history-search-is-not-an-answer)), and a degraded run
  stores nothing.
- A file written by another schema or analysis version is discarded rather than read: a
  tool upgrade that changes what a needle means must not inherit answers to the old
  question.
- The file is written to a sibling and renamed into place, so an interrupted write
  leaves the old file or none. Two runs finishing together each replace the file whole;
  one loses the other's acceleration, never correctness. A run also writes what it has
  learned about once a second while it searches, so a scan that is killed — the agent
  adapter abandons one after five seconds — leaves the next one less to do.
- The cache bounds itself on every write: a repository's entry unused for thirty days
  is removed, and at most 256 are kept, least recently used first. Only directories of
  the cache's own shape are touched, so the adapters' session state beside them is left
  alone.

**Consequences.** The expensive direction is cached across commits; a repository whose
HEAD has not moved pays nothing. Every root path is a separate entry, and before the
bound, throwaway clones, fixtures and test checkouts had left over a hundred thousand
directories for the platform indexer to crawl; losing an entry now costs one slower run,
which is all a disposable cache may cost. The test configuration in
`.cargo/config.toml` points `STILLTRUE_CACHE_DIR` under the build directory, so tests
never touch a developer's own cache. `tests/cache_equivalence.rs` holds each rule to the
uncached reference: branch switching, reset, rebase, collected objects, a shallow clone
deepened, an interrupted write, a tool upgrade, an old layout, newer history, and
concurrent runs.

## Configuration and suppression

### The configuration file

Zero configuration is the intended way to run the tool. When a repository wants one,
stilltrue.toml is read from the repository root only — no cascade, no per-directory
override, no walk up from the working directory. Its absence is the common case. Every
key is optional:

```
include = ["README*", "docs/**"]   # replaces the default document set
exclude = ["docs/legacy/**"]       # extends the default excludes
strict  = false                    # also report Tier B, as --strict does

[ignore]
symbols = ["LegacyThing"]          # matched on the normalised name
env     = ["CI"]                   # matched literally
```

`include` **replaces** the defaults rather than extending them. A merging include would
offer no way to remove a default and becomes impossible to reason about. The cost is a
real footgun — an include of only the docs directory silently stops linting CLAUDE.md,
the tool's own wedge — paid deliberately for predictability, and called out in
`--help`. `exclude` extends the default excludes, so configuring one exclusion never
re-admits the changelogs.

`[ignore]` names a claim, not a spelling. A symbol is matched on its normalised name,
so `fold_events` covers the call form and `params` covers the dotted form; matching raw
text would make the example above impossible to write, since a bare name is never a
symbol claim ([ADR-0017](#adr-0017--a-symbol-claim-needs-a-syntactic-marker)). An exact
raw spelling still matches. An environment variable has no such spellings and is
matched literally.

An unreadable file, a syntax error or an unknown key makes the tool ignore the whole
file and use the defaults, with a diagnostic that marks the run incomplete: a
configuration that could not be used changes which documents were selected. An invalid
glob is dropped with the same kind of diagnostic.

### Suppression markers

An author can say a claim should not be judged:

- `<!-- stilltrue:ignore reason -->`, on a line of its own, suppresses every claim in the
  next block — a paragraph, a fence, a list item or a heading. Anchoring to the block
  rather than the line is what makes a claim inside a fence suppressible at all, since
  the line before it is inside the fence. The text after the marker is free and
  unparsed, so the reason lives beside it. Suppression is one block wide: a marker never
  swallows the rest of a section.
- `<!-- stilltrue:ignore-file -->`, anywhere in a document, suppresses the whole file.
- `exclude` globs suppress whole paths.

In reStructuredText the same markers are comments, `.. stilltrue:ignore` and
`.. stilltrue:ignore-file`, and a marker covers the next paragraph. Claims under a
marker are dropped before anything counts them, so the run report counts markers rather
than pretending to know how many claims they hid.

### ADR-0016 · An unused suppression is reported under `--strict`

**Context.** A suppression that no longer covers any claim was first left unreported,
for a good reason: it fires at the exact moment someone has just fixed their
documentation, the worst moment to hand them a new failure. But that argues about *when*
to report it, not whether it is worth knowing. A marker left behind is a hole in the
linting, and it is the one finding the tool can be certain about — no needle, no
history, no inference, only whether a claim sits in that block.

**Decision.** A marker that covered nothing is reported as Tier B, under `--strict`, with
the rule id `stilltrue/suppression/unused`, so it can be told apart and excluded on its
own. Default runs stay quiet. A whole-file marker is never reported: it is a statement
about a file, and a file with nothing to suppress is the normal case for a fixture or a
template.

**Consequences.** Tier B here is a reuse of the flag, not of the meaning — the finding is
provable, just not rot — and the rule id is what a reader should key on. The run report
counts these separately from lies.

## Baselines and autofix

Two features let a repository adopt the tool without starting red and repair findings
without retyping them. Both are deliberately narrow.

### Baselines

`--write-baseline FILE` records the current findings' fingerprints and exits 0.
`--baseline FILE` then stays silent on every finding already recorded, so new rot is
reported and the existing backlog is not. The two flags cannot be combined, and a
baseline is never written from a run that `--require-history` refuses. An unreadable
baseline suppresses nothing, with a diagnostic that marks the run incomplete.

### ADR-0014 · A baseline is a set of fingerprints

**Context.** A linter that starts red does not get adopted. A repository with years of
documentation has a backlog on its first run, and the only options were to fix all of it
before merging anything or to hand-write a suppression beside each finding.

**Decision.** A baseline is a set of finding fingerprints — the same fingerprint SARIF
carries: rule id, claim text, path, and an ordinal counting earlier occurrences of that
triple, hashed. **No line number goes in**, so reflowing a paragraph does not
un-suppress a document, and a baseline that churned on every edit would be one nobody
kept. The file is newline-delimited hashes under a comment header, sorted, so a reviewer
sees one line appear and understands what happened.

**Consequences.** A baseline matches by identity, not location: moving a claim to
another file or editing its text un-suppresses it, correctly, since the claim is then in
a document that never agreed to carry it. Nothing prunes the file; a fingerprint whose
finding is fixed stops matching and costs a line. Reporting stale entries would be a new
finding class with its own false-positive surface. A baseline is a migration tool, not
configuration: it says "we know about these", while `exclude` and suppression markers
say "do not judge this". Keeping them apart stops a baseline becoming the place rot goes
to die quietly.

### Autofix

`--fix` rewrites a finding's claim when there is exactly one thing it could have meant,
and `--fix-dry-run` prints the same rewrites as a unified diff on stderr without writing
anything. The two are mutually exclusive, share one plan, and exit on what was found,
never on what was changed.

### ADR-0015 · Autofix rewrites only what it can enumerate

**Context.** The first design declined autofix outright, for precision: a linter that
edits your documentation and gets it wrong is worse than one that says nothing, because
the damage is committed rather than displayed. That reason decides the shape of the
feature rather than forbidding it.

**Decision.** A claim is rewritten only when its suggestion list has exactly one entry.
A small candidate set is enumerated rather than guessed at (see
[Suggestions](#suggestions)), and a set of one is the only case where enumeration and
certainty coincide. The replacement is the suggestion exactly as the reader sees it, so
a command keeps its runner and a version its tool. Only the claim's byte span is
replaced, so a fix is a surgical edit, applied last span first because splicing forwards
invalidates every later offset. Every change is printed.

**Consequences.** Most findings are not fixable, and that is the intended ratio: a
deleted target with two plausible replacements is where a human has to choose. A fix
cannot be verified by the run that applied it; the next run re-reads the document, which
is the only honest confirmation. So `--fix` exits on what it found, and a CI job cannot
turn green by editing the repository. Nothing inside a suppressed block is rewritten,
because no finding is produced there.

### ADR-0022 · Fixes are planned, then applied

**Context.** `--fix` used to read, splice and write each document in one pass. There was
no way to see what it would do first, and nothing stopped it overwriting a document
someone edited while it ran.

**Decision.** A fix is a plan before it is a write. Planning reads each document once and
records, per edit, the file, the byte span, the text the span must still hold, the
replacement, and a hash of the document as read. An edit whose span no longer holds the
claim is rejected; overlapping edits are both rejected, because which was meant is a
human's call. `--fix-dry-run` renders the plan as a diff that `git apply` accepts.
`--fix` applies the same plan: a document whose contents no longer match the hash is
not written, and a written one is replaced atomically — a sibling renamed into place,
keeping the original's permissions — so a reader never sees half a rewrite. There is no
transaction across files. The run report lists every edit as eligible, applied or
rejected, with the reason.

**Consequences.** The preview is a promise. Tests apply it with `git apply` and compare
the result with what `--fix` writes, across non-ASCII spans, CRLF line endings, a missing
final newline and several hunks in one file. The diff goes to stderr because stdout
already carries the findings in whatever format was asked for.

## Output

### Findings and rule ids

A finding carries the claim (kind, text, file, position, span), its tier, a rule id, a
message, the commit that likely broke it (short SHA, subject, and a relative date
computed when rendered), and suggestions. Rule ids have the form
`stilltrue/<claim-type>/<tier>` — for example `stilltrue/command/rot` — and are a stable
public interface, because SARIF baselines and third-party suppression tooling key on
them.

### Formats

- **human** is the default: one finding per line, with the likely-breaking commit and
  any suggestions indented beneath, and a tally at the end. It is coloured only when
  stdout is a terminal and `NO_COLOR` is unset; the indent is measured on the unpainted
  text, since escape codes occupy no columns.
- **json** is an object holding the findings, each with rule id, tier, claim, kind,
  position, span, message, likely-breaking commit and suggestions.
- **sarif** is SARIF 2.1.0 for GitHub code scanning. Rot is `error` and a lie is
  `warning`, matching the fact that rot fails the build by default. Columns are declared
  as Unicode code points, URIs are percent-encoded relative references, and each result
  carries the baseline fingerprint as a partial fingerprint, so a reflow does not reopen
  every alert and two identical claims in one file are not merged as duplicates.
- **github** is workflow-command annotations, emitted by the binary so the Action needs
  no runtime. The escaping those commands need — `,` and `:` in a property, `%` and line
  breaks anywhere — happens once, in Rust; an unescaped comma in a path silently moves an
  annotation to another file.

No machine format is ever coloured.

```
CLAUDE.md:14:8  rot  command `make demo` has no target in Makefile
                     likely broke in a3f21c9 "split demo into seed+serve" · 4 months ago
                     did you mean: make seed, make serve?
```

### Suggestions

"Did you mean" is what makes a finding feel authored rather than generated, and a wrong
suggestion costs more trust than no suggestion buys. Candidates come only from the
resolver that failed and are never mixed across claim kinds.

- At five candidates or fewer, all of them are named, sorted. Listing the two targets a
  Makefile has is not a guess; it is telling the reader what exists. It is also the only
  way the example above is reachable: demo is edit distance 3 from seed and 4 from
  serve, and a threshold loose enough to reach them would suggest almost anything.
- Above five, edit distance decides: at most 2 for a name of up to eight characters,
  otherwise at most 30% of its length, and at most three are shown, by distance then
  alphabetically.
- Above 200 candidates, nothing is suggested; in a large symbol index, near matches are
  coincidence.

A suggestion is spelled the way the reader would have to type it: a command target with
its runner, a version number with its tool, an anchor with its target file, and a link
relative to its own document. This is not presentation. `--fix` writes exactly what was
offered, so a suggestion in the resolver's terms would be a rewrite that does not
resolve.

### Exit codes and flags

`--strict` controls what is reported; `--fail-on` controls what is fatal, and the two
are independent. With `rot`, the default, any Tier A finding exits 1; `any` exits 1 on
any reported finding, which differs only under `--strict`; `none` never exits 1, for
advisory runs. Exit 0 means nothing fatal, and exit 2 means an unrecoverable fault, a
requested output that could not be written, or — under `--require-history` — history
that is unavailable.

```
stilltrue [PATHS]...
  --strict                 also report Tier B (lie)
  --format human|json|sarif|github
  --fail-on rot|any|none   default: rot
  --no-cache               ignore the history cache, and rewrite it
  --require-history        exit 2 when history is unavailable
  --baseline FILE          stay silent on findings recorded in FILE
  --write-baseline FILE    record the current findings and exit 0
  --fix                    rewrite claims that have exactly one candidate
  --fix-dry-run            print those rewrites as a diff, and write nothing
  --summary                say what was examined and what was not, on stderr
  --report-file FILE       also write the versioned run report
  --sarif-file FILE        also write SARIF, from the same analysis
```

### ADR-0008 · Machine formats obey the same gate

**Context.** The obvious design is for SARIF and JSON to carry every finding with a tier
field and let downstream tooling filter. But SARIF's real consumer is GitHub code
scanning, which renders results straight into the pull request. Emitting Tier B there by
default would make the tool noisy at exactly the moment a user forms an opinion of it,
however quiet the terminal output.

**Decision.** There is one gate, and every format obeys it. `--strict` means the same
thing everywhere. Tier C is never serialized in any format under any flag: it is an
abstention, not a finding, and giving it a wire representation would invite someone to
surface it. The run report counts ambiguous claims by reason; it never lists them.

**Consequences.** Downstream tools cannot see what the gate withheld, by design. Rule ids
become a public interface that cannot change casually.

### ADR-0021 · One analysis, several outputs

**Context.** The Action used to scan twice when both annotations and SARIF were wanted,
and nothing guaranteed the two scans described the same state. Separately,
`--write-baseline` returned before `--require-history` was checked, so a baseline could
be recorded from a run that could not see history — a file of nothing but lies.

**Decision.** The analysis keeps everything it found, before any baseline. The baseline
is applied once, as presentation, and every output reads that one view: stdout in the
chosen format, `--sarif-file`, `--report-file` and `--summary`. Machine formats are
never appended to one stream. `--require-history` is checked before every success path,
baseline writing included. Failing to write an output that was explicitly asked for
exits 2, because a caller cannot tell a missing file from a clean run. The existing
JSON shape, SARIF fingerprints, rule ids and exit codes did not change; only the new
run report is versioned.

**Consequences.** The outputs cannot disagree. Fingerprints are computed over the whole
finding set, exactly as a baseline records them, so a shown finding carries the identity
a baseline would give it. The Action runs the binary once; its test counts invocations
through a shim and fails against a two-scan step.

## The run report

Every run can say what it examined and what it did not. `--summary` writes a short
account to stderr, and `--report-file FILE` writes the same view as a versioned JSON
envelope, so a tool or an agent reads one field instead of parsing prose.

**Status** is operational, and separate from coverage:

- **complete** — every selected operation succeeded;
- **incomplete** — a read, a parse, a walk or a history search failed, the configuration
  or baseline could not be used, or history was degraded;
- **no-input** — no document matched. Incomplete wins over no-input, because a failed
  selection cannot honestly say that nothing matched.

**Coverage** accounts for every claim exactly once — ignored by configuration, true,
broken, skipped or ambiguous — and every broken claim by exactly one gate outcome. Skips
and ambiguities are aggregated by claim kind and reason code. The counts reconcile at
every stage, and that is asserted over every fixture. A complete run can still leave a
great deal unjudged, so the report also names *coverage limitations*: symbol checking
unavailable, claims ambiguous or skipped, lies not reported without `--strict`, findings
suppressed by the baseline, suppression markers present. A limitation is an expected
abstention, never a failure.

**Diagnostics** are operational issues — an unreadable document, a manifest that did not
parse, failed history searches — each with a stable kebab-case code, and each either
making the run incomplete or not. A diagnostic is never a way to report an ambiguous
claim.

The envelope's `schema` is `stilltrue/run-report` and its `version` is 1, and a consumer
checks the version before reading anything else. It holds the tool, the status and exit
code, the run's HEAD, flags and stage timings, the history state and search counts,
coverage, limitations, diagnostics, the same post-baseline findings as stdout with their
fingerprints, and any planned edits. The summary ends in one sentence stating actual
counts and limitations; it never says the documentation is verified.

### ADR-0019 · A run says whether it was complete

**Context.** "No findings" was printed by a run that read every document, parsed every
manifest and searched history, and equally by a run that could not read half its
documents, whose manifests did not parse, in a shallow clone. The corpus runner counted a
crashed scan as a quiet repository for the same reason. A silence that cannot be told
apart from a failure is not evidence of anything.

**Decision.** A run states an operational status separate from its coverage, and
accounts for every claim, as described above. Reason codes are a public interface, like
rule ids. An expected abstention is named as a coverage limitation, not a failure. A
manifest that exists and does not parse is now skipped with a reason: it used to be read
as having no targets, so every target it named looked missing and its own history
promoted each to rot — a trailing comma reported every script. The
`manifest-unparseable` fixture is the near-miss for command rot.

**Consequences.** A run can be quiet and incomplete, and says so, including in the job
summary the Action writes. Operational failures still warn and never fail the build. The
accounting is built per document and merged after the parallel stage, so it costs no
lock per claim.

## The GitHub Action

`action.yml` is a composite Action with three steps: deepen the clone, install a pinned
prebuilt binary, and run the check once.

- **Inputs:** `version` (required, no default), `args`, `fail-on`, `strict`, `unshallow`
  (default true) and `sarif-file`. **Output:** `report-file`, the run report's path.
- **Deepen.** If the clone is shallow, `git fetch --unshallow`. A failure warns and
  continues, because errors never fail the build — the checker then says for itself that
  it cannot report rot.
- **Install.** The release archive for the runner's platform — Linux or macOS, x86-64 or
  arm64; any other runner exits 2 saying so — is downloaded from the repository the
  Action was referenced from. Its SHA-256 checksum, published beside every real release,
  is verified, and a missing checksum is itself a reason to stop, since the binary goes
  on the path. The step then checks that a binary actually arrived, because tar exits 0
  on an empty stream.
- **Run.** One invocation, with `--format github`, `--summary` and `--report-file`, and
  `--sarif-file` when asked. Stderr goes to the step log and the job summary, and the
  exit code stands.

`version` has no default because it names a published release, and a default pointing
at a release that does not exist fails with a download error instead of saying so; the
runner does not enforce `required`, so the step checks for itself. Release archives are
built by cargo-dist for five targets, Windows included; the Action has no Windows case,
and Windows users install from the release archive. No release has been published yet,
so setup examples show a placeholder version, and `tests/docs.rs` refuses any example
that pins an unverified release or a branch, or that runs the check only when a document
changed — a code change is what breaks an untouched document.

For local use, `.pre-commit-hooks.yaml` offers two pre-commit hooks, built from source
or using an installed binary. Both examine the whole repository on every commit, take no
file list, and run whether or not a document was staged. pre-commit checks the staged
snapshot, while a direct run reads the working tree.

### ADR-0009 · The Action repairs the clone and annotates by default

**Context.** A composite Action runs after checkout, so it cannot set `fetch-depth`, and
default CI is shallow. Without history the tool has nothing to report
([ADR-0006](#adr-0006--degraded-runs-are-loud)). Separately, SARIF upload needs the
`security-events: write` permission, which many repositories will not grant.

**Decision.** The Action repairs the clone with `git fetch --unshallow` unless
`unshallow` is false, so it works in default CI without anyone editing their checkout
step. Its default output is pull-request annotations through workflow commands, which
need no permissions. The binary renders them under `--format github`; assembling them in
the Action's shell would need a runtime in CI and would put the escaping in the wrong
place. SARIF stays available through `sarif-file`, which writes the file for a later
upload step rather than uploading it. The Action installs a pinned prebuilt binary,
never building from source, which would add a Rust toolchain build to every CI run.

**Consequences.** Working by default beats integrating maximally by default. A user who
wants code scanning will enable it; a user whose first run breaks will not come back.

## Agent adapters

`stilltrue-hook` runs the same checker at the moments an agent can still act on what it
says: when a session starts, and each time the agent finishes a turn. It speaks two
hosts' hook protocols — Claude Code (SessionStart, UserPromptSubmit, Stop) and Codex
(SessionStart, Stop, Interrupt) — with configurations in `integrations/claude-code/` and
`integrations/codex/hooks.json`. Installation, trust, limits and the host compatibility
matrix are in [the adapters' guide](agents.md); the design is below.

- **A child process, not a library call.** The adapter runs the checker in a process
  group of its own, with JSON output, no failing exit status, and a run report written
  to its state directory. A scan that overruns its budget — five seconds by default — is
  killed with everything it started, git processes included, and reported as
  unverified. The checker writes its history cache as it goes, so each abandoned scan
  leaves the next less to do.
- **Session state outside the repository.** One file per canonical worktree and host
  session, beside the history cache by default, written atomically, expired after seven
  days, at most 256 kept. It records the starting findings, whether that scan was
  complete, the tool and configuration identities, the branch and the latest scan.
- **A content identity, not an edit event.** A completion check is skipped only when the
  working tree is identical to the last complete check: a hash of HEAD and the content of
  every file the checker's walk would see, untracked files and ignore rules included
  (very large files by size and time). An edit event is not enough, because a shell
  command can change files without one. If files change while a scan runs, its result is
  stale and vouches for nothing.
- **Comparison.** Findings are matched by fingerprint. Present now and absent from the
  starting point is *newly observed* — never "introduced by the agent", since someone
  else may have changed the repository.
- **Bounded, quoted messages.** What the agent is told is at most 4 KiB, quotes claim
  text and commit subjects as data between markers with anything that could end the
  quotation removed, says how many findings were cut, and points to the full report.
  An unchanged message is not repeated.

### ADR-0023 · Agent adapters advise at session boundaries

**Context.** Agents read instruction files, and an agent following a stale one takes
wrong actions without anyone being told. A linter someone must remember to run finds
that afterwards. The hypothesis is that the same evidence, delivered while the agent can
still act on it, is worth more — and that is a hypothesis, not a result.

**Decision.** At session entry the adapter scans and records the findings as the
session's starting point, and tells the agent they predate the session: evidence, not a
task. At each completion it rescans every configured document — touched or not, because
a code change is what breaks an untouched one — and compares. Resume keeps a compatible
starting point, including findings newly observed before the restart. A new or cleared
session, or a change of tool, configuration or branch, starts a new comparison epoch,
records why, and carries what was unresolved forward as advice. A starting point is only
ever taken from a complete scan.

Advice is the default. Blocking is opt-in and asks for at most one continuation per user
turn, only for newly observed rot, and only when both scans were complete, the comparison
is valid, nothing changed during the scan, the host does not flag the stop as a
continuation, and the user did not interrupt. The next completion is then a verification
that reports the finding resolved, unresolved or unverified, and the turn ends.

The adapter always exits 0 and answers only in JSON a host documents. A Claude Code Stop
hook that exits 2 asks the agent to keep going, and 2 is also the checker's
unrecoverable-fault status, so no failure — of the checker, of the adapter's arguments,
or a panic — may surface as an exit status. The adapter never runs autofix, never runs a
command a document names, never edits the repository, and never deepens history over
the network.

**Consequences.** CI stays the independent gate; the adapter trusts nothing a hook did
and replaces none of it. Whether blocking improves an agent's work enough to justify
interrupting it is not known. The harness to measure it is in
[the benchmark](../bench/README.md), and no live run has been made, so blocking stays
opt-in. A host counts as tested only after a live run: replay tests of documented and
recorded payloads are necessary but not sufficient, and Codex has not been run live.

## How precision is verified

This repository was written to satisfy the tool, so it cannot falsify it. Precision is
therefore checked by several independent means, each catching what the others miss, and
a precision number is cited only if one of them produced it.

### Paired fixtures

History is part of the logic, so fixtures are real git repositories, each built by a
script under `tests/fixtures/` with exactly the commits its case needs, and rendered
output is snapshot-tested with insta. Every gating rule ships two cases: the true
positive it must report, and the near-miss it must stay silent on. The near-misses are
the precision regression suite; deleting one is a serious change. **Every new resolver,
runner or classification rule lands with its near-miss in the same change.**

| Fixture | What it holds |
|---|---|
| `command-rot`, `command-near-miss` | a deleted make target is rot; the target still present is silent |
| `path-lie` | a path never tracked is reported under `--strict`, and silent without it |
| `fence-near-miss` | a broken command in a shell fence is rot; a path in a TOML fence is silent |
| `placeholder-vs-real-path` | a real path that moved is rot; a placeholder beside it is silent |
| `history-cut-rename` | a target lost to a manifest rename is silent, not mislabelled rot |
| `symbol-rot`, `symbol-mixed-language` | a renamed Python function is rot; with Go present, silent |
| `envvar-doc-only` | a renamed variable is rot, although another document still names it |
| `version-pin-vs-range`, `version-edition` | prose against a pin is rot; ranges and editions are ignored |
| `ambiguous-manifest` | a qualified path that moved is rot; the bare filename is silent |
| `historical-record` | a moved path in a guide is rot; the same path in a changelog is silent |
| `manifest-unparseable` | a manifest that stopped parsing skips its targets instead of reporting them |
| `nested-worktree` | rot a nested worktree used to hide is reported; the worktree is not read |
| `degraded-shallow`, `degraded-partial` | a shallow or partial clone warns loudly and reports no rot |

CI also runs the built binary over this repository with `--require-history`, so any rot
in its own documents fails the build. That makes the self-referential rule a test, and
catches aggregate noise that isolated fixtures never will.

### The controlled-edit suite

`tests/effectiveness.rs` builds repositories from two families, makes one labelled edit
per case and commits it, and declares what the default run must say. The edits cover
the six kinds of in-scope rot — a deleted command target, a renamed path, a removed
symbol, a changed pin, a removed variable, a broken heading — alongside near-misses and
out-of-scope cases such as behaviour that changed behind an unchanged name, which show
where the tool's benefit stops. Each case also checks that its edit actually happened,
because a near-miss passes by being silent and an edit that did nothing would pass for
the wrong reason. One family is held out and was never used to tune a rule; when a
classification rule changes, fresh held-out cases are added rather than these edited.
This measures controlled-edit detection, not recall over arbitrary prose.

### The pinned corpus

`tests/corpus/pinned.py` clones fifteen third-party repositories at the commits in
`tests/corpus/pinned.tsv`, lints each, and compares the findings with reviewed labels in
`tests/corpus/labels.tsv`; an unlabelled finding is reported as unreviewed and counted
neither way. It records every repository's exit code, history state and run-report
status. A repository that could not be cloned, checked out or scanned is recorded as
failed, never as quiet, because a crashed scan also prints no findings, and the suite
reports its own status: incomplete if any scan failed or was incomplete.
`tests/corpus/run.sh` lints the same repositories at their moving heads, which makes it
exploratory rather than citable.

Most classification rules in this document came from the corpus. Over its history the
default run went from 317 findings to 6, five of which are confirmed true positives, and
each step down was a rule recorded here. The corpus is small and chosen by the authors:
it says the default run is quiet and mostly right on those fifteen, and nothing about
how much rot it misses.

### Mutation testing, fuzzing and product tests

- **Mutation testing** with cargo-mutants finds tests that pass without holding. It found
  that toml's string-parsing trait rejects documents its deserializer accepts, so every
  cargo alias looked missing and no Rust pin had ever been read; and that most surviving
  mutants lived in tests whose inputs tripped several rules at once. A new case should be
  the only thing standing between the claim and the wrong answer. Sweeps run with one
  job, scoped to what changed, and a sweep's own result is trusted only once it can be
  shown to have run — an unexpectedly good result is a reason to inspect the mechanism.
- **Fuzzing.** `tests/fuzz.rs` feeds both extractors pathological documents from a seeded
  generator, so any failure reproduces from its seed, and checks every claim's
  invariants — positions, spans within the document, non-empty text — and that
  extraction is deterministic.
- **Product tests.** `tests/product/run.sh` drives every shipped feature through the
  release binary against repositories with real history, as a user would; it found that
  `[ignore] symbols` could not be used as documented. `tests/action.py` runs the Action's
  literal shell steps against a real archive.
- **Cache equivalence.** `tests/cache_equivalence.rs`
  ([ADR-0007](#adr-0007--the-cache-lives-outside-the-repository)).
- **Adapters.** Process-level and replay tests hold both hosts' payloads to their output
  contracts, with recorded payloads in `tests/hook/payloads/`; the live-host test,
  `tests/product/hooks-live.sh`, decides whether a host counts as tested.
- **Performance.** `tests/perf/generate.py` builds the 500-document, 5,000-file tree the
  budget is defined on, and `tests/perf/measure.py` measures cold and warm runs and git
  process counts against it.

None of this establishes recall over arbitrary prose, anything about adoption, or
whether the agent adapters improve an agent's work. Those need measurements this
repository cannot make on its own.

## Not built

Recorded so that nobody mistakes them for features:

- **Findings introduced since a base revision.** A mode reporting only findings absent
  from a scan at the merge base was designed and not built. Baselines cover adoption
  meanwhile.
- **Package-scoped symbols.** Checking symbols per package in mixed-language
  repositories would change
  [ADR-0004](#adr-0004--symbol-claims-require-a-fully-supported-repository) and needs a
  design for ownership, history and near-misses of its own.
- **A third language resolver**, prose linting, external URLs, untagged fences and bare
  slash-separated references are out of scope by decision, not by omission.
- **Published releases.** Release packaging is configured and tested, but no release
  has been verified from its published archives; see
  [the release procedure](releasing.md).
