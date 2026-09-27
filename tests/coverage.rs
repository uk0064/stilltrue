//! Seam: what a run examined (ADR-0019).
//!
//! Each acceptance case — clean, zero-document, unreadable, unsupported language,
//! shallow history, baseline-filtered and strict — is a status and a set of counts here,
//! read from the structured result rather than from prose. The baseline-filtered case
//! lives with the CLI in `tests/runreport.rs`, because the baseline is applied after
//! analysis.

use std::path::{Path, PathBuf};
use std::process::Command;

use stilltrue::coverage::Status;
use stilltrue::pipeline::{self, Options, Outcome};

fn build(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .join("build.sh");
    let output = Command::new("bash")
        .arg(&script)
        .arg(dir.path().join("repo"))
        .output()
        .expect("run build.sh");
    assert!(
        output.status.success(),
        "{name}/build.sh failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    dir
}

fn analyse(root: &Path, paths: &[&str], strict: bool) -> Outcome {
    pipeline::run(&Options {
        root: root.to_path_buf(),
        cwd: root.to_path_buf(),
        paths: paths.iter().map(PathBuf::from).collect(),
        strict,
        cache: false,
    })
}

fn codes(outcome: &Outcome) -> Vec<&'static str> {
    outcome.diagnostics.iter().map(|d| d.code).collect()
}

/// Whether file modes can refuse this process a read. Root ignores them.
fn permissions_bind() -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let probe = dir.path().join("probe");
    std::fs::write(&probe, "x").unwrap();
    set_mode(&probe, 0o000);
    let refused = std::fs::read(&probe).is_err();
    set_mode(&probe, 0o644);
    refused
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_clean_run_is_complete_and_says_what_it_read() {
    let dir = build("command-near-miss");
    let outcome = analyse(&dir.path().join("repo"), &[], false);
    assert_eq!(
        outcome.status(),
        Status::Complete,
        "{:?}",
        outcome.diagnostics
    );
    assert!(outcome.findings.is_empty());
    let coverage = &outcome.coverage;
    assert_eq!(coverage.documents_matched, outcome.documents);
    assert_eq!(coverage.documents_read, outcome.documents);
    assert!(coverage.claims["command"].true_ >= 1, "{coverage:?}");
    coverage.reconcile().unwrap();
    // Nothing broken, so nothing searched: `resolve` never invokes git, and the only
    // processes are the three capability probes and the cache's `rev-parse HEAD`.
    assert_eq!(outcome.history.searches, 0);
    assert_eq!(outcome.history.git_processes, 4);
}

#[test]
fn a_run_that_matched_nothing_is_no_input_not_complete() {
    let dir = build("command-rot");
    let outcome = analyse(&dir.path().join("repo"), &["nothing-here.md"], false);
    assert_eq!(outcome.status(), Status::NoInput);
    assert_eq!(outcome.documents, 0);
    assert!(codes(&outcome).contains(&"no-documents"));
    assert!(
        outcome.diagnostics.iter().all(|d| !d.incomplete),
        "matching nothing is not an operational failure: {:?}",
        outcome.diagnostics
    );
}

#[test]
fn an_unreadable_document_makes_a_quiet_run_incomplete() {
    if !permissions_bind() {
        eprintln!("skipped: file modes do not bind for this user");
        return;
    }
    let dir = build("command-rot");
    let root = dir.path().join("repo");
    set_mode(&root.join("CLAUDE.md"), 0o000);
    let outcome = analyse(&root, &[], false);
    set_mode(&root.join("CLAUDE.md"), 0o644);
    // Quiet — the only claim was in the file nobody could read — and not clean.
    assert!(outcome.findings.is_empty());
    assert_eq!(outcome.status(), Status::Incomplete);
    assert_eq!(outcome.coverage.documents_unreadable, 1);
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|d| d.code == "document-unreadable")
        .expect("a diagnostic for the unreadable document");
    assert_eq!(diagnostic.path.as_deref(), Some(Path::new("CLAUDE.md")));
    outcome.coverage.reconcile().unwrap();
}

#[test]
fn an_unsupported_language_is_a_coverage_limit_not_a_failure() {
    let dir = build("symbol-mixed-language");
    let outcome = analyse(&dir.path().join("repo"), &[], true);
    assert_eq!(
        outcome.status(),
        Status::Complete,
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        outcome.coverage.ambiguous[&("symbol", "unsupported-language")] >= 1,
        "{:?}",
        outcome.coverage.ambiguous
    );
    outcome.coverage.reconcile().unwrap();
}

#[test]
fn a_shallow_clone_is_incomplete_whatever_it_found() {
    let dir = build("degraded-shallow");
    let outcome = analyse(&dir.path().join("repo"), &[], false);
    assert_eq!(outcome.status(), Status::Incomplete);
    assert!(codes(&outcome).contains(&"history-unavailable"));
    assert_eq!(outcome.degraded.slug(), "shallow");
    // The broken claim is still counted — as a lie nobody reports by default.
    assert_eq!(outcome.coverage.gate.lie_unreported, 1);
    outcome.coverage.reconcile().unwrap();
}

#[test]
fn an_invalid_config_makes_the_run_incomplete() {
    let dir = build("command-rot");
    let root = dir.path().join("repo");
    std::fs::write(root.join("stilltrue.toml"), "include = [\n").unwrap();
    let outcome = analyse(&root, &[], false);
    assert_eq!(outcome.status(), Status::Incomplete);
    assert!(codes(&outcome).contains(&"config-invalid"));
}

#[test]
fn a_bad_glob_in_config_makes_the_run_incomplete() {
    let dir = build("command-rot");
    let root = dir.path().join("repo");
    std::fs::write(root.join("stilltrue.toml"), "exclude = [\"docs/[\"]\n").unwrap();
    let outcome = analyse(&root, &[], false);
    assert_eq!(outcome.status(), Status::Incomplete);
    assert!(codes(&outcome).contains(&"glob-invalid"));
}

#[test]
fn a_directory_the_walk_cannot_enter_makes_the_run_incomplete() {
    if !permissions_bind() {
        eprintln!("skipped: file modes do not bind for this user");
        return;
    }
    let dir = build("command-near-miss");
    let root = dir.path().join("repo");
    let locked = root.join("docs/private");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("guide.md"), "Run `make demo`.\n").unwrap();
    set_mode(&locked, 0o000);
    let outcome = analyse(&root, &[], false);
    set_mode(&locked, 0o755);
    assert_eq!(outcome.status(), Status::Incomplete);
    assert!(
        codes(&outcome).contains(&"walk-error"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn a_manifest_that_does_not_parse_is_counted_and_makes_the_run_incomplete() {
    let dir = build("manifest-unparseable");
    let outcome = analyse(&dir.path().join("repo"), &[], true);
    assert!(outcome.findings.is_empty());
    assert_eq!(outcome.status(), Status::Incomplete);
    assert_eq!(
        outcome.coverage.skipped[&("command", "manifest-unparseable")],
        1
    );
    assert!(codes(&outcome).contains(&"manifest-unparseable"));
}

#[test]
fn a_missing_manifest_is_a_limit_and_leaves_the_run_complete() {
    // The pair of the case above: nothing to read is an abstention, not a failure.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("README.md"), "Run `make demo`.\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            "one",
        ],
    ] {
        assert!(
            Command::new("git")
                .current_dir(root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let outcome = analyse(root, &[], false);
    assert_eq!(
        outcome.status(),
        Status::Complete,
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(outcome.coverage.skipped[&("command", "no-manifest")], 1);
}

#[test]
fn strict_moves_a_lie_from_unreported_to_reported() {
    let dir = build("path-lie");
    let root = dir.path().join("repo");
    let default = analyse(&root, &[], false);
    assert_eq!(default.coverage.gate.lie_unreported, 1);
    assert_eq!(default.coverage.gate.lie_reported, 0);
    assert!(default.findings.is_empty());
    let strict = analyse(&root, &[], true);
    assert_eq!(strict.coverage.gate.lie_unreported, 0);
    assert_eq!(strict.coverage.gate.lie_reported, 1);
    assert_eq!(strict.findings.len(), 1);
}

#[test]
fn suppression_markers_are_counted_rather_than_the_claims_they_hide() {
    let dir = build("suppression-unused");
    let outcome = analyse(&dir.path().join("repo"), &[], false);
    assert!(outcome.coverage.suppression_markers >= 1);
    assert!(outcome.coverage.suppression_unused >= 1);
    assert!(
        outcome.coverage.suppression_unused <= outcome.coverage.suppression_markers,
        "{:?}",
        outcome.coverage
    );
}

#[test]
fn the_counts_reconcile_on_every_fixture() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(&fixtures).unwrap() {
        let entry = entry.unwrap();
        if !entry.path().join("build.sh").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = build(&name);
        for strict in [false, true] {
            let outcome = analyse(&dir.path().join("repo"), &[], strict);
            let coverage = &outcome.coverage;
            coverage
                .reconcile()
                .unwrap_or_else(|e| panic!("{name} (strict: {strict}): {e}"));
            // Every reported finding is accounted for by the gate, or is an unused
            // suppression — which only exists under --strict.
            let suppressions = outcome
                .findings
                .iter()
                .filter(|f| f.rule_id == "stilltrue/suppression/unused")
                .count();
            assert_eq!(
                outcome.findings.len(),
                coverage.gate.rot + coverage.gate.lie_reported + suppressions,
                "{name} (strict: {strict})"
            );
            if !strict {
                assert_eq!(suppressions, 0, "{name}");
            }
            assert_eq!(coverage.documents_matched, outcome.documents, "{name}");
        }
        checked += 1;
    }
    // A loop that silently visited nothing would pass. Say how many it visited.
    assert!(checked >= 25, "only {checked} fixtures were checked");
}

#[test]
fn a_config_that_exists_and_cannot_be_read_makes_the_run_incomplete() {
    // Its pair is an absent config, which is the ordinary case and says nothing.
    if !permissions_bind() {
        eprintln!("skipped: file modes do not bind for this user");
        return;
    }
    let dir = build("command-near-miss");
    let root = dir.path().join("repo");
    let absent = analyse(&root, &[], false);
    assert!(!codes(&absent).contains(&"config-unreadable"));
    std::fs::write(root.join("stilltrue.toml"), "strict = true\n").unwrap();
    set_mode(&root.join("stilltrue.toml"), 0o000);
    let outcome = analyse(&root, &[], false);
    set_mode(&root.join("stilltrue.toml"), 0o644);
    assert_eq!(outcome.status(), Status::Incomplete);
    assert!(codes(&outcome).contains(&"config-unreadable"));
}

#[test]
fn an_invalid_config_is_explained_on_one_line() {
    let dir = build("command-rot");
    let root = dir.path().join("repo");
    std::fs::write(root.join("stilltrue.toml"), "include = [\n").unwrap();
    let outcome = analyse(&root, &[], false);
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|d| d.code == "config-invalid")
        .unwrap();
    assert!(
        diagnostic
            .message
            .starts_with("stilltrue.toml was ignored: "),
        "{}",
        diagnostic.message
    );
    assert!(!diagnostic.message.contains('\n'), "{}", diagnostic.message);
    assert!(
        diagnostic.message.len() > "stilltrue.toml was ignored: ".len() + 10,
        "the parser's reason is kept: {}",
        diagnostic.message
    );
}

#[test]
fn a_history_search_that_fails_on_a_corrupt_object_is_diagnosed() {
    // A real git failure, not a fake one: the blob the pickaxe must read is gone.
    let dir = build("command-rot");
    let root = dir.path().join("repo");
    let blob = Command::new("git")
        .current_dir(&root)
        .args(["rev-parse", "HEAD~1:Makefile"])
        .output()
        .unwrap();
    let blob = String::from_utf8(blob.stdout).unwrap();
    let blob = blob.trim();
    std::fs::remove_file(root.join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
    let outcome = analyse(&root, &[], true);
    assert!(
        outcome.findings.is_empty(),
        "a failed search is not a lie: {:?}",
        outcome.findings
    );
    assert_eq!(outcome.coverage.gate.history_failed, 1);
    assert_eq!(outcome.history.failed_searches, 1);
    assert!(codes(&outcome).contains(&"history-search-failed"));
    assert_eq!(outcome.status(), Status::Incomplete);
    outcome.coverage.reconcile().unwrap();
}

#[test]
fn the_timings_are_real_and_add_up() {
    let dir = build("command-rot");
    let outcome = analyse(&dir.path().join("repo"), &[], false);
    let t = outcome.timings;
    // git spawns alone take milliseconds, so a run cannot honestly report under two.
    assert!(t.total_ms >= 2, "{t:?}");
    assert!(
        t.total_ms >= t.select_ms + t.extract_resolve_ms + t.history_ms,
        "{t:?}"
    );
}
