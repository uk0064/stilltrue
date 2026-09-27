//! The demonstration, as a test.
//!
//! The first block of output in README.md is the tool's advertisement. This builds the
//! repository it describes — a documented `make demo` whose target was split in two —
//! runs the real binary, and compares. Then it does what the quickstart tells a reader
//! to do: correct the document and run again, and get a complete, clean run.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn build(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = Command::new("bash")
        .arg(root().join("tests/fixtures").join(name).join("build.sh"))
        .arg(dir.path().join("repo"))
        .output()
        .expect("run build.sh");
    assert!(output.status.success(), "{name}/build.sh failed");
    dir
}

fn stilltrue(repo: &Path, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_stilltrue"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run stilltrue");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// The first fenced block in a Markdown file.
fn first_fence(markdown: &str) -> String {
    let mut lines = markdown.lines().skip_while(|l| !l.starts_with("```"));
    lines.next().expect("an opening fence");
    lines
        .take_while(|l| !l.starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A commit sha and a relative date are the only parts of a finding that depend on
/// when and where it was produced.
fn normalise(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = match line.find("likely broke in ") {
            Some(at) => {
                let (head, tail) = line.split_at(at + "likely broke in ".len());
                let tail = tail.split_once(' ').map(|(_, rest)| rest).unwrap_or("");
                let tail = match tail.rfind(" · ") {
                    Some(dot) => format!("{} · <when>", &tail[..dot]),
                    None => tail.to_string(),
                };
                format!("{head}<sha> {tail}")
            }
            None => line.to_string(),
        };
        out.push(line.trim_end().to_string());
    }
    out.join("\n")
}

#[test]
fn the_readme_example_is_what_the_tool_prints() {
    let readme = std::fs::read_to_string(root().join("README.md")).unwrap();
    let advertised = normalise(&first_fence(&readme));
    let dir = build("readme-demo");
    let (stdout, _, code) = stilltrue(&dir.path().join("repo"), &[]);
    assert_eq!(code, 1, "the example is rot, and rot fails the build");
    let printed: String = stdout
        .lines()
        .take_while(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(normalise(&printed), advertised);
}

#[test]
fn the_quickstart_walkthrough_ends_in_a_complete_clean_run() {
    let dir = build("readme-demo");
    let repo = dir.path().join("repo");

    // Observe the finding.
    let (stdout, _, code) = stilltrue(&repo, &["--summary"]);
    assert_eq!(code, 1);
    assert!(stdout.contains("did you mean: make seed, make serve?"));

    // Correct the document: the reader chooses between the two candidates, because
    // two is a choice and `--fix` does not make choices (ADR-0015).
    let claude = repo.join("CLAUDE.md");
    let text = std::fs::read_to_string(&claude).unwrap();
    std::fs::write(&claude, text.replace("`make demo`", "`make seed`")).unwrap();

    // Observe a clean rerun — and a complete one, which is what makes it clean.
    let report = repo.join("report.json");
    let (stdout, stderr, code) = stilltrue(
        &repo,
        &["--summary", "--report-file", report.to_str().unwrap()],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("stilltrue: no findings"));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(report["status"], "complete");
    assert_eq!(report["coverage"]["claims"]["true"], 1);
    assert!(
        stderr.contains("No reportable rot. Checked 1 document(s)."),
        "{stderr}"
    );
}
