//! Seam: the history cache (ADR-0007).
//!
//! The cache decides whether a claim is rot or a lie on a second run, so a wrong answer
//! here is a wrong finding — with none of the noise that would make it obvious.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use stilltrue::cache::{Cached, Mode, cache_path};
use stilltrue::git::{Commit, Git, HistoryError};
use stilltrue::resolution::Needle;

/// A `Git` that answers a fixed way and counts how often it was asked.
struct Counting {
    answer: Option<Commit>,
    calls: AtomicUsize,
    /// What `contains` says. A rewrite is modelled by answering `false`: the commit is
    /// no longer reachable from HEAD.
    reachable: bool,
    ancestry_calls: AtomicUsize,
}

impl Counting {
    fn new(answer: Option<Commit>) -> Self {
        Self {
            answer,
            calls: AtomicUsize::new(0),
            reachable: true,
            ancestry_calls: AtomicUsize::new(0),
        }
    }

    /// As if history had been rewritten: the stored commit is gone from HEAD.
    fn rewritten(answer: Option<Commit>) -> Self {
        Self {
            reachable: false,
            ..Self::new(answer)
        }
    }
}

impl Git for Counting {
    fn available(&self) -> bool {
        true
    }
    fn shallow(&self) -> bool {
        false
    }
    fn search(&self, _needle: &Needle) -> Option<Commit> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
    fn contains(&self, _sha: &str) -> bool {
        self.ancestry_calls.fetch_add(1, Ordering::SeqCst);
        self.reachable
    }
}

fn commit() -> Commit {
    Commit {
        sha: "a3f21c9".into(),
        subject: "split demo into seed+serve".into(),
        timestamp: 1_700_000_000,
    }
}

fn needle(pattern: &str, scope: &[&str]) -> Needle {
    Needle::Regex {
        pattern: pattern.into(),
        scope: scope.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn git(root: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .expect("run git")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

/// A real repository, plus cleanup of whatever the cache writes for it.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "one"]);
        Self { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn advance_head(&self) {
        std::fs::write(self.root().join("a.txt"), "two\n").unwrap();
        git(self.root(), &["add", "-A"]);
        git(self.root(), &["commit", "-q", "-m", "two"]);
    }

    fn cache_file(&self) -> Option<PathBuf> {
        cache_path(self.root())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(file) = self.cache_file()
            && let Some(parent) = file.parent()
        {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

#[test]
fn the_cache_lives_outside_the_repository() {
    // A linter that writes into the working tree dirties CI diffs (ADR-0007).
    let fixture = Fixture::new();
    let path = fixture.cache_file().expect("a cache path");
    assert!(
        !path.starts_with(fixture.root()),
        "cache {path:?} is inside {:?}",
        fixture.root()
    );
}

#[test]
fn a_positive_result_is_answered_without_asking_git_again() {
    let fixture = Fixture::new();
    let counting = Counting::new(Some(commit()));
    let cached = Cached::new(counting, fixture.root(), Mode::ReadWrite);
    let n = needle("^demo:", &["Makefile"]);

    assert_eq!(cached.search(&n).map(|c| c.sha), Some("a3f21c9".into()));
    assert_eq!(cached.search(&n).map(|c| c.sha), Some("a3f21c9".into()));
    assert_eq!(cached.search(&n).map(|c| c.sha), Some("a3f21c9".into()));
}

#[test]
fn a_positive_result_survives_a_new_commit() {
    // History is append-only: "this needle existed at some commit" stays true forever,
    // so a positive result never expires. This is the expensive direction.
    let fixture = Fixture::new();
    {
        let cached = Cached::new(
            Counting::new(Some(commit())),
            fixture.root(),
            Mode::ReadWrite,
        );
        cached.search(&needle("^demo:", &["Makefile"]));
        cached.persist();
    }
    fixture.advance_head();

    // A git that would answer "never existed" if it were asked at all.
    let counting = Counting::new(None);
    let cached = Cached::new(counting, fixture.root(), Mode::ReadWrite);
    let found = cached.search(&needle("^demo:", &["Makefile"]));
    assert_eq!(
        found.map(|c| c.sha),
        Some("a3f21c9".into()),
        "a positive result must outlive the HEAD it was found at"
    );
}

#[test]
fn a_negative_result_is_rechecked_when_head_moves() {
    // The cheap direction is always fresh: a claim that was never true yesterday may be
    // true today, and reporting otherwise would hide a real finding.
    let fixture = Fixture::new();
    {
        let cached = Cached::new(Counting::new(None), fixture.root(), Mode::ReadWrite);
        assert!(cached.search(&needle("^demo:", &["Makefile"])).is_none());
        cached.persist();
    }
    fixture.advance_head();

    let cached = Cached::new(
        Counting::new(Some(commit())),
        fixture.root(),
        Mode::ReadWrite,
    );
    let found = cached.search(&needle("^demo:", &["Makefile"]));
    assert_eq!(
        found.map(|c| c.sha),
        Some("a3f21c9".into()),
        "a negative must not be trusted at a HEAD it was not computed at"
    );
}

#[test]
fn a_negative_result_is_reused_at_the_same_head() {
    // The near-miss for the rule above: same HEAD, so the stored negative stands and
    // git is asked exactly once.
    let fixture = Fixture::new();
    {
        let cached = Cached::new(Counting::new(None), fixture.root(), Mode::ReadWrite);
        cached.search(&needle("^demo:", &["Makefile"]));
        cached.persist();
    }

    let counting = Counting::new(None);
    let cached = Cached::new(counting, fixture.root(), Mode::ReadWrite);
    assert!(cached.search(&needle("^demo:", &["Makefile"])).is_none());
    let asked = cached.into_inner().calls.load(Ordering::SeqCst);
    assert_eq!(asked, 0, "git was asked again at an unchanged HEAD");
}

#[test]
fn entries_are_keyed_by_scope_as_well_as_pattern() {
    // The same pattern against a different manifest is a different question.
    let fixture = Fixture::new();
    let counting = Counting::new(Some(commit()));
    let cached = Cached::new(counting, fixture.root(), Mode::ReadWrite);

    cached.search(&needle("^demo:", &["Makefile"]));
    cached.search(&needle("^demo:", &["sub/Makefile"]));
    let asked = cached.into_inner().calls.load(Ordering::SeqCst);
    assert_eq!(asked, 2, "two scopes collapsed into one cache entry");
}

#[test]
fn no_cache_ignores_what_is_stored_and_replaces_it() {
    // `--no-cache` bypasses *and rewrites*. Bypassing alone would leave a bad
    // cache bad, and repairing one is the reason to reach for the flag.
    let fixture = Fixture::new();
    {
        let cached = Cached::new(
            Counting::new(Some(commit())),
            fixture.root(),
            Mode::ReadWrite,
        );
        cached.search(&needle("^demo:", &["Makefile"]));
        cached.persist();
    }

    let cached = Cached::new(Counting::new(None), fixture.root(), Mode::Rewrite);
    assert!(
        cached.search(&needle("^demo:", &["Makefile"])).is_none(),
        "a stored positive was served to a run that asked to bypass the cache"
    );
    cached.persist();

    // And the replacement stuck: a later ordinary run sees the fresh answer.
    let cached = Cached::new(Counting::new(None), fixture.root(), Mode::ReadWrite);
    assert!(cached.search(&needle("^demo:", &["Makefile"])).is_none());
    assert_eq!(
        cached.into_inner().calls.load(Ordering::SeqCst),
        0,
        "the rewritten negative was not stored"
    );
}

#[test]
fn a_run_that_cannot_see_history_writes_nothing() {
    // Every answer such a run gives is "no history found". Storing those would poison
    // the cache for every later run in a repaired clone.
    let fixture = Fixture::new();
    let cached = Cached::new(Counting::new(None), fixture.root(), Mode::Off);
    cached.search(&needle("^demo:", &["Makefile"]));
    cached.persist();

    let path = fixture.cache_file().expect("a cache path");
    assert!(!path.exists(), "a degraded run wrote {path:?}");
}

/// A `Git` that reports whatever degraded state it is told to.
struct Degraded {
    available: bool,
    shallow: bool,
    partial: bool,
}

impl Git for Degraded {
    fn available(&self) -> bool {
        self.available
    }
    fn shallow(&self) -> bool {
        self.shallow
    }
    fn partial(&self) -> bool {
        self.partial
    }
    fn search(&self, _needle: &Needle) -> Option<Commit> {
        None
    }
}

#[test]
fn a_git_that_says_nothing_about_partial_clones_is_not_one() {
    // `partial` is the only method on the trait with a default, because it was added
    // after the others. The default has to be "no": an implementation that has not
    // heard of partial clones is not reporting one, and a default of `true` would put
    // every such implementation into a permanently degraded state where nothing can be
    // reported as rot at all.
    let counting = Counting::new(None);
    assert!(
        !counting.partial(),
        "the default must be false for an implementation that does not override it"
    );
}

#[test]
fn wrapping_git_in_a_cache_does_not_change_what_it_says_about_the_clone() {
    // `Cached` answers these three by delegation, and nothing in the pipeline asks it
    // to: the degraded check runs against `Subprocess` before the wrapper is built. So
    // the delegation is unobservable today and would be load-bearing the moment that
    // order changed — a cache that reported a healthy clone as shallow would take every
    // Tier A finding in the repository down to an invisible Tier B, which is the exact
    // failure ADR-0006 exists to prevent.
    let dir = tempfile::tempdir().unwrap();
    for (available, shallow, partial) in [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (true, false, true),
        (true, true, true),
    ] {
        let inner = Degraded {
            available,
            shallow,
            partial,
        };
        let cached = Cached::new(inner, dir.path(), Mode::ReadWrite);
        assert_eq!(cached.available(), available, "available({available})");
        assert_eq!(cached.shallow(), shallow, "shallow({shallow})");
        assert_eq!(cached.partial(), partial, "partial({partial})");
    }
}

#[test]
fn a_positive_is_recomputed_when_its_commit_is_no_longer_in_history() {
    // ADR-0007 originally kept positives forever, because history is append-only.
    // `git commit --amend` is enough to make that false, and the stored commit is the
    // attribution: a cached run reported `likely broke in <sha>` for a SHA the reader
    // cannot find, carrying the subject it had before the rewrite.
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);

    {
        let cached = Cached::new(
            Counting::new(Some(commit())),
            fixture.root(),
            Mode::ReadWrite,
        );
        assert!(cached.search(&n).is_some());
        cached.persist();
    }

    // A new run, at a HEAD the stored commit is not an ancestor of.
    fixture.advance_head();
    let rewritten = Commit {
        sha: "b69ad0f".into(),
        subject: "split demo into seed+serve (reworded)".into(),
        timestamp: 1_700_000_100,
    };
    let counting = Counting::rewritten(Some(rewritten.clone()));
    let cached = Cached::new(counting, fixture.root(), Mode::ReadWrite);

    let found = cached.search(&n).expect("still broken, so still a commit");
    assert_eq!(
        found.sha, rewritten.sha,
        "a stored commit that is not an ancestor of HEAD must not be reported",
    );
    let counting = cached.into_inner();
    assert_eq!(counting.calls.load(Ordering::SeqCst), 1, "it re-searched");
    assert_eq!(
        counting.ancestry_calls.load(Ordering::SeqCst),
        1,
        "and asked once whether the stored commit was still reachable",
    );
}

#[test]
fn an_unchanged_head_costs_no_ancestry_check() {
    // The reason the cache exists is the pre-commit loop, where HEAD does not move.
    // Verifying reachability there would spend a git process to learn nothing.
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);

    {
        let cached = Cached::new(
            Counting::new(Some(commit())),
            fixture.root(),
            Mode::ReadWrite,
        );
        assert!(cached.search(&n).is_some());
        cached.persist();
    }

    let cached = Cached::new(Counting::new(None), fixture.root(), Mode::ReadWrite);
    let found = cached.search(&n).expect("the stored positive");
    assert_eq!(found.sha, commit().sha);
    let counting = cached.into_inner();
    assert_eq!(counting.calls.load(Ordering::SeqCst), 0, "no re-search");
    assert_eq!(
        counting.ancestry_calls.load(Ordering::SeqCst),
        0,
        "and no ancestry check at the HEAD it was computed at",
    );
}

#[test]
fn ancestry_is_asked_once_per_commit_however_many_claims_share_it() {
    // One rename breaks every claim that named the old symbol, and they all resolve to
    // the same commit. Without memoization each would spawn its own `git merge-base`,
    // which is the cost the cache was built to avoid.
    let fixture = Fixture::new();
    let needles = [
        needle("^demo:", &["Makefile"]),
        needle("^seed:", &["Makefile"]),
        needle("^serve:", &["Makefile"]),
    ];

    {
        let cached = Cached::new(
            Counting::new(Some(commit())),
            fixture.root(),
            Mode::ReadWrite,
        );
        for n in &needles {
            assert!(cached.search(n).is_some());
        }
        cached.persist();
    }

    fixture.advance_head();
    let cached = Cached::new(
        Counting::rewritten(Some(commit())),
        fixture.root(),
        Mode::ReadWrite,
    );
    for n in &needles {
        let _ = cached.search(n);
    }
    let counting = cached.into_inner();
    assert_eq!(
        counting.ancestry_calls.load(Ordering::SeqCst),
        1,
        "three claims, one commit, one question",
    );
}

/// A `Git` that answers searches and leaves `contains` to the trait's default.
struct Silent(Option<Commit>);

impl Git for Silent {
    fn available(&self) -> bool {
        true
    }
    fn shallow(&self) -> bool {
        false
    }
    fn search(&self, _needle: &Needle) -> Option<Commit> {
        self.0.clone()
    }
}

#[test]
fn a_git_that_does_not_answer_reachability_is_not_taken_to_have_agreed() {
    // The trait default for `contains` is `false`, and which way it defaults is the
    // whole safety property: on a `true` the cache reuses the stored commit, so a `Git`
    // that cannot answer the question must decline rather than agree. Defaulting to
    // `true` would silently restore the stale attribution that ADR-0007's amendment
    // exists to prevent.
    assert!(
        !Silent(None).contains("abc1234"),
        "the default answer is no",
    );

    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);
    {
        let cached = Cached::new(Silent(Some(commit())), fixture.root(), Mode::ReadWrite);
        assert!(cached.search(&n).is_some());
        cached.persist();
    }

    // HEAD moves, so the stored positive needs proving. This `Git` cannot prove it, so
    // the entry is recomputed rather than trusted.
    fixture.advance_head();
    let rewritten = Commit {
        sha: "fadedfa".into(),
        subject: "recomputed".into(),
        timestamp: 1_700_000_500,
    };
    let cached = Cached::new(
        Silent(Some(rewritten.clone())),
        fixture.root(),
        Mode::ReadWrite,
    );
    let found = cached.search(&n).expect("still broken");
    assert_eq!(
        found.sha, rewritten.sha,
        "an unprovable stored commit is not reported",
    );
}

/// A `Git` whose first `failures` searches fail, and which answers after that.
struct Flaky {
    failures: AtomicUsize,
    answer: Option<Commit>,
    calls: AtomicUsize,
}

impl Git for Flaky {
    fn available(&self) -> bool {
        true
    }
    fn shallow(&self) -> bool {
        false
    }
    fn search(&self, needle: &Needle) -> Option<Commit> {
        self.try_search(needle).ok().flatten()
    }
    fn try_search(&self, _needle: &Needle) -> Result<Option<Commit>, HistoryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(HistoryError);
        }
        Ok(self.answer.clone())
    }
}

#[test]
fn a_failed_search_is_not_cached_as_absence() {
    // ADR-0020. Before it, a failed search returned the same `None` as an empty one,
    // was stored as a negative at this HEAD, and every later run at this HEAD read
    // "never existed" out of the cache without asking git again.
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);
    {
        let flaky = Flaky {
            failures: AtomicUsize::new(1),
            answer: Some(commit()),
            calls: AtomicUsize::new(0),
        };
        let cached = Cached::new(flaky, fixture.root(), Mode::ReadWrite);
        assert_eq!(cached.try_search(&n), Err(HistoryError));
        cached.persist();
    }
    let working = Counting::new(Some(commit()));
    let cached = Cached::new(working, fixture.root(), Mode::ReadWrite);
    assert_eq!(cached.try_search(&n), Ok(Some(commit())));
    assert_eq!(
        cached.into_inner().calls.load(Ordering::SeqCst),
        1,
        "the failed search was answered from the cache"
    );
    if let Some(path) = fixture.cache_file() {
        let _ = std::fs::remove_file(path);
    }
}

/// A `Git` that answers full searches and range searches separately, and counts both.
struct Ranged {
    full: Option<Commit>,
    range: Result<Option<Commit>, HistoryError>,
    full_calls: AtomicUsize,
    range_calls: AtomicUsize,
}

impl Ranged {
    fn new(full: Option<Commit>, range: Result<Option<Commit>, HistoryError>) -> Self {
        Self {
            full,
            range,
            full_calls: AtomicUsize::new(0),
            range_calls: AtomicUsize::new(0),
        }
    }
}

impl Git for Ranged {
    fn available(&self) -> bool {
        true
    }
    fn shallow(&self) -> bool {
        false
    }
    fn search(&self, _needle: &Needle) -> Option<Commit> {
        self.full_calls.fetch_add(1, Ordering::SeqCst);
        self.full.clone()
    }
    fn search_since(&self, _needle: &Needle, _since: &str) -> Result<Option<Commit>, HistoryError> {
        self.range_calls.fetch_add(1, Ordering::SeqCst);
        self.range.clone()
    }
    fn contains(&self, _sha: &str) -> bool {
        true
    }
}

/// A stored positive for `commit()` at the fixture's first HEAD, and HEAD then moved.
fn stored_then_moved(fixture: &Fixture, n: &Needle) {
    let cached = Cached::new(
        Counting::new(Some(commit())),
        fixture.root(),
        Mode::ReadWrite,
    );
    assert!(cached.search(n).is_some());
    cached.persist();
    fixture.advance_head();
}

fn at(sha: &str, subject: &str, timestamp: i64) -> Commit {
    Commit {
        sha: sha.into(),
        subject: subject.into(),
        timestamp,
    }
}

#[test]
fn a_moved_head_searches_only_the_history_added_since() {
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);
    stored_then_moved(&fixture, &n);

    // A full search would say something else entirely; it must not be asked.
    let ranged = Ranged::new(Some(at("0ther00", "never asked", 1_900_000_000)), Ok(None));
    let cached = Cached::new(ranged, fixture.root(), Mode::ReadWrite);
    assert_eq!(
        cached.search(&n),
        Some(commit()),
        "nothing newer: the stored answer"
    );
    let ranged = cached.into_inner();
    assert_eq!(ranged.range_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ranged.full_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_later_match_in_the_new_history_takes_the_blame() {
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);
    stored_then_moved(&fixture, &n);
    let later = at("1a7e500", "removed again", commit().timestamp + 500);
    let cached = Cached::new(
        Ranged::new(None, Ok(Some(later.clone()))),
        fixture.root(),
        Mode::ReadWrite,
    );
    assert_eq!(cached.search(&n), Some(later.clone()));
    cached.persist();

    // And the refreshed answer is what is stored, at the new HEAD.
    let cached = Cached::new(Counting::new(None), fixture.root(), Mode::ReadWrite);
    assert_eq!(cached.search(&n), Some(later));
    assert_eq!(cached.into_inner().calls.load(Ordering::SeqCst), 0);
}

#[test]
fn an_earlier_dated_match_merged_in_does_not_take_the_blame() {
    // A side branch merged in brings commits dated before the stored one. `git log`
    // walks newest first, so an uncached search still names the stored commit, and so
    // must the cache.
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);
    stored_then_moved(&fixture, &n);
    let older = at("01de500", "on a side branch", commit().timestamp - 500);
    let cached = Cached::new(
        Ranged::new(None, Ok(Some(older))),
        fixture.root(),
        Mode::ReadWrite,
    );
    assert_eq!(cached.search(&n), Some(commit()));
}

#[test]
fn a_failed_range_search_falls_back_to_a_full_one() {
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);
    stored_then_moved(&fixture, &n);
    let full = at("f011000", "from the full search", commit().timestamp);
    let cached = Cached::new(
        Ranged::new(Some(full.clone()), Err(HistoryError)),
        fixture.root(),
        Mode::ReadWrite,
    );
    assert_eq!(cached.search(&n), Some(full));
    let ranged = cached.into_inner();
    assert_eq!(ranged.range_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ranged.full_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn the_range_is_real_git_history() {
    // Through the real subprocess: `since..HEAD` holds exactly the commits after it.
    let fixture = Fixture::new();
    let root = fixture.root();
    std::fs::write(root.join("Makefile"), "demo:\n\techo demo\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "add demo"]);
    std::fs::write(root.join("Makefile"), "seed:\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "first removal"]);
    let since = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let since = since.trim();
    let subprocess = stilltrue::git::Subprocess::new(root);
    let n = needle("^demo[[:space:]]*:", &["Makefile"]);
    assert_eq!(
        subprocess.search_since(&n, since),
        Ok(None),
        "nothing after it yet"
    );

    std::fs::write(root.join("unrelated.txt"), "x\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "unrelated"]);
    assert_eq!(subprocess.search_since(&n, since), Ok(None));

    std::fs::write(root.join("Makefile"), "demo:\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "restore"]);
    std::fs::write(root.join("Makefile"), "seed:\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "second removal"]);
    let found = subprocess
        .search_since(&n, since)
        .unwrap()
        .expect("a match after it");
    assert_eq!(found.subject, "second removal");
    let path = Needle::Path {
        path: "Makefile".into(),
    };
    assert!(subprocess.search_since(&path, since).unwrap().is_some());
}

/// Age a cache entry's `history.json` by `by`.
fn stale(dir: &Path, name: &str, by: std::time::Duration) {
    let entry = dir.join(name);
    std::fs::create_dir_all(&entry).unwrap();
    let file = entry.join("history.json");
    std::fs::write(&file, "{}").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - by)
        .unwrap();
}

#[test]
fn the_cache_keeps_itself_bounded() {
    // Every throwaway checkout is a new root, and each root had a directory forever: an
    // unbounded cache reached 107,818 directories on one machine. Old entries go, and
    // at most MAX_REPOSITORIES stay, least recently used first.
    use stilltrue::cache::{MAX_AGE, MAX_REPOSITORIES, prune};
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let hour = std::time::Duration::from_secs(3600);
    stale(base, "00000000000000aa", MAX_AGE + hour);
    for i in 0..MAX_REPOSITORIES + 20 {
        stale(base, &format!("{:016x}", 0x1000 + i), hour * (i as u32 + 1));
    }
    // Not this cache's shape: never touched, however old.
    stale(base, "sessions", MAX_AGE * 4);
    std::fs::write(base.join(".metadata_never_index"), "").unwrap();

    prune(base);

    let left: Vec<String> = std::fs::read_dir(base)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let repositories = left
        .iter()
        .filter(|n| n.len() == 16 && n.chars().all(|c| c.is_ascii_hexdigit()))
        .count();
    assert_eq!(repositories, MAX_REPOSITORIES);
    assert!(
        !left.contains(&"00000000000000aa".to_string()),
        "older than MAX_AGE"
    );
    assert!(
        left.contains(&format!("{:016x}", 0x1000)),
        "the most recent stays"
    );
    assert!(
        !left.contains(&format!("{:016x}", 0x1000 + MAX_REPOSITORIES + 19)),
        "the least recent goes"
    );
    assert!(left.contains(&"sessions".to_string()));
    assert!(left.contains(&".metadata_never_index".to_string()));
}

#[test]
fn tests_never_write_to_the_developers_own_cache() {
    // .cargo/config.toml points every test at the build directory's cache instead.
    let root = stilltrue::cache::cache_root().expect("a cache root");
    assert!(
        root.ends_with("target/test-cache"),
        "tests would write into {}",
        root.display()
    );
}

/// A `Git` whose every search takes a while, as a pickaxe over long history does.
struct Slow;

impl Git for Slow {
    fn available(&self) -> bool {
        true
    }
    fn shallow(&self) -> bool {
        false
    }
    fn search(&self, _needle: &Needle) -> Option<Commit> {
        std::thread::sleep(std::time::Duration::from_millis(400));
        Some(commit())
    }
}

#[test]
fn a_run_that_is_killed_still_leaves_what_it_had_found() {
    // The hook adapter kills a scan that overruns its budget. Written only at the end,
    // the cache lost every finished search with it, so a large repository's scan was
    // abandoned at the same point every time and never finished. Nothing here calls
    // persist(): whatever is on disk was checkpointed while the searches ran.
    let fixture = Fixture::new();
    let cached = Cached::new(Slow, fixture.root(), Mode::ReadWrite);
    for pattern in ["^one:", "^two:", "^three:", "^four:"] {
        cached.search(&needle(pattern, &["Makefile"]));
    }
    let path = fixture.cache_file().unwrap();
    std::mem::forget(cached); // as if killed: no destructor, no final write
    let store: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("checkpointed")).unwrap();
    let kept = store["entries"].as_object().unwrap().len();
    assert!(
        (1..=4).contains(&kept),
        "{kept} entries were on disk before the run ended"
    );
}

#[test]
fn the_file_names_the_tool_version_that_wrote_it() {
    let fixture = Fixture::new();
    let cached = Cached::new(
        Counting::new(Some(commit())),
        fixture.root(),
        Mode::ReadWrite,
    );
    cached.search(&needle("^demo:", &["Makefile"]));
    cached.persist();
    let store: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture.cache_file().unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        store["analysis"],
        format!("{}/1", env!("CARGO_PKG_VERSION")),
        "a tool upgrade must be able to tell its own cache from an older one"
    );
    assert_eq!(stilltrue::cache::analysis_version(), store["analysis"]);
}

#[test]
fn a_fast_run_writes_the_cache_once_not_after_every_search() {
    // The checkpoint is a floor on how often, not a write per search: a hundred quick
    // answers inside a second leave nothing on disk until the run persists.
    let fixture = Fixture::new();
    let cached = Cached::new(
        Counting::new(Some(commit())),
        fixture.root(),
        Mode::ReadWrite,
    );
    for i in 0..100 {
        cached.search(&needle(&format!("^target{i}:"), &["Makefile"]));
    }
    let path = fixture.cache_file().unwrap();
    assert!(
        !path.exists(),
        "the cache was written mid-run without a second passing"
    );
    cached.persist();
    assert!(path.exists());
}

#[test]
fn a_git_that_cannot_search_a_range_still_finds_a_later_match() {
    // The default `search_since` searches everything, and the later of that answer and
    // the stored one wins — so an implementation without a range search is only slower.
    let fixture = Fixture::new();
    let n = needle("^demo:", &["Makefile"]);
    stored_then_moved(&fixture, &n);
    let later = at("1a7e999", "later", commit().timestamp + 1000);
    let cached = Cached::new(
        Counting::new(Some(later.clone())),
        fixture.root(),
        Mode::ReadWrite,
    );
    assert_eq!(cached.search(&n), Some(later));
}

#[test]
fn a_cache_counts_its_own_process_on_top_of_the_git_it_wraps() {
    let fixture = Fixture::new();
    // A `Git` that spawns nothing says so by default.
    assert_eq!(Counting::new(None).processes(), 0);
    let cached = Cached::new(Counting::new(None), fixture.root(), Mode::ReadWrite);
    assert_eq!(cached.processes(), 1, "its own `rev-parse HEAD`");
}

#[test]
fn age_alone_expires_an_entry_well_under_the_count_bound() {
    use stilltrue::cache::{MAX_AGE, prune};
    let dir = tempfile::tempdir().unwrap();
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    stale(dir.path(), "00000000000000b1", MAX_AGE + day);
    stale(dir.path(), "00000000000000b2", MAX_AGE - day);
    // Shapes that are not this cache's: sixteen characters that are not hex, and hex
    // that is not sixteen characters. Neither is ever pruned, however old.
    stale(dir.path(), "not-a-cache-dir!", MAX_AGE * 3);
    stale(dir.path(), "abc", MAX_AGE * 3);
    prune(dir.path());
    let left: std::collections::BTreeSet<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        left,
        ["00000000000000b2", "abc", "not-a-cache-dir!"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
}

#[test]
fn an_empty_cache_directory_setting_is_no_setting() {
    // `STILLTRUE_CACHE_DIR=` must not become the current directory — that would write
    // the cache into the repository being checked, which ADR-0007 exists to prevent.
    let fixture = Fixture::new();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(fixture.root().join("README.md"), "See `a.txt`.\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_stilltrue"))
        .current_dir(fixture.root())
        .env("STILLTRUE_CACHE_DIR", "")
        .env("HOME", home.path())
        .env("XDG_CACHE_HOME", home.path().join(".cache"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let status = Command::new("git")
        .arg("-C")
        .arg(fixture.root())
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&status.stdout).trim(),
        "?? README.md",
        "the run wrote into the working tree"
    );
}
