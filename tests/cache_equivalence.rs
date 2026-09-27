//! Seam: a cached run and an uncached run over the same state say the same thing
//! (ADR-0007).
//!
//! The cache is disposable acceleration. Every acceptance case — branch switching,
//! reset, rebase, evidence objects that were collected, a shallow clone deepened, an
//! interrupted write, a tool upgrade, and runs racing each other — is a real repository
//! here, and the assertion is always the same: the findings a cached run reports,
//! attribution included, are the findings an uncached run reports.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use stilltrue::cache::cache_path;
use stilltrue::pipeline::{self, Options};

/// Commit timestamps a second apart would tie; these are a hundred seconds apart, so
/// "the latest commit" never depends on how fast the test ran.
static CLOCK: AtomicU64 = AtomicU64::new(1_700_000_000);

fn git(root: &Path, args: &[&str]) -> String {
    let when = format!("@{} +0000", CLOCK.fetch_add(100, Ordering::SeqCst));
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_AUTHOR_DATE", &when)
        .env("GIT_COMMITTER_DATE", &when)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A real repository whose cache is removed with it.
struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        Self { _dir: dir, root }
    }

    fn at(dir: tempfile::TempDir, root: PathBuf) -> Self {
        Self { _dir: dir, root }
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.root.join(name), contents).unwrap();
    }

    fn commit(&self, message: &str) -> String {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", message]);
        git(&self.root, &["rev-parse", "HEAD"])
    }

    /// `make demo` documented and present, then removed in a commit named `removal`.
    fn with_rot(removal: &str) -> Self {
        let repo = Self::new();
        repo.write("Makefile", "demo:\n\techo demo\n");
        repo.write("README.md", "Run `make demo`.\n");
        repo.commit("add demo");
        repo.write("Makefile", "seed:\n\techo seed\n");
        repo.commit(removal);
        repo
    }

    fn cache_file(&self) -> PathBuf {
        cache_path(&self.root).expect("a cache path")
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        if let Some(parent) = self.cache_file().parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

/// Every finding, with its attribution, as one comparable line each.
fn scan(root: &Path, cache: bool) -> Vec<String> {
    let outcome = pipeline::run(&Options {
        root: root.to_path_buf(),
        cwd: root.to_path_buf(),
        paths: vec![],
        strict: true,
        cache,
    });
    outcome
        .findings
        .iter()
        .map(|f| {
            format!(
                "{} {}:{} `{}` {:?}",
                f.rule_id,
                f.claim.file.display(),
                f.claim.line,
                f.claim.text,
                f.breaking_commit
                    .as_ref()
                    .map(|c| format!("{} {}", c.sha, c.subject)),
            )
        })
        .collect()
}

/// The cached run first, so the uncached reference cannot repair the cache before it
/// is read.
fn equivalent(repo: &Repo) -> Vec<String> {
    let cached = scan(&repo.root, true);
    let uncached = scan(&repo.root, false);
    assert_eq!(
        cached, uncached,
        "a cached run disagreed with an uncached one"
    );
    cached
}

fn blamed(findings: &[String], subject: &str) -> bool {
    findings.len() == 1 && findings[0].ends_with(&format!(" {subject}\")"))
}

#[test]
fn switching_branches() {
    let repo = Repo::with_rot("remove demo on main");
    assert!(blamed(&scan(&repo.root, true), "remove demo on main"));

    git(&repo.root, &["checkout", "-q", "-b", "alt", "HEAD~1"]);
    repo.write("Makefile", "serve:\n\techo serve\n");
    repo.commit("remove demo on alt");
    assert!(blamed(&equivalent(&repo), "remove demo on alt"));

    git(&repo.root, &["checkout", "-q", "main"]);
    assert!(blamed(&equivalent(&repo), "remove demo on main"));
}

#[test]
fn resetting() {
    let repo = Repo::with_rot("remove demo");
    scan(&repo.root, true);
    git(&repo.root, &["reset", "-q", "--hard", "HEAD~1"]);
    assert!(equivalent(&repo).is_empty(), "the target exists again");

    repo.write("Makefile", "serve:\n\techo serve\n");
    repo.commit("remove demo a second way");
    assert!(blamed(&equivalent(&repo), "remove demo a second way"));
}

#[test]
fn rebasing() {
    let repo = Repo::with_rot("remove demo");
    repo.write("other.txt", "unrelated\n");
    repo.commit("unrelated work");
    let before = scan(&repo.root, true);
    assert!(before[0].contains("remove demo"));

    let first = git(&repo.root, &["rev-list", "--max-parents=0", "HEAD"]);
    git(&repo.root, &["checkout", "-q", "-b", "base", &first]);
    repo.write("base.txt", "base\n");
    repo.commit("base work");
    git(&repo.root, &["checkout", "-q", "main"]);
    git(&repo.root, &["rebase", "-q", "base"]);

    let after = equivalent(&repo);
    assert!(blamed(&after, "remove demo"));
    assert_ne!(before, after, "the rebased commit has a new sha");
}

#[test]
fn evidence_objects_that_were_collected() {
    let repo = Repo::with_rot("remove demo");
    let original = git(&repo.root, &["rev-parse", "HEAD"]);
    scan(&repo.root, true);

    git(
        &repo.root,
        &["commit", "-q", "--amend", "-m", "remove demo, reworded"],
    );
    git(&repo.root, &["reflog", "expire", "--expire=now", "--all"]);
    git(&repo.root, &["gc", "-q", "--prune=now"]);
    let gone = Command::new("git")
        .current_dir(&repo.root)
        .args(["cat-file", "-e", &original])
        .status()
        .unwrap();
    assert!(
        !gone.success(),
        "the original commit is still in the object store"
    );

    assert!(blamed(&equivalent(&repo), "remove demo, reworded"));
}

#[test]
fn a_shallow_clone_deepened() {
    let origin = Repo::with_rot("remove demo");
    let dir = tempfile::tempdir().unwrap();
    let clone = dir.path().join("clone");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &format!("file://{}", origin.root.display()),
            clone.to_str().unwrap(),
        ],
    );
    let clone = Repo::at(dir, clone.canonicalize().unwrap());
    assert!(
        scan(&clone.root, true).iter().all(|f| !f.contains("/rot ")),
        "a shallow clone reported rot"
    );
    assert!(
        !clone.cache_file().exists(),
        "a degraded run wrote a cache for the deepened clone to read"
    );

    git(&clone.root, &["fetch", "-q", "--unshallow"]);
    assert!(blamed(&equivalent(&clone), "remove demo"));
}

#[test]
fn an_interrupted_write() {
    let repo = Repo::with_rot("remove demo");
    scan(&repo.root, true);
    let file = repo.cache_file();
    let text = std::fs::read(&file).unwrap();
    std::fs::write(&file, &text[..text.len() / 2]).unwrap();
    std::fs::write(file.with_extension("json.1.2.tmp"), b"{\"schema\":").unwrap();

    assert!(blamed(&equivalent(&repo), "remove demo"));
    let rewritten: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(rewritten["schema"], 2);
}

/// Replace every stored commit with one that never existed.
fn poison(file: &Path, analysis: Option<&str>) {
    let mut store: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    for entry in store["entries"].as_object_mut().unwrap().values_mut() {
        entry["found"] =
            serde_json::json!({"sha": "deadbee", "subject": "poisoned", "timestamp": 1});
    }
    if let Some(analysis) = analysis {
        store["analysis"] = serde_json::json!(analysis);
    }
    std::fs::write(file, store.to_string()).unwrap();
}

#[test]
fn a_tool_upgrade_discards_what_an_older_analysis_stored() {
    let repo = Repo::with_rot("remove demo");
    scan(&repo.root, true);

    // The control: at an unchanged HEAD a stored positive is trusted without asking git,
    // so a poisoned entry written by this very analysis is served. That is the cache
    // working, and it is what makes the version key below load-bearing.
    poison(&repo.cache_file(), None);
    assert!(scan(&repo.root, true)[0].contains("poisoned"));

    // Written by another analysis, the same entry is discarded.
    scan(&repo.root, false);
    poison(&repo.cache_file(), Some("0.0.0/0"));
    let findings = equivalent(&repo);
    assert!(!findings[0].contains("poisoned"), "{findings:?}");
}

#[test]
fn a_cache_in_the_old_layout_is_discarded() {
    let repo = Repo::with_rot("remove demo");
    scan(&repo.root, true);
    let file = repo.cache_file();
    let store: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    // Schema 1 was the bare map of entries, with no envelope.
    let mut entries = store["entries"].clone();
    for entry in entries.as_object_mut().unwrap().values_mut() {
        entry["found"] =
            serde_json::json!({"sha": "deadbee", "subject": "poisoned", "timestamp": 1});
    }
    std::fs::write(&file, entries.to_string()).unwrap();
    let findings = equivalent(&repo);
    assert!(!findings[0].contains("poisoned"), "{findings:?}");
}

#[test]
fn newer_history_refreshes_the_attribution() {
    // The stored positive is still true about the history it was computed over — the
    // target did exist once — but it names the *latest* removal, and a later one now
    // exists. Reusing it would blame a commit an uncached run would not.
    let repo = Repo::with_rot("first removal");
    assert!(blamed(&scan(&repo.root, true), "first removal"));

    repo.write("Makefile", "demo:\n\techo demo\n");
    repo.commit("restore demo");
    repo.write("Makefile", "seed:\n\techo seed\n");
    repo.commit("second removal");
    assert!(blamed(&equivalent(&repo), "second removal"));
}

#[test]
fn history_that_did_not_touch_the_needle_keeps_the_stored_answer() {
    // The near-miss of the case above: HEAD moved, nothing in the new history matches,
    // and the stored attribution is still what an uncached run reports.
    let repo = Repo::with_rot("the removal");
    scan(&repo.root, true);
    repo.write("other.txt", "unrelated\n");
    repo.commit("unrelated work");
    assert!(blamed(&equivalent(&repo), "the removal"));
}

#[test]
fn repeated_concurrent_runs() {
    let repo = Repo::with_rot("remove demo");
    let root = repo.root.clone();
    let runs: Vec<Vec<String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| scan(&root, true))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let uncached = scan(&repo.root, false);
    for run in &runs {
        assert_eq!(run, &uncached);
    }
    // Whoever renamed last, the file is whole.
    let store: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.cache_file()).unwrap()).unwrap();
    assert_eq!(store["schema"], 2);
    let strays = std::fs::read_dir(repo.cache_file().parent().unwrap())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        })
        .count();
    assert_eq!(strays, 0, "a run left its temporary file behind");
}
