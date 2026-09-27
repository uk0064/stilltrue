//! Run the checker as a child process, and read what it reported.
//!
//! A child rather than a library call, so that a scan that overruns its budget can be
//! killed rather than waited for, and so the adapter judges the same binary a user or CI
//! runs. Every way the child can fail — not found, too slow, a bad report, an exit
//! status of its own — becomes a `Completion` that can only ever be advised, never a
//! request to continue.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use crate::hook::policy::{Completion, Finding};

/// What one scan found, and how it ended.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub completion: Completion,
    pub findings: Vec<Finding>,
    /// Documents the checker examined, when it said.
    pub documents: u64,
}

impl Outcome {
    fn without_report(completion: Completion) -> Self {
        Self {
            completion,
            findings: Vec::new(),
            documents: 0,
        }
    }
}

/// The process group of the checker running now, or 0.
///
/// The checker runs in a group of its own so that abandoning it kills everything it
/// started — the git processes a scan spawns included — rather than only the checker,
/// which left them running after the adapter had answered. The adapter's signal handler
/// reads this too, so a host that terminates the adapter mid-scan does not leave the
/// scan behind.
pub static RUNNING: AtomicI32 = AtomicI32::new(0);

/// Kill a process group outright. Only ever called for a group whose leader has not
/// been waited for, so its id cannot have been reused by an unrelated process.
pub fn kill_group(group: i32) {
    #[cfg(unix)]
    if group > 0 {
        // SAFETY: killpg has no memory-safety preconditions; a stale or absent group
        // is reported through its return value, which there is nothing to do about.
        unsafe {
            libc::killpg(group, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = group;
}

/// The checker binary: an explicit path, else `stilltrue` beside this executable, else
/// the first on PATH.
pub fn locate(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        return explicit.is_file().then(|| explicit.to_path_buf());
    }
    let name = format!("stilltrue{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|p| p.is_file());
    beside.or_else(|| {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(&name))
                .find(|p| p.is_file())
        })
    })
}

/// Run `checker` over `root` with a deadline, writing its report to `report`.
pub fn run(
    checker: Option<&Path>,
    root: &Path,
    baseline: Option<&Path>,
    report: &Path,
    timeout: Duration,
) -> Outcome {
    let Some(checker) = checker else {
        return Outcome::without_report(Completion::Missing);
    };
    if let Some(parent) = report.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // A report left from an earlier scan must not be read as this one's.
    let _ = std::fs::remove_file(report);

    let mut command = Command::new(checker);
    command
        .current_dir(root)
        .args(["--format", "json", "--fail-on", "none", "--report-file"])
        .arg(report)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(baseline) = baseline {
        command.arg("--baseline").arg(baseline);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let Ok(mut child) = command.spawn() else {
        return Outcome::without_report(Completion::Missing);
    };
    let group = i32::try_from(child.id()).unwrap_or(0);
    RUNNING.store(group, Ordering::SeqCst);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // The whole group, before the leader is reaped: the checker and every
                // git process it had running.
                kill_group(group);
                let _ = child.kill();
                let _ = child.wait();
                RUNNING.store(0, Ordering::SeqCst);
                return Outcome::without_report(Completion::TimedOut);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                kill_group(group);
                RUNNING.store(0, Ordering::SeqCst);
                return Outcome::without_report(Completion::Failed(-1));
            }
        }
    };
    RUNNING.store(0, Ordering::SeqCst);
    // 0 is clean and 1 is findings; `--fail-on none` makes 1 unexpected, but it is still
    // a run that wrote a report. Anything else — 2 above all — is the checker failing.
    match status.code() {
        Some(0 | 1) => read(report),
        Some(code) => Outcome::without_report(Completion::Failed(code)),
        None => Outcome::without_report(Completion::Failed(-1)),
    }
}

/// Read a run report, refusing anything that is not version 1 of the envelope.
pub fn read(report: &Path) -> Outcome {
    let Some(value) = std::fs::read_to_string(report)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    else {
        return Outcome::without_report(Completion::Malformed);
    };
    if value["schema"] != "stilltrue/run-report" || value["version"] != 1 {
        return Outcome::without_report(Completion::Malformed);
    }
    let completion = match value["status"].as_str() {
        Some("complete") => Completion::Complete,
        Some("no-input") => Completion::NoInput,
        Some("incomplete") => Completion::Incomplete(
            value["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| d["incomplete"] == true)
                .filter_map(|d| d["code"].as_str().map(str::to_string))
                .collect(),
        ),
        _ => return Outcome::without_report(Completion::Malformed),
    };
    let Some(findings) = value["findings"].as_array() else {
        return Outcome::without_report(Completion::Malformed);
    };
    let mut parsed = Vec::with_capacity(findings.len());
    for f in findings {
        let text = |key: &str| f[key].as_str().map(str::to_string);
        let (Some(fingerprint), Some(rule), Some(tier), Some(file)) = (
            text("fingerprint"),
            text("ruleId"),
            text("tier"),
            text("file"),
        ) else {
            return Outcome::without_report(Completion::Malformed);
        };
        parsed.push(Finding {
            fingerprint,
            rule,
            tier,
            file,
            line: f["line"].as_u64().unwrap_or(0),
            claim: text("claim").unwrap_or_default(),
            message: text("message").unwrap_or_default(),
            commit: f["likelyBrokeIn"]["sha"].as_str().map(|sha| {
                format!(
                    "{sha} \"{}\"",
                    f["likelyBrokeIn"]["subject"].as_str().unwrap_or_default()
                )
            }),
        });
    }
    Outcome {
        completion,
        findings: parsed,
        documents: value["coverage"]["documents"]["read"].as_u64().unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(value: serde_json::Value) -> Outcome {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        std::fs::write(&path, value.to_string()).unwrap();
        read(&path)
    }

    fn envelope(status: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": "stilltrue/run-report", "version": 1, "status": status,
            "findings": [], "diagnostics": [{"code": "walk-error", "incomplete": true},
                                            {"code": "no-documents", "incomplete": false}],
            "coverage": {"documents": {"read": 3}},
        })
    }

    #[test]
    fn each_status_is_read_as_itself() {
        assert_eq!(
            report(envelope("complete")).completion,
            Completion::Complete
        );
        assert_eq!(report(envelope("no-input")).completion, Completion::NoInput);
        assert_eq!(
            report(envelope("incomplete")).completion,
            Completion::Incomplete(vec!["walk-error".into()]),
            "only the diagnostics that made it incomplete"
        );
        assert_eq!(report(envelope("complete")).documents, 3);
    }

    #[test]
    fn anything_but_version_one_of_this_envelope_is_malformed() {
        let mut wrong_version = envelope("complete");
        wrong_version["version"] = serde_json::json!(2);
        let mut wrong_schema = envelope("complete");
        wrong_schema["schema"] = serde_json::json!("someone-else/report");
        let mut unknown_status = envelope("complete");
        unknown_status["status"] = serde_json::json!("fine");
        let mut no_findings = envelope("complete");
        no_findings.as_object_mut().unwrap().remove("findings");
        let mut bad_finding = envelope("complete");
        bad_finding["findings"] = serde_json::json!([{"ruleId": "x"}]);
        for (name, value) in [
            ("version", wrong_version),
            ("schema", wrong_schema),
            ("status", unknown_status),
            ("findings", no_findings),
            ("finding", bad_finding),
        ] {
            assert_eq!(report(value).completion, Completion::Malformed, "{name}");
        }
    }

    #[test]
    fn a_finding_carries_its_commit_as_sha_and_subject() {
        let mut value = envelope("complete");
        value["findings"] = serde_json::json!([{
            "fingerprint": "f", "ruleId": "stilltrue/command/rot", "tier": "rot",
            "file": "CLAUDE.md", "line": 3, "claim": "make demo", "message": "m",
            "likelyBrokeIn": {"sha": "abc1234", "subject": "split demo"},
        }]);
        let outcome = report(value);
        assert_eq!(
            outcome.findings[0].commit.as_deref(),
            Some("abc1234 \"split demo\"")
        );
        assert_eq!(outcome.findings[0].line, 3);
    }
}
