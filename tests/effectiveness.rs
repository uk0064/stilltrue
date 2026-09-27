//! The controlled-edit effectiveness suite.
//!
//! Each case under tests/effectiveness/cases builds a real repository from one of two
//! families, makes one labelled edit and commits it, and declares what the default run
//! must say: the finding the edit should produce, or silence. `# check:` proves the edit
//! happened — a near-miss passes by being silent, so an edit that silently did nothing
//! would otherwise pass for the wrong reason.
//!
//! What this measures is *controlled-edit detection*: of the in-scope rot these edits
//! introduce, how much the default run reports, and how often it reports something no
//! edit introduced. It is not recall over arbitrary prose, and the numbers are printed
//! with their denominators so nobody mistakes a handful of cases for a rate. The
//! `holdout` family was written without tuning any rule against it; keep it that way
//! when a classification rule changes, and add fresh cases rather than editing these.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

/// The six kinds of in-scope rot every split must cover.
const KINDS: &[&str] = &[
    "deleted command target",
    "renamed path",
    "removed symbol",
    "changed pin",
    "removed env var",
    "broken heading",
];

struct Case {
    split: String,
    name: String,
    kind: String,
    in_scope: bool,
    expect: Vec<String>,
    check: String,
    script: PathBuf,
}

fn cases() -> Vec<Case> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/effectiveness/cases");
    let mut out = Vec::new();
    for split in ["dev", "holdout"] {
        let mut scripts: Vec<PathBuf> = std::fs::read_dir(root.join(split))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "sh"))
            .collect();
        scripts.sort();
        for script in scripts {
            let text = std::fs::read_to_string(&script).unwrap();
            let header = |key: &str| -> Vec<String> {
                text.lines()
                    .filter_map(|l| l.strip_prefix(&format!("# {key}: ")))
                    .map(str::to_string)
                    .collect()
            };
            let one = |key: &str| {
                header(key)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| panic!("{}: no `# {key}:`", script.display()))
            };
            let expect: Vec<String> = header("expect")
                .into_iter()
                .filter(|e| e != "silent")
                .collect();
            out.push(Case {
                split: split.to_string(),
                name: script.file_stem().unwrap().to_string_lossy().into_owned(),
                kind: one("kind"),
                in_scope: one("scope") == "in",
                expect,
                check: one("check"),
                script,
            });
        }
    }
    out
}

struct Result {
    findings: Vec<String>,
    status: String,
    abstentions: u64,
    cold_ms: u128,
    warm_ms: u128,
}

fn run(case: &Case) -> Result {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let built = Command::new("bash")
        .arg(&case.script)
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}: build failed:\n{}",
        case.name,
        String::from_utf8_lossy(&built.stderr)
    );
    let checked = Command::new("bash")
        .args(["-c", &case.check])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(
        checked.success(),
        "{}: the edit did not happen (`{}` failed)",
        case.name,
        case.check
    );

    let report = dir.path().join("report.json");
    let scan = || {
        let started = Instant::now();
        let output = Command::new(env!("CARGO_BIN_EXE_stilltrue"))
            .current_dir(&repo)
            .env("HOME", dir.path())
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .args(["--format", "json", "--report-file"])
            .arg(&report)
            .output()
            .unwrap();
        (started.elapsed().as_millis(), output)
    };
    let (cold_ms, _) = scan();
    let (warm_ms, output) = scan();
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    let report: Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    let claims = &report["coverage"]["claims"];
    Result {
        findings: stdout["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                format!(
                    "{} {}",
                    f["ruleId"].as_str().unwrap(),
                    f["claim"].as_str().unwrap()
                )
            })
            .collect(),
        status: report["status"].as_str().unwrap().to_string(),
        abstentions: claims["skipped"].as_u64().unwrap() + claims["ambiguous"].as_u64().unwrap(),
        cold_ms,
        warm_ms,
    }
}

#[derive(Default)]
struct Tally {
    rot: usize,
    detected: usize,
    silent_cases: usize,
    false_alarms: usize,
    out_of_scope: usize,
    out_of_scope_silent: usize,
    abstentions: u64,
    cold_ms: Vec<u128>,
    warm_ms: Vec<u128>,
}

fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

#[test]
fn controlled_edits_are_detected_and_near_misses_stay_silent() {
    let cases = cases();
    let mut failures = Vec::new();
    let mut records = Vec::new();
    let mut tallies: std::collections::BTreeMap<String, Tally> = Default::default();
    for case in &cases {
        let result = run(case);
        let tally = tallies.entry(case.split.clone()).or_default();
        tally.abstentions += result.abstentions;
        tally.cold_ms.push(result.cold_ms);
        tally.warm_ms.push(result.warm_ms);
        if result.status != "complete" {
            failures.push(format!("{}: the run was {}", case.name, result.status));
        }
        let expected: Vec<&String> = case.expect.iter().collect();
        let detected = expected.iter().all(|e| result.findings.contains(e));
        let unexpected: Vec<&String> = result
            .findings
            .iter()
            .filter(|f| !case.expect.contains(f))
            .collect();
        if !case.in_scope {
            tally.out_of_scope += 1;
            tally.out_of_scope_silent += usize::from(result.findings.is_empty());
        } else if expected.is_empty() {
            tally.silent_cases += 1;
        } else {
            tally.rot += expected.len();
            tally.detected += if detected { expected.len() } else { 0 };
            if !detected {
                failures.push(format!(
                    "{}/{}: missed {:?}, got {:?}",
                    case.split, case.name, case.expect, result.findings
                ));
            }
        }
        if !unexpected.is_empty() {
            tally.false_alarms += unexpected.len();
            failures.push(format!(
                "{}/{}: false alarm {unexpected:?}",
                case.split, case.name
            ));
        }
        records.push(json!({
            "split": case.split, "case": case.name, "kind": case.kind,
            "inScope": case.in_scope, "expected": case.expect, "found": result.findings,
            "status": result.status, "abstentions": result.abstentions,
            "coldMs": result.cold_ms, "warmMs": result.warm_ms,
        }));
    }

    // Say what was measured, with denominators, whatever the outcome.
    eprintln!("controlled-edit detection (default run), per split:");
    for (split, tally) in &mut tallies {
        eprintln!(
            "  {split:8} in-scope rot detected {}/{} · false alarms {} across {} silent cases \
             and {} rot cases · out of scope {} ({} silent) · abstentions {} · median cold {}ms, warm {}ms",
            tally.detected,
            tally.rot,
            tally.false_alarms,
            tally.silent_cases,
            tally.rot,
            tally.out_of_scope,
            tally.out_of_scope_silent,
            tally.abstentions,
            median(&mut tally.cold_ms),
            median(&mut tally.warm_ms),
        );
    }
    if let Ok(path) = std::env::var("STILLTRUE_EFFECTIVENESS_OUT") {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({"cases": records})).unwrap(),
        )
        .unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn both_splits_cover_every_kind_with_a_near_miss_and_an_out_of_scope_case() {
    let cases = cases();
    for split in ["dev", "holdout"] {
        let here: Vec<&Case> = cases.iter().filter(|c| c.split == split).collect();
        for kind in KINDS {
            assert!(
                here.iter().any(|c| c.kind == *kind && !c.expect.is_empty()),
                "{split} has no `{kind}` case"
            );
        }
        assert!(
            here.iter()
                .filter(|c| c.in_scope && c.expect.is_empty())
                .count()
                >= 4,
            "{split} needs near-misses"
        );
        assert!(
            here.iter().any(|c| !c.in_scope),
            "{split} needs an out-of-scope case, to show the boundary"
        );
        assert!(
            here.iter().all(|c| c.in_scope || c.expect.is_empty()),
            "an out-of-scope case cannot expect a finding"
        );
    }
}

#[test]
fn the_families_do_not_share_a_base() {
    // A held-out set that is the development set respelled holds nothing out.
    let base_of = |case: &Case| {
        std::fs::read_to_string(&case.script)
            .unwrap()
            .lines()
            .find_map(|l| {
                l.strip_prefix("source \"$(dirname \"$0\")/../../")
                    .map(str::to_string)
            })
            .unwrap()
    };
    let cases = cases();
    let bases = |split: &str| -> std::collections::BTreeSet<String> {
        cases
            .iter()
            .filter(|c| c.split == split)
            .map(base_of)
            .collect()
    };
    assert!(bases("dev").is_disjoint(&bases("holdout")));
}
