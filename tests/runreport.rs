//! Seam: the run report and the summary, through the real binary (ADR-0019, ADR-0021).
//!
//! The acceptance question is whether an agent can tell a complete quiet run from an
//! incomplete one without parsing prose. Every test that asks it reads `status` and
//! nothing else.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

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
    assert!(output.status.success(), "{name}/build.sh failed");
    dir
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn stilltrue(repo: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_stilltrue"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run stilltrue");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

/// Run with `--report-file` and return the parsed report alongside the run.
fn report(repo: &Path, args: &[&str]) -> (Run, Value) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    let mut all: Vec<&str> = args.to_vec();
    all.extend(["--report-file", &path]);
    let run = stilltrue(repo, &all);
    let text = std::fs::read_to_string(&path).expect("report written");
    let value: Value = serde_json::from_str(&text).expect("report parses");
    (run, value)
}

fn repo_of(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("repo")
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn permissions_bind() -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let probe = dir.path().join("probe");
    std::fs::write(&probe, "x").unwrap();
    set_mode(&probe, 0o000);
    let refused = std::fs::read(&probe).is_err();
    set_mode(&probe, 0o644);
    refused
}

#[test]
fn a_complete_quiet_run_and_an_incomplete_quiet_run_differ_by_status() {
    let clean = build("command-near-miss");
    let shallow = build("degraded-shallow");
    let (quiet, complete) = report(&repo_of(&clean), &["--format", "json"]);
    let (also_quiet, incomplete) = report(&repo_of(&shallow), &["--format", "json"]);
    // Identical to anything reading findings alone...
    let findings = |run: &Run| -> Value {
        serde_json::from_str::<Value>(&run.stdout).unwrap()["findings"].clone()
    };
    assert_eq!(findings(&quiet), findings(&also_quiet));
    assert_eq!(quiet.code, also_quiet.code);
    // ...and told apart by one field.
    assert_eq!(complete["status"], "complete");
    assert_eq!(incomplete["status"], "incomplete");
    assert_eq!(incomplete["history"]["state"], "shallow");
    assert_eq!(incomplete["history"]["complete"], false);
}

#[test]
fn the_report_is_versioned_and_its_counts_reconcile() {
    let dir = build("command-rot");
    let (run, report) = report(&repo_of(&dir), &[]);
    assert_eq!(run.code, 1);
    assert_eq!(report["schema"], "stilltrue/run-report");
    assert_eq!(report["version"], 1);
    assert_eq!(report["tool"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["exitCode"], 1);
    let claims = &report["coverage"]["claims"];
    let parts: u64 = ["ignored", "true", "broken", "skipped", "ambiguous"]
        .iter()
        .map(|k| claims[k].as_u64().unwrap())
        .sum();
    assert_eq!(claims["extracted"].as_u64().unwrap(), parts);
    let gate = &report["coverage"]["gate"];
    let judged: u64 = [
        "rot",
        "lieReported",
        "lieUnreported",
        "abstained",
        "historyFailed",
    ]
    .iter()
    .map(|k| gate[k].as_u64().unwrap())
    .sum();
    assert_eq!(claims["broken"].as_u64().unwrap(), judged);
    assert_eq!(
        report["coverage"]["reported"]["total"].as_u64().unwrap(),
        report["findings"].as_array().unwrap().len() as u64
    );
    assert_eq!(report["coverage"]["reported"]["rot"], 1);
    assert!(
        report["run"]["head"]
            .as_str()
            .is_some_and(|h| h.len() == 40)
    );
}

#[test]
fn the_report_file_carries_the_same_findings_as_json() {
    let dir = build("command-rot");
    let (run, report) = report(&repo_of(&dir), &["--format", "json"]);
    let stdout: Value = serde_json::from_str(&run.stdout).unwrap();
    let mut from_report = report["findings"].clone();
    for finding in from_report.as_array_mut().unwrap() {
        let fingerprint = finding
            .as_object_mut()
            .unwrap()
            .remove("fingerprint")
            .expect("each finding carries its fingerprint");
        assert_eq!(fingerprint.as_str().unwrap().len(), 16);
    }
    assert_eq!(stdout["findings"], from_report);
}

#[test]
fn the_report_fingerprint_is_the_sarif_fingerprint() {
    let dir = build("command-rot");
    let (run, report) = report(&repo_of(&dir), &["--format", "sarif"]);
    let sarif: Value = serde_json::from_str(&run.stdout).unwrap();
    let from_sarif: Vec<&str> = sarif["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["partialFingerprints"]["stilltrue/v1"].as_str().unwrap())
        .collect();
    let from_report: Vec<&str> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["fingerprint"].as_str().unwrap())
        .collect();
    assert_eq!(from_sarif, from_report);
}

#[test]
fn json_stdout_is_unchanged_by_the_new_flags() {
    let dir = build("command-rot");
    let plain = stilltrue(&repo_of(&dir), &["--format", "json"]);
    let (with_flags, _) = report(&repo_of(&dir), &["--format", "json", "--summary"]);
    assert_eq!(plain.stdout, with_flags.stdout);
    assert_eq!(plain.code, with_flags.code);
    // The summary went to stderr, where a machine format cannot be corrupted by it.
    assert!(with_flags.stderr.contains("stilltrue: summary ("));
    assert!(!plain.stderr.contains("stilltrue: summary ("));
}

#[test]
fn a_baselined_run_reports_what_the_baseline_suppressed() {
    let dir = build("command-rot");
    let repo = repo_of(&dir);
    let baseline = repo.join(".stilltrue-baseline");
    let written = stilltrue(&repo, &["--write-baseline", baseline.to_str().unwrap()]);
    assert_eq!(written.code, 0);
    let (run, report) = report(
        &repo,
        &["--baseline", baseline.to_str().unwrap(), "--summary"],
    );
    assert_eq!(run.code, 0);
    assert_eq!(report["status"], "complete");
    assert_eq!(report["coverage"]["baseline"]["suppressed"], 1);
    assert_eq!(
        report["coverage"]["gate"]["rot"], 1,
        "analysis keeps what it found"
    );
    assert_eq!(report["coverage"]["reported"]["total"], 0);
    assert!(report["findings"].as_array().unwrap().is_empty());
    let limitations: Vec<&str> = report["coverage"]["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["code"].as_str().unwrap())
        .collect();
    assert!(
        limitations.contains(&"baseline-suppressed"),
        "{limitations:?}"
    );
    assert!(
        run.stderr.contains("suppressed by the baseline"),
        "{}",
        run.stderr
    );
}

#[test]
fn an_unreadable_baseline_is_diagnosed_and_suppresses_nothing() {
    let dir = build("command-rot");
    let (run, report) = report(&repo_of(&dir), &["--baseline", "no-such-baseline"]);
    assert_eq!(run.code, 1, "judging everything is the safe direction");
    assert_eq!(report["status"], "incomplete");
    assert_eq!(report["diagnostics"][0]["code"], "baseline-unreadable");
    assert_eq!(report["findings"].as_array().unwrap().len(), 1);
}

#[test]
fn strict_mode_counts_lies_as_reported() {
    let dir = build("path-lie");
    let (default, quiet) = report(&repo_of(&dir), &[]);
    assert_eq!(default.code, 0);
    assert_eq!(quiet["coverage"]["gate"]["lieUnreported"], 1);
    assert_eq!(quiet["coverage"]["reported"]["lie"], 0);
    assert_eq!(quiet["run"]["strict"], false);
    let (strict, loud) = report(&repo_of(&dir), &["--strict"]);
    assert_eq!(strict.code, 0, "--fail-on rot is not moved by --strict");
    assert_eq!(loud["coverage"]["gate"]["lieReported"], 1);
    assert_eq!(loud["coverage"]["reported"]["lie"], 1);
    assert_eq!(loud["run"]["strict"], true);
}

#[test]
fn a_run_over_nothing_is_no_input() {
    let dir = build("command-rot");
    let (run, report) = report(&repo_of(&dir), &["missing.md"]);
    assert_eq!(run.code, 0);
    assert_eq!(report["status"], "no-input");
    assert_eq!(report["coverage"]["documents"]["matched"], 0);
}

#[test]
fn an_unreadable_document_is_incomplete_in_the_report() {
    if !permissions_bind() {
        eprintln!("skipped: file modes do not bind for this user");
        return;
    }
    let dir = build("command-rot");
    let repo = repo_of(&dir);
    set_mode(&repo.join("CLAUDE.md"), 0o000);
    let (run, report) = report(&repo, &[]);
    set_mode(&repo.join("CLAUDE.md"), 0o644);
    assert_eq!(run.code, 0);
    assert_eq!(report["status"], "incomplete");
    assert_eq!(report["coverage"]["documents"]["unreadable"], 1);
    assert_eq!(report["diagnostics"][0]["path"], "CLAUDE.md");
    assert_eq!(report["diagnostics"][0]["incomplete"], true);
}

#[test]
fn an_unsupported_language_is_named_as_a_limitation_of_a_complete_run() {
    let dir = build("symbol-mixed-language");
    let (run, report) = report(&repo_of(&dir), &["--summary"]);
    assert_eq!(report["status"], "complete");
    let limitation = report["coverage"]["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["code"] == "symbols-unsupported-language")
        .expect("the limitation is named");
    assert!(limitation["count"].as_u64().unwrap() >= 1);
    assert!(
        run.stderr.contains("symbol checking unavailable"),
        "{}",
        run.stderr
    );
}

#[test]
fn require_history_refuses_before_writing_a_baseline() {
    let dir = build("degraded-shallow");
    let repo = repo_of(&dir);
    let baseline = repo.join(".stilltrue-baseline");
    let run = stilltrue(
        &repo,
        &[
            "--require-history",
            "--write-baseline",
            baseline.to_str().unwrap(),
        ],
    );
    assert_eq!(run.code, 2);
    assert!(
        !baseline.exists(),
        "a baseline recorded blind records nothing but lies"
    );
    assert!(run.stderr.contains("history unavailable"), "{}", run.stderr);
}

#[test]
fn require_history_still_writes_the_report_it_was_asked_for() {
    let dir = build("degraded-shallow");
    let (run, report) = report(&repo_of(&dir), &["--require-history"]);
    assert_eq!(run.code, 2);
    assert_eq!(report["exitCode"], 2);
    assert_eq!(report["status"], "incomplete");
}

#[test]
fn an_unwritable_report_file_exits_2() {
    let dir = build("command-near-miss");
    let run = stilltrue(
        &repo_of(&dir),
        &[
            "--format",
            "json",
            "--report-file",
            "no/such/dir/report.json",
        ],
    );
    assert_eq!(run.code, 2);
    // stdout is still the whole document it would have been.
    serde_json::from_str::<Value>(&run.stdout).expect("stdout still parses");
}

#[test]
fn an_unwritable_sarif_file_exits_2_and_says_so_in_the_report() {
    let dir = build("command-near-miss");
    let (run, report) = report(&repo_of(&dir), &["--sarif-file", "no/such/dir/out.sarif"]);
    assert_eq!(run.code, 2);
    assert_eq!(report["exitCode"], 2);
    assert_eq!(report["diagnostics"][0]["code"], "output-unwritable");
}

#[test]
fn sarif_file_and_stdout_come_from_one_analysis() {
    let dir = build("command-rot");
    let repo = repo_of(&dir);
    let sarif_path = repo.join("out.sarif");
    let run = stilltrue(
        &repo,
        &[
            "--format",
            "sarif",
            "--sarif-file",
            sarif_path.to_str().unwrap(),
        ],
    );
    assert_eq!(run.code, 1);
    assert_eq!(std::fs::read_to_string(&sarif_path).unwrap(), run.stdout);

    // And beside annotations: one result per annotation, at the same place.
    let github = stilltrue(
        &repo,
        &[
            "--format",
            "github",
            "--sarif-file",
            sarif_path.to_str().unwrap(),
        ],
    );
    let sarif: Value =
        serde_json::from_str(&std::fs::read_to_string(&sarif_path).unwrap()).unwrap();
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    let annotations: Vec<&str> = github
        .stdout
        .lines()
        .filter(|l| l.starts_with("::error"))
        .collect();
    assert_eq!(results.len(), annotations.len());
    for (result, annotation) in results.iter().zip(annotations) {
        let region = &result["locations"][0]["physicalLocation"]["region"];
        assert!(annotation.contains(&format!(
            "line={},col={}",
            region["startLine"], region["startColumn"]
        )));
    }
}

#[test]
fn a_baseline_applies_to_the_sarif_file_as_it_does_to_stdout() {
    let dir = build("command-rot");
    let repo = repo_of(&dir);
    let baseline = repo.join(".stilltrue-baseline");
    stilltrue(&repo, &["--write-baseline", baseline.to_str().unwrap()]);
    let sarif_path = repo.join("out.sarif");
    let run = stilltrue(
        &repo,
        &[
            "--baseline",
            baseline.to_str().unwrap(),
            "--sarif-file",
            sarif_path.to_str().unwrap(),
        ],
    );
    assert_eq!(run.code, 0);
    let sarif: Value =
        serde_json::from_str(&std::fs::read_to_string(&sarif_path).unwrap()).unwrap();
    assert!(sarif["runs"][0]["results"].as_array().unwrap().is_empty());
}

#[test]
fn the_summary_never_says_the_documentation_is_verified() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(&fixtures).unwrap() {
        let entry = entry.unwrap();
        if !entry.path().join("build.sh").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = build(&name);
        for args in [&["--summary"][..], &["--summary", "--strict"]] {
            let run = stilltrue(&repo_of(&dir), args);
            let summary = &run.stderr[run
                .stderr
                .find("stilltrue: summary (")
                .unwrap_or_else(|| panic!("{name}: no summary"))..];
            let lower = summary.to_lowercase();
            for forbidden in ["verified", "all documentation", "documentation is correct"] {
                assert!(!lower.contains(forbidden), "{name}: {summary}");
            }
            assert!(summary.lines().last().unwrap().starts_with("stilltrue: "));
        }
        checked += 1;
    }
    assert!(checked >= 25, "only {checked} fixtures were checked");
}

/// Timings are the only thing in a summary that changes between runs, and the summary
/// prints none.
macro_rules! summary_snapshot {
    ($name:expr, $fixture:expr, $args:expr) => {{
        let dir = build($fixture);
        let run = stilltrue(&repo_of(&dir), $args);
        let summary = run.stderr[run.stderr.find("stilltrue: summary (").unwrap()..].to_string();
        insta::with_settings!({
            filters => vec![
                (r"\b[0-9a-f]{7,40}\b", "<sha>"),
                (r#"/private/var/folders[^\s"]*"#, "<tmp>"),
                (r#"/var/folders[^\s"]*"#, "<tmp>"),
            ],
        }, {
            insta::assert_snapshot!($name, summary);
        });
    }};
}

#[test]
fn the_summary_of_each_acceptance_case() {
    summary_snapshot!("summary-clean", "command-near-miss", &["--summary"]);
    summary_snapshot!(
        "summary-zero-documents",
        "command-rot",
        &["--summary", "missing.md"]
    );
    summary_snapshot!(
        "summary-unsupported-language",
        "symbol-mixed-language",
        &["--summary"]
    );
    summary_snapshot!("summary-shallow", "degraded-shallow", &["--summary"]);
    summary_snapshot!("summary-strict", "path-lie", &["--summary", "--strict"]);
    summary_snapshot!("summary-rot", "command-rot", &["--summary"]);
    summary_snapshot!(
        "summary-unparseable-manifest",
        "manifest-unparseable",
        &["--summary"]
    );
}

#[test]
fn the_summary_of_a_baselined_run() {
    let dir = build("command-rot");
    let repo = repo_of(&dir);
    stilltrue(&repo, &["--write-baseline", ".stilltrue-baseline"]);
    let run = stilltrue(&repo, &["--summary", "--baseline", ".stilltrue-baseline"]);
    let summary = &run.stderr[run.stderr.find("stilltrue: summary (").unwrap()..];
    insta::assert_snapshot!("summary-baseline", summary);
}

fn limitations(report: &Value) -> Vec<(String, u64)> {
    report["coverage"]["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["code"].as_str().unwrap().to_string(),
                l["count"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, u64)]) -> Vec<(String, u64)> {
    list.iter().map(|(c, n)| ((*c).to_string(), *n)).collect()
}

#[test]
fn each_limitation_is_named_exactly_when_it_applies() {
    // Every fixture here has exactly the limitation its name describes, and no other:
    // a limitation that is always named, or never, would pass a test that only looked
    // for its presence.
    for (fixture, args, expected) in [
        ("command-near-miss", &[][..], &[][..]),
        ("command-near-miss", &["--strict"][..], &[][..]),
        (
            "symbol-mixed-language",
            &[],
            &[("symbols-unsupported-language", 1)],
        ),
        ("path-lie", &[], &[("lies-not-reported", 1)]),
        ("path-lie", &["--strict"], &[]),
        ("manifest-unparseable", &[], &[("claims-skipped", 1)]),
        ("ambiguous-manifest", &[], &[("claims-ambiguous", 1)]),
        ("suppression-unused", &[], &[("suppression-markers", 2)]),
    ] {
        let dir = build(fixture);
        let (_, report) = report(&repo_of(&dir), args);
        assert_eq!(limitations(&report), pairs(expected), "{fixture} {args:?}");
    }
}

#[test]
fn a_symbol_ambiguous_for_another_reason_is_not_an_unsupported_language() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    std::fs::write(repo.join("app.py"), "def handler():\n    pass\n").unwrap();
    std::fs::write(repo.join("README.md"), "Call `netrc.retrieve()`.\n").unwrap();
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
                .current_dir(repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let (_, report) = report(repo, &[]);
    assert_eq!(limitations(&report), pairs(&[("claims-ambiguous", 1)]));
    assert_eq!(
        report["coverage"]["ambiguous"],
        serde_json::json!([{"kind": "symbol", "reason": "unreadable-namespace", "count": 1}])
    );
}

#[test]
fn the_report_lists_every_skip_and_ambiguity_by_kind_and_reason() {
    let dir = build("manifest-unparseable");
    let (_, report) = report(&repo_of(&dir), &[]);
    assert_eq!(
        report["coverage"]["skipped"],
        serde_json::json!([{"kind": "command", "reason": "manifest-unparseable", "count": 1}])
    );
    assert_eq!(report["coverage"]["ambiguous"], serde_json::json!([]));
    assert_eq!(report["coverage"]["byKind"]["command"]["skipped"], 1);
}

#[test]
fn a_clean_strict_run_says_no_reportable_findings_not_no_rot() {
    let dir = build("command-near-miss");
    let run = stilltrue(&repo_of(&dir), &["--strict", "--summary"]);
    assert!(
        run.stderr
            .contains("stilltrue: No reportable findings. Checked 1 document(s)."),
        "{}",
        run.stderr
    );
    let default = stilltrue(&repo_of(&dir), &["--summary"]);
    assert!(
        default
            .stderr
            .contains("stilltrue: No reportable rot. Checked 1 document(s)."),
        "{}",
        default.stderr
    );
}
