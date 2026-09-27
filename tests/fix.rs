//! Seam: fix plans — preview and rewrite from one plan (ADR-0022).
//!
//! `git apply` is the judge of the preview throughout: a diff that git cannot apply to
//! the file it was computed from is not a preview of anything, and a diff that applies
//! to something other than what `--fix` writes is a promise broken.

use std::path::{Path, PathBuf};
use std::process::Command;

use stilltrue::claim::{Claim, ClaimKind};
use stilltrue::fix::{self, EditStatus, Rejection};
use stilltrue::gate::{Finding, Tier};

/// A path finding in `file` whose only suggestion is `to`, located by searching for
/// `text` in `source` — so spans are real byte offsets, multi-byte text included.
fn finding(file: &str, source: &str, text: &str, to: &str) -> Finding {
    let start = source.find(&format!("`{text}`")).expect("claim present") + 1;
    finding_at(file, start..start + text.len(), text, to)
}

fn finding_at(file: &str, span: std::ops::Range<usize>, text: &str, to: &str) -> Finding {
    Finding {
        claim: Claim {
            kind: ClaimKind::Path,
            text: text.to_string(),
            file: PathBuf::from(file),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
            span,
        },
        tier: Tier::Rot,
        rule_id: "stilltrue/path/rot".into(),
        message: format!("path `{text}` does not exist"),
        breaking_commit: None,
        suggestions: vec![to.to_string()],
    }
}

fn write(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Apply `diff` with git in a fresh copy of `original`, and return the result.
fn git_apply(name: &str, original: &str, diff: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), name, original);
    let patch = dir.path().join("preview.patch");
    std::fs::write(&patch, diff).unwrap();
    for check in [true, false] {
        let mut command = Command::new("git");
        command.current_dir(dir.path()).arg("apply");
        if check {
            command.arg("--check");
        }
        let output = command.arg(&patch).output().expect("run git apply");
        assert!(
            output.status.success(),
            "git apply refused the preview:\n{diff}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::read_to_string(dir.path().join(name)).unwrap()
}

/// Plan, preview and apply one document, asserting the preview and the rewrite agree.
fn round_trip(name: &str, source: &str, findings: &[Finding]) -> String {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), name, source);
    let plan = fix::plan(findings, dir.path());
    let preview = fix::diff(&plan);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(name)).unwrap(),
        source,
        "planning wrote something"
    );
    let records = fix::apply(&plan, dir.path());
    assert!(
        records.iter().all(|r| r.status == EditStatus::Applied),
        "{records:?}"
    );
    let written = std::fs::read_to_string(dir.path().join(name)).unwrap();
    assert_eq!(
        git_apply(name, source, &preview),
        written,
        "preview != rewrite"
    );
    written
}

#[test]
fn a_single_edit_round_trips() {
    let source = "# Guide\n\nSee `docs/gude.md` for more.\n";
    let written = round_trip(
        "README.md",
        source,
        &[finding(
            "README.md",
            source,
            "docs/gude.md",
            "docs/guide.md",
        )],
    );
    assert_eq!(written, "# Guide\n\nSee `docs/guide.md` for more.\n");
}

#[test]
fn non_ascii_spans_survive() {
    // Byte spans, not characters: `é`, `☕` and `—` are two, three and three bytes, and
    // an edit placed by character count would cut one in half.
    let source = "Café ☕ — see `docs/gúde.md` for détails.\nNext line — ünchanged.\n";
    let written = round_trip(
        "README.md",
        source,
        &[finding(
            "README.md",
            source,
            "docs/gúde.md",
            "docs/guíde.md",
        )],
    );
    assert_eq!(
        written,
        "Café ☕ — see `docs/guíde.md` for détails.\nNext line — ünchanged.\n"
    );
}

#[test]
fn a_missing_trailing_newline_is_preserved() {
    let source = "Intro.\n\nThe last line names `old.md`";
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", source);
    let findings = [finding("README.md", source, "old.md", "new.md")];
    let plan = fix::plan(&findings, dir.path());
    assert!(
        fix::diff(&plan).contains("\\ No newline at end of file"),
        "{}",
        fix::diff(&plan)
    );
    let written = round_trip("README.md", source, &findings);
    assert_eq!(written, "Intro.\n\nThe last line names `new.md`");
}

#[test]
fn crlf_lines_keep_their_carriage_returns() {
    let source = "One.\r\nSee `a.md`.\r\nThree.\r\n";
    let written = round_trip(
        "README.md",
        source,
        &[finding("README.md", source, "a.md", "b.md")],
    );
    assert_eq!(written, "One.\r\nSee `b.md`.\r\nThree.\r\n");
}

#[test]
fn distant_edits_become_separate_hunks_and_near_ones_share() {
    let mut source = String::from("See `a.md`.\n");
    for i in 0..20 {
        source.push_str(&format!("filler {i}\n"));
    }
    source.push_str("See `b.md`.\nAnd `c.md` right after.\n");
    let findings = [
        finding("README.md", &source, "a.md", "a2.md"),
        finding("README.md", &source, "b.md", "b2.md"),
        finding("README.md", &source, "c.md", "c2.md"),
    ];
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", &source);
    let preview = fix::diff(&fix::plan(&findings, dir.path()));
    assert_eq!(
        preview.matches("\n@@ ").count() + usize::from(preview.starts_with("@@")),
        2,
        "{preview}"
    );
    let written = round_trip("README.md", &source, &findings);
    assert!(
        written.contains("`a2.md`") && written.contains("`b2.md`") && written.contains("`c2.md`")
    );
}

#[test]
fn two_edits_on_one_line_round_trip() {
    let source = "Compare `x.md` with `y.md` here.\n";
    let written = round_trip(
        "README.md",
        source,
        &[
            finding("README.md", source, "x.md", "xx.md"),
            finding("README.md", source, "y.md", "yy.md"),
        ],
    );
    assert_eq!(written, "Compare `xx.md` with `yy.md` here.\n");
}

#[test]
fn overlapping_edits_are_both_rejected_and_the_rest_applied() {
    let source = "abcdefghijklmnop and `keep.md`\n";
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", source);
    let findings = [
        finding_at("README.md", 2..8, &source[2..8], "one"),
        finding_at("README.md", 5..11, &source[5..11], "two"),
        finding("README.md", source, "keep.md", "kept.md"),
    ];
    let plan = fix::plan(&findings, dir.path());
    let overlapping: Vec<_> = plan
        .rejected
        .iter()
        .filter(|r| r.status == EditStatus::Rejected(Rejection::Overlap))
        .collect();
    assert_eq!(overlapping.len(), 2, "{:?}", plan.rejected);
    fix::apply(&plan, dir.path());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "abcdefghijklmnop and `kept.md`\n"
    );
}

#[test]
fn a_span_that_no_longer_holds_the_claim_is_rejected_at_planning() {
    let source = "See `docs/gude.md`.\n";
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", source);
    let stale = finding_at("README.md", 0..12, "docs/gude.md", "docs/guide.md");
    let plan = fix::plan(&[stale], dir.path());
    assert!(plan.files.is_empty());
    assert_eq!(
        plan.rejected[0].status,
        EditStatus::Rejected(Rejection::SpanMismatch)
    );
}

#[test]
fn a_document_changed_between_plan_and_apply_is_not_written() {
    let source = "See `docs/gude.md`.\n";
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", source);
    let plan = fix::plan(
        &[finding(
            "README.md",
            source,
            "docs/gude.md",
            "docs/guide.md",
        )],
        dir.path(),
    );
    // Someone else edits the document — even somewhere the edit does not touch.
    let concurrent = "See `docs/gude.md`.\nA line added by someone else.\n";
    write(dir.path(), "README.md", concurrent);
    let records = fix::apply(&plan, dir.path());
    assert_eq!(records[0].status, EditStatus::Rejected(Rejection::Changed));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("README.md")).unwrap(),
        concurrent,
        "a concurrent edit was overwritten"
    );
}

#[test]
fn one_changed_document_does_not_stop_another_from_being_fixed() {
    let a = "See `x.md`.\n";
    let b = "See `y.md`.\n";
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", a);
    write(dir.path(), "b.md", b);
    let plan = fix::plan(
        &[
            finding("a.md", a, "x.md", "xx.md"),
            finding("b.md", b, "y.md", "yy.md"),
        ],
        dir.path(),
    );
    write(dir.path(), "a.md", "Changed.\n");
    let records = fix::apply(&plan, dir.path());
    let status_of = |file: &str| {
        records
            .iter()
            .find(|r| r.edit.file == Path::new(file))
            .unwrap()
            .status
    };
    assert_eq!(status_of("a.md"), EditStatus::Rejected(Rejection::Changed));
    assert_eq!(status_of("b.md"), EditStatus::Applied);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.md")).unwrap(),
        "See `yy.md`.\n"
    );
}

#[test]
fn the_same_rewrite_asked_twice_is_one_edit() {
    let source = "See `docs/gude.md`.\n";
    let once = finding("README.md", source, "docs/gude.md", "docs/guide.md");
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", source);
    let plan = fix::plan(&[once.clone(), once], dir.path());
    assert_eq!(plan.files[0].edits.len(), 1);
    assert!(plan.rejected.is_empty());
}

#[test]
fn a_finding_with_two_candidates_is_not_planned_at_all() {
    let source = "See `docs/gude.md`.\n";
    let mut two = finding("README.md", source, "docs/gude.md", "docs/guide.md");
    two.suggestions.push("docs/guides.md".into());
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", source);
    assert!(fix::plan(&[two], dir.path()).is_empty());
}

#[test]
fn applying_keeps_permissions_and_leaves_no_temporary_file() {
    use std::os::unix::fs::PermissionsExt;
    let source = "See `docs/gude.md`.\n";
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", source);
    let path = dir.path().join("README.md");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let plan = fix::plan(
        &[finding(
            "README.md",
            source,
            "docs/gude.md",
            "docs/guide.md",
        )],
        dir.path(),
    );
    fix::apply(&plan, dir.path());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["README.md"]);
}

// ---------------------------------------------------------------------------------
// Through the binary.
// ---------------------------------------------------------------------------------

fn build(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = Command::new("bash")
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name)
                .join("build.sh"),
        )
        .arg(dir.path().join("repo"))
        .output()
        .unwrap();
    assert!(output.status.success());
    dir
}

fn stilltrue(repo: &Path, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_stilltrue"))
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The diff out of `--fix-dry-run`'s stderr: every line that is not one of the tool's
/// own `stilltrue:` lines.
fn preview_of(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| !l.starts_with("stilltrue: "))
        .map(|l| format!("{l}\n"))
        .collect()
}

#[test]
fn preview_leaves_the_tree_unchanged_and_exits_on_what_it_found() {
    let dir = build("fix-every-claim-type");
    let repo = dir.path().join("repo");
    let (_, stderr, code) = stilltrue(&repo, &["--fix-dry-run"]);
    assert_eq!(code, 1, "the findings decide the exit code");
    assert_eq!(
        git(&repo, &["status", "--porcelain"]),
        "",
        "the preview wrote"
    );
    assert!(stderr.contains("nothing was written"));
    assert!(preview_of(&stderr).contains("--- a/README.md"), "{stderr}");
}

#[test]
fn the_cli_preview_applies_with_git_and_equals_what_fix_writes() {
    let previewed = build("fix-every-claim-type");
    let fixed = build("fix-every-claim-type");
    let (a, b) = (previewed.path().join("repo"), fixed.path().join("repo"));

    let (_, stderr, _) = stilltrue(&a, &["--fix-dry-run"]);
    let patch = previewed.path().join("preview.patch");
    std::fs::write(&patch, preview_of(&stderr)).unwrap();
    git(&a, &["apply", "--check", patch.to_str().unwrap()]);
    git(&a, &["apply", patch.to_str().unwrap()]);

    let (_, _, code) = stilltrue(&b, &["--fix"]);
    assert_eq!(
        code, 1,
        "--fix exits on what it found, not on what it changed"
    );

    let changed = git(&b, &["diff", "--name-only"]);
    assert!(!changed.is_empty(), "--fix changed nothing");
    assert_eq!(git(&a, &["diff"]), git(&b, &["diff"]), "preview != rewrite");

    // And the rerun no longer finds what was fixed.
    let (stdout, _, code) = stilltrue(&b, &[]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("stilltrue: no findings"), "{stdout}");
}

#[test]
fn fix_and_fix_dry_run_are_mutually_exclusive() {
    let dir = build("fix-every-claim-type");
    let (_, stderr, code) = stilltrue(&dir.path().join("repo"), &["--fix", "--fix-dry-run"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

#[test]
fn the_report_lists_eligible_applied_and_rejected_edits() {
    let dir = build("fix-every-claim-type");
    let repo = dir.path().join("repo");
    let report = dir.path().join("report.json");
    let read = || -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap()
    };

    let (_, stderr, _) = stilltrue(
        &repo,
        &["--fix-dry-run", "--report-file", report.to_str().unwrap()],
    );
    let preview = read();
    let eligible = preview["edits"]["eligible"].as_u64().unwrap();
    assert!(eligible >= 3, "{}", preview["edits"]);
    // The line a person reads says the same as the report a machine reads.
    let findings = preview["findings"].as_array().unwrap().len();
    assert!(
        stderr.contains(&format!(
            "would rewrite {eligible} of {findings} findings; nothing was written"
        )),
        "{stderr}"
    );
    assert_eq!(preview["edits"]["applied"], 0);
    assert!(
        preview["edits"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "eligible" && r["reason"].is_null())
    );

    stilltrue(&repo, &["--fix", "--report-file", report.to_str().unwrap()]);
    let applied = read();
    assert_eq!(applied["edits"]["applied"].as_u64().unwrap(), eligible);
    assert_eq!(applied["edits"]["eligible"], 0);

    // Without either flag, the report says no edits were planned rather than zero.
    stilltrue(&repo, &["--report-file", report.to_str().unwrap()]);
    assert!(read()["edits"].is_null());
}

// ---------------------------------------------------------------------------------
// The diff against an independent implementation, and the plan's own edges.
// ---------------------------------------------------------------------------------

/// Unified diffs from Python's difflib, which shares no code with this crate and groups
/// hunks by the same three lines of context. `git apply` is not enough of a judge on its
/// own: it tolerates a hunk header that is off by a line.
fn difflib(pairs: &[(String, String)]) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("pairs.json");
    std::fs::write(&input, serde_json::to_string(pairs).unwrap()).unwrap();
    let script = "import difflib, json, sys\n\
        pairs = json.load(open(sys.argv[1]))\n\
        print(json.dumps([''.join(difflib.unified_diff(a.splitlines(True), \
        b.splitlines(True), 'a/README.md', 'b/README.md', n=3)) for a, b in pairs]))";
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(&input)
        .output()
        .expect("python3, the oracle");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// A small deterministic generator, so a failing layout can be reproduced.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % n
    }
}

/// One document and a set of non-overlapping edits to it, of three shapes: a word inside
/// a line, a word at the very start of a line, and a span running across a line break
/// replaced by one line — which shifts every later hunk up by one.
fn layout(rng: &mut Lcg) -> (String, Vec<Finding>) {
    let count = 5 + rng.below(40);
    let source: String = (0..count)
        .map(|i| format!("word{i} alpha{i} beta{i}\n"))
        .collect();
    let starts: Vec<usize> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let mut findings = Vec::new();
    let mut line = rng.below(3);
    while line < count {
        let base = starts[line];
        let text = &source[base..starts.get(line + 1).copied().unwrap_or(source.len())];
        let (span, to) = match rng.below(3) {
            0 => {
                let at = base + text.find("alpha").unwrap();
                let end = at + format!("alpha{line}").len();
                (at..end, format!("ALPHA{line}"))
            }
            1 => {
                let end = base + format!("word{line}").len();
                (base..end, format!("WORD{line}"))
            }
            _ if line + 1 < count => {
                let at = base + text.find("beta").unwrap();
                let next = starts[line + 1];
                let end = next + format!("word{}", line + 1).len();
                line += 1; // the next line is taken
                (at..end, format!("JOINED{line}"))
            }
            _ => break,
        };
        findings.push(finding_at("README.md", span.clone(), &source[span], &to));
        // Gaps on both sides of the hunk-merging threshold of 2 × 3 lines.
        line += 1 + [0, 1, 5, 6, 7, 8, 12][rng.below(7)];
    }
    (source, findings)
}

#[test]
fn the_diff_matches_difflib_across_generated_layouts() {
    let mut rng = Lcg(0x5717_7a0e);
    let mut ours = Vec::new();
    let mut pairs = Vec::new();
    for _ in 0..300 {
        let (source, findings) = layout(&mut rng);
        if findings.is_empty() {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", &source);
        let plan = fix::plan(&findings, dir.path());
        assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
        ours.push(fix::diff(&plan));
        pairs.push((source, fix::render(&plan.files[0])));
    }
    // And a one-line document, where the ranges are a single line each: `@@ -1 +1 @@`.
    for source in ["See `a.md`.\n", "See `a.md` and `b.md`.\n"] {
        let findings = [finding("README.md", source, "a.md", "c.md")];
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", source);
        let plan = fix::plan(&findings, dir.path());
        ours.push(fix::diff(&plan));
        pairs.push((source.to_string(), fix::render(&plan.files[0])));
    }
    assert!(
        pairs.len() > 250,
        "only {} layouts were compared",
        pairs.len()
    );
    let expected = difflib(&pairs);
    for (i, (mine, theirs)) in ours.iter().zip(&expected).enumerate() {
        assert_eq!(mine, theirs, "layout {i}:\n{}", pairs[i].0);
    }
}

#[test]
fn rejection_and_status_codes_are_the_published_ones() {
    let rejections = [
        (Rejection::Overlap, "overlap"),
        (Rejection::Unreadable, "unreadable"),
        (Rejection::SpanMismatch, "span-mismatch"),
        (Rejection::Changed, "changed"),
        (Rejection::WriteFailed, "write-failed"),
    ];
    for (rejection, code) in rejections {
        assert_eq!(rejection.code(), code);
        assert_eq!(EditStatus::Rejected(rejection).code(), "rejected");
    }
    assert_eq!(EditStatus::Eligible.code(), "eligible");
    assert_eq!(EditStatus::Applied.code(), "applied");
}

#[test]
fn a_plan_is_empty_only_when_it_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let source = "See `a.md`.\n";
    write(dir.path(), "README.md", source);
    assert!(fix::plan(&[], dir.path()).is_empty());
    let planned = fix::plan(&[finding("README.md", source, "a.md", "b.md")], dir.path());
    assert!(!planned.is_empty(), "a plan with an edit");
    let refused = fix::plan(&[finding_at("README.md", 0..4, "a.md", "b.md")], dir.path());
    assert!(refused.files.is_empty());
    assert!(!refused.is_empty(), "a plan with only a rejection");
}

#[test]
fn records_come_back_ordered_by_file_then_position() {
    let dir = tempfile::tempdir().unwrap();
    let a = "One `x.md` and `y.md`.\n";
    let b = "Then `z.md`.\n";
    write(dir.path(), "a.md", a);
    write(dir.path(), "b.md", b);
    let findings = [
        finding("b.md", b, "z.md", "zz.md"),
        finding("a.md", a, "y.md", "yy.md"),
        finding("a.md", a, "x.md", "xx.md"),
    ];
    let order = |records: &[fix::EditRecord]| -> Vec<String> {
        records.iter().map(|r| r.edit.expected.clone()).collect()
    };
    let plan = fix::plan(&findings, dir.path());
    assert_eq!(order(&plan.records()), ["x.md", "y.md", "z.md"]);
    assert_eq!(
        order(&fix::apply(&plan, dir.path())),
        ["x.md", "y.md", "z.md"]
    );

    // Rejections are gathered apart from eligible edits, so only sorting interleaves
    // them: a rejected edit in the first file still comes before the second file.
    let c = "Here `p.md` and `q.md`.\n";
    let d = "There `r.md`.\n";
    write(dir.path(), "c.md", c);
    write(dir.path(), "d.md", d);
    let start = c.find("p.md").unwrap();
    let mixed = [
        finding("d.md", d, "r.md", "rr.md"),
        finding_at("c.md", start..start + 4, "p.md", "one"),
        finding_at("c.md", start + 1..start + 4, ".md", "two"),
        finding("c.md", c, "q.md", "qq.md"),
    ];
    let plan = fix::plan(&mixed, dir.path());
    assert_eq!(plan.rejected.len(), 2, "the overlapping pair");
    assert_eq!(order(&plan.records()), ["p.md", ".md", "q.md", "r.md"]);
    assert_eq!(
        order(&fix::apply(&plan, dir.path())),
        ["p.md", ".md", "q.md", "r.md"]
    );
}

#[test]
fn edits_that_touch_without_overlapping_are_both_made() {
    let dir = tempfile::tempdir().unwrap();
    let source = "aaaaaabbbbbb\n";
    write(dir.path(), "README.md", source);
    let plan = fix::plan(
        &[
            finding_at("README.md", 0..6, "aaaaaa", "A"),
            finding_at("README.md", 6..12, "bbbbbb", "B"),
        ],
        dir.path(),
    );
    assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
    fix::apply(&plan, dir.path());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "AB\n"
    );
}

#[test]
fn an_edit_that_only_touches_an_overlapping_pair_is_kept() {
    // x ends where y and z begin; y and z overlap each other, not x.
    let dir = tempfile::tempdir().unwrap();
    let source = "xxxxxxyyyyyyyy\n";
    write(dir.path(), "README.md", source);
    let plan = fix::plan(
        &[
            finding_at("README.md", 0..6, "xxxxxx", "X"),
            finding_at("README.md", 6..8, "yy", "Z"),
            finding_at("README.md", 6..10, "yyyy", "Y"),
        ],
        dir.path(),
    );
    let rejected: Vec<&str> = plan
        .rejected
        .iter()
        .map(|r| r.edit.expected.as_str())
        .collect();
    assert_eq!(rejected, ["yy", "yyyy"]);
    assert_eq!(plan.files[0].edits.len(), 1);
    assert_eq!(plan.files[0].edits[0].expected, "xxxxxx");
}
