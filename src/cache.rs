//! History results, cached outside the repository (ADR-0007).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git::{Commit, Git, HistoryError};
use crate::resolution::Needle;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    /// `None` means history has never known this needle.
    found: Option<StoredCommit>,
    /// The HEAD this answer was computed at. A negative is trusted only there; a
    /// positive is reused past it only when that HEAD is an ancestor of the one being
    /// analyzed, and then only after the history added since has been searched.
    head: Option<String>,
}

/// Bumped whenever the stored layout changes.
const SCHEMA: u32 = 2;

/// Bumped whenever the meaning of a stored answer could change without its key
/// changing — a needle's spelling, the search's range or flags. A cache written under
/// any other analysis is discarded, not trusted: the cache is acceleration, and a run
/// with an empty one is merely slower.
const NEEDLE_FORMAT: u32 = 1;

/// Which tool and which analysis wrote a cache file.
pub fn analysis_version() -> String {
    format!("{}/{NEEDLE_FORMAT}", env!("CARGO_PKG_VERSION"))
}

/// The file on disk: a versioned envelope around the entries.
#[derive(Debug, Serialize, Deserialize)]
struct Store {
    schema: u32,
    analysis: String,
    entries: BTreeMap<String, Entry>,
}

/// Read a cache file, returning nothing unless it was written by this schema and this
/// analysis. A truncated file, an older layout and a newer tool's file all read as
/// empty — never as a partial answer.
fn read_store(path: &Path) -> BTreeMap<String, Entry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Store>(&text).ok())
        .filter(|store| store.schema == SCHEMA && store.analysis == analysis_version())
        .map(|store| store.entries)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCommit {
    sha: String,
    subject: String,
    timestamp: i64,
}

/// A `Git` that remembers. Wrapping rather than threading a cache through `gate` keeps
/// the gating logic unaware that caching exists.
pub struct Cached<G: Git> {
    inner: G,
    head: Option<String>,
    path: Option<PathBuf>,
    entries: std::sync::Mutex<BTreeMap<String, Entry>>,
    /// Ancestry answers for this run. Several claims routinely resolve to the same
    /// commit, and without this each would spawn its own `git merge-base`.
    reachable: std::sync::Mutex<BTreeMap<String, bool>>,
    /// When the entries were last written out during the run.
    saved: std::sync::Mutex<std::time::Instant>,
}

/// How often a run writes what it has learned so far, while it is still searching.
///
/// A run that is killed — the hook adapter abandons a scan after five seconds — used to
/// lose every search it had finished, because the cache was written only at the end.
/// On a large repository with a cold cache that meant every scan was abandoned at the
/// same point and none ever got further: codex, 8,650 files, never finished once.
/// Written as it goes, each abandoned scan leaves the next one less to do.
pub const CHECKPOINT: std::time::Duration = std::time::Duration::from_secs(1);

/// What a run is allowed to do with the stored cache.
///
/// Three modes, not two. `--no-cache` bypasses *and rewrites*: it ignores what
/// is stored and replaces it, which is what makes it the way to repair a bad cache. A
/// run that could not see history is different again — its every answer is "no history
/// found", and writing those would poison the cache for every later run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Read stored results, write new ones.
    ReadWrite,
    /// Ignore stored results, write new ones.
    Rewrite,
    /// Neither. This run's answers are not worth keeping.
    Off,
}

impl<G: Git> Cached<G> {
    pub fn new(inner: G, root: &Path, mode: Mode) -> Self {
        let path = (mode != Mode::Off).then(|| cache_path(root)).flatten();
        let entries = match (&path, mode) {
            (Some(path), Mode::ReadWrite) => read_store(path),
            _ => BTreeMap::new(),
        };
        let head = head_of(root);
        Self {
            inner,
            head,
            path,
            entries: std::sync::Mutex::new(entries),
            reachable: std::sync::Mutex::new(BTreeMap::new()),
            saved: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }

    /// Whether `sha` is still in the analyzed history, asked once per distinct commit.
    fn reachable(&self, sha: &str) -> bool {
        if let Ok(memo) = self.reachable.lock()
            && let Some(known) = memo.get(sha)
        {
            return *known;
        }
        let answer = self.inner.contains(sha);
        if let Ok(mut memo) = self.reachable.lock() {
            memo.insert(sha.to_string(), answer);
        }
        answer
    }

    /// The HEAD this run analyzed, if there is one.
    pub fn head(&self) -> Option<&str> {
        self.head.as_deref()
    }

    /// Unwrap back to the `Git` underneath.
    pub fn into_inner(self) -> G {
        self.inner
    }

    /// Write the cache out if it has not been written for `CHECKPOINT`. Only one
    /// thread writes at a time; the others carry on searching.
    fn checkpoint(&self) {
        let Ok(mut saved) = self.saved.try_lock() else {
            return;
        };
        if saved.elapsed() < CHECKPOINT {
            return;
        }
        self.persist();
        *saved = std::time::Instant::now();
    }

    /// Write the cache back. Failure is never fatal.
    ///
    /// Written to a sibling and renamed into place, so a reader sees the old file or
    /// the new one and never half of either. Two runs finishing together each replace
    /// the file whole; one's entries are lost, which costs the next run some
    /// acceleration and never a wrong answer.
    pub fn persist(&self) {
        let Some(path) = &self.path else { return };
        let Ok(entries) = self.entries.lock() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let store = Store {
            schema: SCHEMA,
            analysis: analysis_version(),
            entries: entries.clone(),
        };
        let Ok(json) = serde_json::to_string(&store) else {
            return;
        };
        let temporary = path.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        if std::fs::write(&temporary, json).is_err() || std::fs::rename(&temporary, path).is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        if let Some(base) = path.parent().and_then(Path::parent) {
            prune(base);
        }
    }
}

/// What a stored entry is worth at the HEAD being analyzed.
enum Reuse {
    /// This is the answer an uncached search would give.
    Answer(Option<Commit>),
    /// Nothing stored can be trusted; search from scratch.
    Search,
}

impl<G: Git> Cached<G> {
    fn reuse(&self, needle: &Needle, entry: &Entry) -> Reuse {
        let stored = entry.found.as_ref().map(|c| Commit {
            sha: c.sha.clone(),
            subject: c.subject.clone(),
            timestamp: c.timestamp,
        });
        // At the HEAD it was computed at, an answer is exactly what a search would say.
        if entry.head == self.head && self.head.is_some() {
            return Reuse::Answer(stored);
        }
        // A negative is only trustworthy at the HEAD it was computed at.
        let Some(stored) = stored else {
            return Reuse::Search;
        };
        // A positive survives HEAD moving only forward (ADR-0007, amended). If the
        // HEAD it was found at is an ancestor of this one, everything reachable from
        // there still is — the stored commit included — so it is still a true answer
        // about the older history. But it names the *latest* matching commit, and the
        // history added since may hold a later one: re-adding a target and removing it
        // again moves the blame. So that range is searched, and the later of the two
        // wins. A rewrite, a branch switch, or a stored HEAD that is gone: search again.
        let Some(since) = entry.head.as_deref() else {
            return Reuse::Search;
        };
        if !self.reachable(since) {
            return Reuse::Search;
        }
        // A range search that failed proves nothing either way; search it all.
        let Ok(newer) = self.inner.search_since(needle, since) else {
            return Reuse::Search;
        };
        Reuse::Answer(Some(match newer {
            Some(newer) if newer.timestamp >= stored.timestamp => newer,
            _ => stored,
        }))
    }
}

impl<G: Git> Git for Cached<G> {
    fn available(&self) -> bool {
        self.inner.available()
    }

    fn shallow(&self) -> bool {
        self.inner.shallow()
    }

    fn partial(&self) -> bool {
        self.inner.partial()
    }

    /// The wrapped `Git`'s processes, plus the `rev-parse HEAD` this spawned itself.
    fn processes(&self) -> usize {
        1 + self.inner.processes()
    }

    fn search(&self, needle: &Needle) -> Option<Commit> {
        self.try_search(needle).ok().flatten()
    }

    fn try_search(&self, needle: &Needle) -> Result<Option<Commit>, HistoryError> {
        let key = key_of(needle);

        let stored = self
            .entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(&key).cloned());
        let found = match stored {
            // A reused answer is stored again at this HEAD, which changes nothing when
            // HEAD has not moved and records the refreshed attribution when it has.
            Some(entry) => match self.reuse(needle, &entry) {
                Reuse::Answer(answer) => answer,
                Reuse::Search => self.inner.try_search(needle)?,
            },
            // A search that failed is not stored. Storing it would record "never
            // existed" for a needle nobody looked for, and every later run at this HEAD
            // would read that as proof (ADR-0020).
            None => self.inner.try_search(needle)?,
        };
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                key,
                Entry {
                    found: found.as_ref().map(|c| StoredCommit {
                        sha: c.sha.clone(),
                        subject: c.subject.clone(),
                        timestamp: c.timestamp,
                    }),
                    head: self.head.clone(),
                },
            );
        }
        self.checkpoint();
        Ok(found)
    }
}

fn key_of(needle: &Needle) -> String {
    match needle {
        Needle::Regex { pattern, scope } => format!("r\u{1f}{pattern}\u{1f}{}", scope.join(",")),
        Needle::Literal { text, scope } => format!("l\u{1f}{text}\u{1f}{}", scope.join(",")),
        Needle::Path { path } => format!("p\u{1f}{}", path.display()),
        Needle::Heading { slug, path } => format!("h\u{1f}{slug}\u{1f}{}", path.display()),
    }
}

/// Where every repository's history cache lives: `STILLTRUE_CACHE_DIR` when it is set,
/// otherwise `stilltrue` in the platform cache directory. Outside the working tree,
/// because a linter that writes into it dirties CI diffs and gets cleaned away
/// (ADR-0007).
pub fn cache_root() -> Option<PathBuf> {
    match std::env::var_os("STILLTRUE_CACHE_DIR") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::cache_dir().map(|base| base.join("stilltrue")),
    }
}

/// Where this repository's history cache lives.
pub fn cache_path(root: &Path) -> Option<PathBuf> {
    Some(
        cache_root()?
            .join(fnv1a(&root.to_string_lossy()))
            .join("history.json"),
    )
}

/// A repository cache unused for this long is removed.
pub const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);
/// At most this many repositories are cached; the least recently used go first.
pub const MAX_REPOSITORIES: usize = 256;

/// Keep the cache bounded (ADR-0007, amended 2026-09-26).
///
/// Each repository root gets its own directory, and a root is a path: every throwaway
/// clone, fixture and temporary checkout is a new one. Unbounded, the test suite alone
/// left over a hundred thousand directories on a maintainer's machine, and the platform
/// indexer spent its memory crawling them. Only this cache's own directories — sixteen
/// hex digits, the shape `cache_path` names — are ever considered; anything else in the
/// cache root, the hook adapters' session state included, is left alone.
pub fn prune(base: &Path) {
    let now = std::time::SystemTime::now();
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let mut repositories: Vec<(std::time::Duration, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.len() == 16 && name.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .map(|entry| {
            let path = entry.path();
            let unused_for = std::fs::metadata(path.join("history.json"))
                .or_else(|_| std::fs::metadata(&path))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .unwrap_or_default();
            (unused_for, path)
        })
        .collect();
    repositories.sort_by_key(|(unused_for, _)| *unused_for);
    for (rank, (unused_for, path)) in repositories.iter().enumerate() {
        if rank >= MAX_REPOSITORIES || *unused_for > MAX_AGE {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn head_of(root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A stable 64-bit hash. Not `DefaultHasher`, which is explicitly unstable across
/// releases — these values name directories and appear in SARIF fingerprints.
///
/// The prime is `0x100_0000_01b3`. Grouped in fours it reads `0x1000_0000_01b3`, which
/// is a different number — sixteen times larger — and was what this function multiplied
/// by until the published vectors below were written down. It hashed perfectly well; it
/// was simply not the algorithm its own name claimed.
pub fn fnv1a(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod hash_tests {
    use super::fnv1a;

    /// The FNV-1a 64-bit vectors, from the reference implementation. A hash that only
    /// has to be *stable* is satisfied by any odd multiplier, so nothing in this
    /// repository could tell the difference — which is why the name needs an external
    /// witness rather than an internal one.
    #[test]
    fn the_hash_is_the_algorithm_it_is_named_after() {
        assert_eq!(fnv1a(""), "cbf29ce484222325");
        assert_eq!(fnv1a("a"), "af63dc4c8601ec8c");
        assert_eq!(fnv1a("foobar"), "85944171f73967e8");
    }
}
