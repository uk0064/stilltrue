//! extract → resolve → gate → report, wired together.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;

use crate::cache::{self, Cached};
use crate::claim::{Claim, ClaimKind};
use crate::config::Config;
use crate::coverage::{Coverage, Diagnostic, HistoryStats, Status, Timings};
use crate::gate::{self, Finding, Judgement};
use crate::git::{Git, Subprocess};
use crate::repo::Repo;
use crate::report::Degraded;
use crate::resolution::{Evidence, Resolution};
use crate::{extract, resolve};

/// Git is a process spawn per check, so the history stage is bounded rather than run
/// on the ambient rayon pool.
const HISTORY_THREADS: usize = 8;

pub struct Options {
    pub root: PathBuf,
    /// Where the user ran the command, which is what positional paths are relative to.
    pub cwd: PathBuf,
    /// Positional paths *filter* the resolved document set rather than replacing it.
    /// Given as the user typed them, relative to `cwd`.
    pub paths: Vec<PathBuf>,
    pub strict: bool,
    pub cache: bool,
}

pub struct Outcome {
    /// Every finding the gate reported, before any baseline is applied. The baseline is
    /// presentation (ADR-0021): the analysis keeps what it found.
    pub findings: Vec<Finding>,
    pub degraded: Degraded,
    /// How many documents were actually examined. Zero is worth saying out loud: a
    /// clean bill of health over a repository nothing was read from is the worst
    /// failure this tool has, and it is what a mistyped path or a too-narrow `include`
    /// produces.
    pub documents: usize,
    /// Where every extracted claim went (ADR-0019).
    pub coverage: Coverage,
    /// Operational issues, sorted by path and code.
    pub diagnostics: Vec<Diagnostic>,
    pub history: HistoryStats,
    pub timings: Timings,
    /// The HEAD this run analyzed.
    pub head: Option<String>,
    /// Whether Tier B was reported: the flag or the config, whichever said so.
    pub strict: bool,
}

impl Outcome {
    /// Complete, incomplete, or no-input — readable without parsing any prose.
    pub fn status(&self) -> Status {
        Status::of(
            &self.diagnostics,
            !self.degraded.is_degraded(),
            self.documents,
        )
    }
}

/// One document's contribution. Built without touching shared state and merged after
/// the parallel stage, so no claim ever waits on a lock and no true claim outlives the
/// document it was read from.
#[derive(Default)]
struct Tally {
    coverage: Coverage,
    broken: Vec<(Claim, Evidence)>,
    unused: Vec<Finding>,
    diagnostics: Vec<Diagnostic>,
}

impl Tally {
    fn merge(mut self, other: Tally) -> Tally {
        self.coverage.merge(other.coverage);
        self.broken.extend(other.broken);
        self.unused.extend(other.unused);
        self.diagnostics.extend(other.diagnostics);
        self
    }
}

pub fn run(options: &Options) -> Outcome {
    let started = Instant::now();
    let repo = Repo::new(&options.root);
    let (config, mut diagnostics) = Config::load_reporting(&options.root);
    let strict = options.strict || config.strict;

    let filter = (!options.paths.is_empty()).then(|| {
        rebase(
            &options.paths,
            &options.cwd,
            &options.root,
            &mut diagnostics,
        )
    });
    let documents = documents(&repo, &config, filter.as_deref());
    for error in repo.walk_errors() {
        eprintln!("stilltrue: warning: could not walk part of the repository: {error}");
        diagnostics.push(Diagnostic::failure("walk-error", error.clone(), None));
    }
    if documents.is_empty() {
        eprintln!(
            "stilltrue: warning: no documents matched; nothing was examined. \
             Check the paths you passed, or `include` in stilltrue.toml."
        );
        diagnostics.push(Diagnostic::note(
            "no-documents",
            "no documents matched; nothing was examined",
            None,
        ));
    }
    let select_ms = millis(started);

    let git = Subprocess::new(&options.root);
    let degraded = if !git.available() {
        Degraded::NoRepository
    } else if git.shallow() {
        Degraded::ShallowClone
    } else if git.partial() {
        Degraded::PartialClone
    } else {
        Degraded::No
    };
    if degraded.is_degraded() {
        diagnostics.push(Diagnostic::failure(
            "history-unavailable",
            format!(
                "history unavailable ({}); nothing can be reported as rot",
                degraded.slug()
            ),
            None,
        ));
    }
    let mode = if degraded.is_degraded() {
        // Every answer this run can give is "no history found". Storing those would
        // poison the cache for every later run in a repaired clone.
        cache::Mode::Off
    } else if options.cache {
        cache::Mode::ReadWrite
    } else {
        cache::Mode::Rewrite
    };
    let git = Cached::new(git, &options.root, mode);

    // extract and resolve run in parallel across documents; the git-touching stage
    // does not.
    let resolving = Instant::now();
    let tally = documents
        .par_iter()
        .map(|document| tally_document(document, &repo, &config, strict))
        .reduce(Tally::default, Tally::merge);
    let extract_resolve_ms = millis(resolving);

    // History checks run in a bounded pool: each is a process spawn rather than
    // a cheap task, and unbounded rayon over process spawns is a fork bomb on a large
    // repository. Order does not matter — findings are sorted below.
    let searching = Instant::now();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(HISTORY_THREADS))
        .unwrap_or(1);
    let broken = &tally.broken;
    let judge = || -> Vec<(ClaimKind, Judgement)> {
        broken
            .par_iter()
            .map(|(claim, evidence)| {
                (
                    claim.kind.clone(),
                    gate::judge(claim, evidence, &git, strict),
                )
            })
            .collect()
    };
    let judged = match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
        Ok(pool) => pool.install(judge),
        // A pool we could not build is not a reason to fail; fall back to the ambient
        // one, which is still correct, just unbounded.
        Err(error) => {
            eprintln!("stilltrue: warning: using the default thread pool: {error}");
            judge()
        }
    };
    let history_ms = millis(searching);

    git.persist();

    let mut coverage = tally.coverage;
    let mut findings: Vec<Finding> = Vec::new();
    for (kind, judgement) in judged {
        coverage.judged(&kind, &judgement);
        if let Judgement::Reported(finding) = judgement {
            findings.push(*finding);
        }
    }
    let history = HistoryStats {
        searches: broken.iter().filter(|(_, e)| e.needle.is_some()).count(),
        failed_searches: coverage.gate.history_failed,
        git_processes: git.processes(),
    };
    if history.failed_searches > 0 {
        diagnostics.push(Diagnostic::failure(
            "history-search-failed",
            format!(
                "{} history searches failed; those claims were not judged",
                history.failed_searches
            ),
            None,
        ));
    }

    // ADR-0016: a marker covering nothing is worth saying, but only under --strict.
    findings.extend(tally.unused);
    diagnostics.extend(tally.diagnostics);
    diagnostics.sort_by(|a, b| (&a.path, a.code, &a.message).cmp(&(&b.path, b.code, &b.message)));
    diagnostics.dedup();

    // Determinism comes from one rule: sort before report.
    findings.sort_by(|a, b| {
        (&a.claim.file, a.claim.line, a.claim.column, &a.rule_id).cmp(&(
            &b.claim.file,
            b.claim.line,
            b.claim.column,
            &b.rule_id,
        ))
    });

    let head = git.head().map(str::to_string);
    Outcome {
        findings,
        degraded,
        documents: documents.len(),
        coverage,
        diagnostics,
        history,
        timings: Timings {
            select_ms,
            extract_resolve_ms,
            history_ms,
            total_ms: millis(started),
        },
        head,
        strict,
    }
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Extract and resolve one document, counting where every claim went.
fn tally_document(document: &Path, repo: &Repo, config: &Config, strict: bool) -> Tally {
    let mut tally = Tally::default();
    tally.coverage.documents_matched = 1;
    // Unreadable input warns and skips that unit. Skipping in silence would let a
    // permission error or a non-UTF-8 file read as a clean document, which is the same
    // lie as a clean bill of health.
    let source = match std::fs::read_to_string(repo.root().join(document)) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "stilltrue: warning: skipping {}: {error}",
                document.display()
            );
            tally.coverage.documents_unreadable = 1;
            tally.diagnostics.push(Diagnostic::failure(
                "document-unreadable",
                format!("could not read this document: {error}"),
                Some(document.to_path_buf()),
            ));
            return tally;
        }
    };
    tally.coverage.documents_read = 1;

    // One claim model, one gate; only the reader differs.
    let extraction = if document.extension().and_then(|e| e.to_str()) == Some("rst") {
        crate::rst::extract(&source, document)
    } else {
        extract::extract(&source, document)
    };
    if extraction.unparseable {
        eprintln!(
            "stilltrue: warning: could not parse {}; no claims were read from it",
            document.display()
        );
        tally.coverage.documents_unparseable = 1;
        tally.diagnostics.push(Diagnostic::failure(
            "document-unparseable",
            "the parser produced nothing; no claims were read",
            Some(document.to_path_buf()),
        ));
    }
    tally.coverage.suppression_markers = extraction.markers;
    tally.coverage.suppression_unused = extraction.unused_suppressions.len();
    tally.coverage.files_suppressed = usize::from(extraction.whole_file);
    if strict {
        for marker in &extraction.unused_suppressions {
            tally.unused.push(unused_suppression(document, marker));
        }
    }

    // A manifest that could not be used is reported once per document and reason, not
    // once per claim that named it.
    let mut failed: BTreeMap<(&'static str, &'static str), usize> = BTreeMap::new();
    for claim in extraction.claims {
        tally.coverage.extracted(&claim.kind);
        if ignored(&claim, config) {
            tally.coverage.ignored(&claim.kind);
            continue;
        }
        let resolution = resolve::resolve(&claim, repo);
        tally.coverage.resolved(&claim.kind, &resolution);
        match resolution {
            Resolution::Broken(evidence) => tally.broken.push((claim, evidence)),
            Resolution::Skip(reason) if reason.is_failure() => {
                *failed
                    .entry((claim.kind.slug(), reason.code()))
                    .or_default() += 1;
            }
            Resolution::True | Resolution::Skip(_) | Resolution::Ambiguous(_) => {}
        }
    }
    for ((kind, code), count) in failed {
        eprintln!(
            "stilltrue: warning: {}: {count} {kind} claim(s) not checked ({code})",
            document.display()
        );
        tally.diagnostics.push(Diagnostic::failure(
            code,
            format!("{count} {kind} claim(s) in this document were not checked"),
            Some(document.to_path_buf()),
        ));
    }
    tally
}

fn ignored(claim: &crate::claim::Claim, config: &Config) -> bool {
    match claim.kind {
        // A symbol is named by its name. The resolver judges the normalised name —
        // `fold_events` for `fold_events()`, `params` for the `~Context.params` Sphinx
        // writes — so that is what `[ignore]` has to accept, or the documented example
        // of a bare `LegacyThing` could never match anything: ADR-0017 means a bare
        // name is not a symbol claim at all, so the raw text always carries a marker.
        // The raw spelling still matches, so an exact-text config keeps working.
        ClaimKind::Symbol => {
            let name = crate::resolve::lang::normalise_symbol(&claim.text);
            config
                .ignore
                .symbols
                .iter()
                .any(|ignored| ignored == name || *ignored == claim.text)
        }
        ClaimKind::EnvVar => config.ignore.env.contains(&claim.text),
        _ => false,
    }
}

/// Turn positional paths into repository-relative ones. Without this, an absolute path
/// or one typed from a subdirectory matches no document and the run exits 0 having
/// linted nothing — a clean bill of health over a rotting repository.
fn rebase(
    paths: &[PathBuf],
    cwd: &Path,
    root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PathBuf> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    paths
        .iter()
        .filter_map(|path| {
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            };
            let absolute = absolute.canonicalize().unwrap_or(absolute);
            match absolute.strip_prefix(&root) {
                Ok(relative) => Some(relative.to_path_buf()),
                Err(_) => {
                    eprintln!(
                        "stilltrue: warning: `{}` is outside the repository; ignoring it",
                        path.display()
                    );
                    diagnostics.push(Diagnostic::note(
                        "path-outside-repository",
                        format!(
                            "`{}` is outside the repository and was ignored",
                            path.display()
                        ),
                        None,
                    ));
                    None
                }
            }
        })
        .collect()
}

/// The document set: include globs, minus exclude globs, filtered by any positional
/// paths. `None` means no positional filter was given; an empty slice means every one
/// given fell outside the repository, which filters everything out.
fn documents(repo: &Repo, config: &Config, paths: Option<&[PathBuf]>) -> Vec<PathBuf> {
    let include = config.include_set();
    let exclude = config.exclude_set();

    repo.files()
        .iter()
        .filter(|path| is_document(path))
        .filter(|path| include.is_match(path))
        .filter(|path| !exclude.is_match(path))
        .filter(|path| match paths {
            None => true,
            Some(paths) => paths
                .iter()
                .any(|p| path.starts_with(p) || p.as_path() == path.as_path()),
        })
        .cloned()
        .collect()
}

/// Formats a document can be written in. reStructuredText is here because a whole
/// documentation ecosystem never adopted Markdown, and a repository whose docs are
/// `.rst` got nothing at all from this tool — pytest yielded zero documents.
fn is_document(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "markdown" | "mdc" | "mdx" | "rst")
    )
}

/// A marker that covered nothing (ADR-0016). Tier B, so default runs stay quiet.
fn unused_suppression(document: &Path, marker: &extract::UnusedSuppression) -> Finding {
    Finding {
        claim: crate::claim::Claim {
            kind: ClaimKind::Suppression,
            text: "stilltrue:ignore".to_string(),
            file: document.to_path_buf(),
            line: marker.line,
            column: marker.column,
            end_line: marker.line,
            end_column: marker.column + "<!-- stilltrue:ignore -->".len(),
            span: 0..0,
        },
        tier: gate::Tier::Lie,
        rule_id: "stilltrue/suppression/unused".to_string(),
        message: "suppression covers no claim".to_string(),
        breaking_commit: None,
        suggestions: Vec::new(),
    }
}
