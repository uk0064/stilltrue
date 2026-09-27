//! Git as a subprocess behind a trait. Git is present in every CI image, and the trait
//! keeps a linked library available as a later optimisation.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::resolution::Needle;

/// A commit that changed the needle's occurrence count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    /// Commit time as a Unix timestamp. Stored rather than git's own `%cr` string,
    /// because a cached positive result never expires and would otherwise keep
    /// reporting "4 months ago" forever.
    pub timestamp: i64,
}

/// A history search that did not run to completion: git could not be started, or it
/// exited non-zero.
///
/// Kept apart from "found nothing" because the two answers mean opposite things. An
/// empty search is evidence that a claim never existed; a failed one is no evidence at
/// all, and treating it as the first made it a Tier B lie under `--strict` and a
/// cached absence for every later run (ADR-0020).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryError;

pub trait Git: Sync {
    /// Whether history can be searched at all.
    fn available(&self) -> bool;

    /// Whether the clone is shallow, which makes any search unreliable.
    fn shallow(&self) -> bool;

    /// Whether the clone is partial (`--filter=blob:none` and friends). Not shallow —
    /// every commit is present — but blobs are fetched on demand, so a pickaxe reads
    /// an incomplete tree and answers "never existed" for things that did.
    fn partial(&self) -> bool {
        false
    }

    /// The most recent commit matching the needle, or `None` if history has never
    /// known it.
    fn search(&self, needle: &Needle) -> Option<Commit>;

    /// The most recent matching commit among those reachable from HEAD but not from
    /// `since` — the history added since the cache last looked.
    ///
    /// The default searches everything, which is correct and merely slower: the caller
    /// keeps whichever of this answer and the stored one is more recent.
    fn search_since(&self, needle: &Needle, _since: &str) -> Result<Option<Commit>, HistoryError> {
        self.try_search(needle)
    }

    /// The same search, able to say that it failed rather than that it found nothing.
    ///
    /// The default cannot fail, which is right for an implementation that never runs
    /// anything; `Subprocess` overrides it, and `gate` and the cache only ever call
    /// this one.
    fn try_search(&self, needle: &Needle) -> Result<Option<Commit>, HistoryError> {
        Ok(self.search(needle))
    }

    /// Whether `sha` is reachable from the analyzed HEAD.
    ///
    /// Only the cache asks. A stored positive names the commit it will be reported
    /// against, and a rewrite — an amend, a rebase, a squash-merge — can leave that
    /// commit out of the repository's history while the claim it proved is still
    /// broken. Reporting a SHA the reader cannot find is worse than recomputing.
    ///
    /// The default is `false`: a `Git` that cannot answer must not be taken to have
    /// said yes, because the caller reuses stale attribution on a yes.
    fn contains(&self, _sha: &str) -> bool {
        false
    }

    /// How many git processes this has spawned. The run report records it, because a
    /// process spawn per check is the cost the history stage is bounded for.
    fn processes(&self) -> usize {
        0
    }
}

/// Git invoked as a subprocess.
pub struct Subprocess {
    root: PathBuf,
    available: bool,
    shallow: bool,
    partial: bool,
    spawned: AtomicUsize,
}

impl Subprocess {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let spawned = AtomicUsize::new(0);
        let available = probe(&spawned, &root, &["rev-parse", "--git-dir"]).is_some();
        let shallow = probe(&spawned, &root, &["rev-parse", "--is-shallow-repository"])
            .is_some_and(|out| out.trim() == "true");
        // `git config --list` exits 0 whether or not anything matches, unlike
        // `--get-regexp`, which exits 1 and would warn on every ordinary repository.
        let partial = probe(&spawned, &root, &["config", "--list"]).is_some_and(|out| {
            out.lines().any(|line| {
                line.contains(".promisor=true") || line.contains(".partialclonefilter=")
            })
        });
        Self {
            root,
            available,
            shallow,
            partial,
            spawned,
        }
    }
}

impl Git for Subprocess {
    fn available(&self) -> bool {
        self.available
    }

    fn partial(&self) -> bool {
        self.partial
    }

    fn shallow(&self) -> bool {
        self.shallow
    }

    fn processes(&self) -> usize {
        self.spawned.load(Ordering::Relaxed)
    }

    fn contains(&self, sha: &str) -> bool {
        // A shallow clone holds a truncated history, so "not an ancestor" there means
        // "not in the part I was given" — which is not the question. Say no and let the
        // caller recompute rather than trust an answer this clone cannot give.
        if !self.available || self.shallow {
            return false;
        }
        probe_status(
            &self.spawned,
            &self.root,
            &["merge-base", "--is-ancestor", sha, "HEAD"],
        )
    }

    fn search(&self, needle: &Needle) -> Option<Commit> {
        self.try_search(needle).ok().flatten()
    }

    fn try_search(&self, needle: &Needle) -> Result<Option<Commit>, HistoryError> {
        self.log(needle, None)
    }

    fn search_since(&self, needle: &Needle, since: &str) -> Result<Option<Commit>, HistoryError> {
        self.log(needle, Some(since))
    }
}

impl Subprocess {
    /// `git log -1` for a needle, over HEAD's history or only the part of it `since`
    /// does not reach.
    fn log(&self, needle: &Needle, since: Option<&str>) -> Result<Option<Commit>, HistoryError> {
        // Degraded history is not a failure of this search: the run already says, loudly,
        // that nothing can be rot (ADR-0006).
        if !self.available || self.shallow || self.partial {
            return Ok(None);
        }
        let range = since.map(|since| format!("{since}..HEAD"));
        const FORMAT: &str = "--pretty=format:%h%x1f%s%x1f%ct";
        let output = match needle {
            Needle::Regex { pattern, scope }
            | Needle::Literal {
                text: pattern,
                scope,
            } => {
                // HEAD's history only. Searching every ref would let an abandoned
                // branch prove a claim, and would make the answer depend on which refs
                // a CI checkout happened to fetch. It is also what made a 172-ref
                // repository take three minutes.
                let mut args = vec![
                    "log".to_string(),
                    "-1".to_string(),
                    format!("-S{pattern}"),
                    FORMAT.to_string(),
                ];
                if matches!(needle, Needle::Regex { .. }) {
                    args.insert(2, "--pickaxe-regex".to_string());
                }
                args.extend(range.clone());
                if !scope.is_empty() {
                    args.push("--".to_string());
                    args.extend(scope.iter().cloned());
                }
                let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
                git(&self.spawned, &self.root, &borrowed).ok_or(HistoryError)?
            }
            Needle::Heading { slug, path } => {
                let pattern = format!("-S{}", Needle::heading_pattern(slug));
                let literal = format!(":(literal){}", path.display());
                // A slug is lowercased; the heading it came from was not.
                let mut args = vec!["log", "-i", "--pickaxe-regex", "-1", &pattern, FORMAT];
                args.extend(range.as_deref());
                args.extend(["--", &literal]);
                git(&self.spawned, &self.root, &args).ok_or(HistoryError)?
            }
            Needle::Path { path } => {
                // `:(literal)` stops git reading the path as a pathspec: without it a
                // `*` in a claim silently becomes a wildcard and manufactures history.
                let literal = format!(":(literal){}", path.display());
                let mut args = vec!["log", "--full-history", "-1", FORMAT];
                args.extend(range.as_deref());
                args.extend(["--", &literal]);
                git(&self.spawned, &self.root, &args).ok_or(HistoryError)?
            }
        };

        Ok(parse_commit(&output))
    }
}

/// The first `%h\x1f%s\x1f%ct` line of `git log`, if there is one.
fn parse_commit(output: &str) -> Option<Commit> {
    let line = output.lines().next()?;
    let mut fields = line.split('\u{1f}');
    Some(Commit {
        sha: fields.next()?.to_string(),
        subject: fields.next().unwrap_or_default().to_string(),
        timestamp: fields.next().and_then(|t| t.parse().ok()).unwrap_or(0),
    })
}

/// Ask git a question whose answer may legitimately be "no".
///
/// `git` below warns on failure, because a failed search is indistinguishable from an
/// empty one. These are capability probes — "is this a repository?" — and a non-zero
/// exit is the answer, not a fault. Warning here would put two `fatal: not a git
/// repository` lines above the banner that already says exactly that.
/// Run git for its exit status alone. `merge-base --is-ancestor` answers by exiting
/// 0 or 1 and prints nothing, so `probe` — which reads stdout — cannot tell its two
/// answers apart. A git that could not run at all is a `false`, never a yes.
fn probe_status(spawned: &AtomicUsize, root: &Path, args: &[&str]) -> bool {
    spawned.fetch_add(1, Ordering::Relaxed);
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

fn probe(spawned: &AtomicUsize, root: &Path, args: &[&str]) -> Option<String> {
    spawned.fetch_add(1, Ordering::Relaxed);
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run git, returning its stdout. A git failure is never fatal — but it is
/// never silent either.
///
/// `None` from a pickaxe means "history does not know this needle", which downgrades a
/// finding from rot to a lie and so makes it invisible by default. A git that failed
/// returns the same `None`, so without a warning a broken git quietly turns every
/// Tier A finding in the repository into silence.
fn git(spawned: &AtomicUsize, root: &Path, args: &[&str]) -> Option<String> {
    spawned.fetch_add(1, Ordering::Relaxed);
    let output = match Command::new("git").arg("-C").arg(root).args(args).output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("stilltrue: warning: could not run git: {error}");
            return None;
        }
    };
    if !output.status.success() {
        // `git log -S` finding nothing exits 0 with empty output, so a non-zero status
        // is a real failure rather than an empty answer.
        eprintln!(
            "stilltrue: warning: git {} failed: {}",
            args.first().copied().unwrap_or("?"),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    // Lossy, not strict: a commit subject in Latin-1 is still a commit. Rejecting the
    // whole output turned a found needle into "never existed".
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}
