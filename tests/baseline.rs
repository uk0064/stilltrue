//! Seam: a baseline is the set of fingerprints a repository has agreed to live with
//! (ADR-0014). Its whole value rests on two properties — that two occurrences of one
//! claim are told apart, and that a reflow does not renumber anything — so those are
//! what these pin.

use std::path::{Path, PathBuf};

use stilltrue::baseline;
use stilltrue::claim::{Claim, ClaimKind};
use stilltrue::gate::{Finding, Tier};

fn finding(text: &str, file: &str, line: usize) -> Finding {
    Finding {
        claim: Claim {
            kind: ClaimKind::Command {
                runner: "make".into(),
                args: vec![text.split_whitespace().nth(1).unwrap_or("").to_string()],
            },
            text: text.to_string(),
            file: PathBuf::from(file),
            line,
            column: 6,
            end_line: line,
            end_column: 6 + text.chars().count(),
            span: 0..text.len(),
        },
        tier: Tier::Rot,
        rule_id: "stilltrue/command/rot".into(),
        message: format!("command `{text}` has no target in Makefile"),
        breaking_commit: None,
        suggestions: vec![],
    }
}

#[test]
fn two_occurrences_of_one_claim_get_different_fingerprints() {
    // Without the occurrence ordinal these hash identically, and GitHub code scanning
    // silently drops the second as a duplicate — the finding disappears with no sign
    // that it ever existed.
    let findings = vec![
        finding("make demo", "README.md", 3),
        finding("make demo", "README.md", 5),
        finding("make demo", "README.md", 9),
    ];
    let prints = baseline::fingerprints(&findings);
    assert_eq!(prints.len(), 3);
    let distinct: std::collections::BTreeSet<_> = prints.iter().collect();
    assert_eq!(distinct.len(), 3, "ordinals must disambiguate: {prints:?}");
}

#[test]
fn a_fingerprint_ignores_the_line_number() {
    // The reason a baseline is not a list of line numbers: reflowing a paragraph would
    // otherwise un-suppress every finding below the edit.
    let before = baseline::fingerprints(&[finding("make demo", "README.md", 3)]);
    let after = baseline::fingerprints(&[finding("make demo", "README.md", 400)]);
    assert_eq!(before, after, "a moved claim is the same claim");
}

#[test]
fn a_fingerprint_separates_claims_that_differ_in_any_other_way() {
    let base = finding("make demo", "README.md", 3);
    let other_text = finding("make serve", "README.md", 3);
    let other_file = finding("make demo", "CLAUDE.md", 3);
    let mut other_rule = finding("make demo", "README.md", 3);
    other_rule.rule_id = "stilltrue/command/lie".into();

    let prints: Vec<String> = [base, other_text, other_file, other_rule]
        .iter()
        .map(|f| baseline::fingerprints(std::slice::from_ref(f)).remove(0))
        .collect();
    let distinct: std::collections::BTreeSet<_> = prints.iter().collect();
    assert_eq!(
        distinct.len(),
        4,
        "each field must reach the hash: {prints:?}"
    );
}

#[test]
fn a_baseline_file_is_fingerprints_and_nothing_else() {
    // The file is meant to be readable and reviewable, so it carries a header comment
    // and blank lines. Reading either one back as a fingerprint would silence a real
    // finding whose hash happened to be empty.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline");
    std::fs::write(
        &path,
        "# stilltrue baseline\n\n  abc123  \n\n# another comment\ndef456\n",
    )
    .unwrap();

    let loaded = baseline::load(&path);
    assert_eq!(loaded.len(), 2, "got {loaded:?}");
    assert!(loaded.contains("abc123"), "surrounding space is trimmed");
    assert!(loaded.contains("def456"));
    assert!(
        !loaded.iter().any(|l| l.starts_with('#') || l.is_empty()),
        "comments and blank lines are not fingerprints: {loaded:?}"
    );
}

#[test]
fn a_missing_baseline_judges_everything_rather_than_nothing() {
    // The safe direction. A typo in the path must not silently pass the build.
    let loaded = baseline::load(Path::new("/nonexistent/stilltrue-baseline"));
    assert!(loaded.is_empty());

    let findings = vec![finding("make demo", "README.md", 3)];
    let kept = baseline::apply(findings, &loaded);
    assert_eq!(kept.len(), 1, "an unreadable baseline suppresses nothing");
}

#[test]
fn a_baseline_suppresses_exactly_what_it_recorded() {
    let recorded = vec![
        finding("make demo", "README.md", 3),
        finding("make demo", "README.md", 5),
    ];
    let set: std::collections::BTreeSet<String> =
        baseline::fingerprints(&recorded).into_iter().collect();

    // The same two, reflowed, plus one that is genuinely new.
    let now = vec![
        finding("make demo", "README.md", 40),
        finding("make demo", "README.md", 52),
        finding("make lint", "README.md", 60),
    ];
    let kept = baseline::apply(now, &set);
    assert_eq!(
        kept.len(),
        1,
        "got {:?}",
        kept.iter().map(|f| &f.claim.text)
    );
    assert_eq!(kept[0].claim.text, "make lint");
}
