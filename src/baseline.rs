//! A recorded set of findings, so a repository with a backlog can adopt the tool
//! without starting red (ADR-0014).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::cache::fnv1a;
use crate::gate::Finding;

const HEADER: &str = "\
# stilltrue baseline — findings known at the time this was written.
# New rot is still reported; these are not. Delete a line to start judging it again.
# Fingerprints carry no line number, so reflowing a document does not change them.
";

/// A stable identity for a finding: rule id, claim text, path, and which occurrence of
/// that triple this is.
///
/// No line number, deliberately. A fingerprint that moved when a paragraph was rewrapped
/// would un-suppress a whole document on a whitespace change, and a baseline that churns
/// is a baseline nobody keeps.
pub fn fingerprints(findings: &[Finding]) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    findings
        .iter()
        .map(|f| {
            let identity = format!(
                "{}|{}|{}",
                f.rule_id,
                f.claim.text,
                f.claim.file.to_string_lossy()
            );
            let ordinal = seen.entry(identity.clone()).or_insert(0);
            let hash = fnv1a(&format!("{identity}|{ordinal}"));
            *ordinal += 1;
            hash
        })
        .collect()
}

/// Read a baseline. A missing or unreadable file is not an error: it warns and judges
/// everything, which is the safe direction.
pub fn load(path: &Path) -> BTreeSet<String> {
    try_load(path).unwrap_or_else(|error| {
        eprintln!(
            "stilltrue: warning: could not read baseline {}: {error}",
            path.display()
        );
        BTreeSet::new()
    })
}

/// Read a baseline, saying why when it cannot be read.
pub fn try_load(path: &Path) -> std::io::Result<BTreeSet<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// Write one, sorted so the file is diffable and a review shows one line appearing.
pub fn write(path: &Path, findings: &[Finding]) -> std::io::Result<usize> {
    let hashes: BTreeSet<String> = fingerprints(findings).into_iter().collect();
    let mut out = String::from(HEADER);
    for hash in &hashes {
        out.push_str(hash);
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(hashes.len())
}

/// Split findings into those the baseline does not know, each with its fingerprint,
/// and a count of those it does.
///
/// Fingerprints are computed over the whole set, exactly as `write` computed them, so a
/// shown finding carries the identity a baseline would record for it — not one
/// renumbered by whatever the baseline happened to hide.
pub fn partition(
    findings: &[Finding],
    baseline: &BTreeSet<String>,
) -> (Vec<(Finding, String)>, usize) {
    let mut suppressed = 0;
    let shown = findings
        .iter()
        .zip(fingerprints(findings))
        .filter(|(_, hash)| {
            let known = baseline.contains(hash);
            suppressed += usize::from(known);
            !known
        })
        .map(|(finding, hash)| (finding.clone(), hash))
        .collect();
    (shown, suppressed)
}

/// Drop every finding the baseline already knows about.
pub fn apply(findings: Vec<Finding>, baseline: &BTreeSet<String>) -> Vec<Finding> {
    let hashes = fingerprints(&findings);
    findings
        .into_iter()
        .zip(hashes)
        .filter(|(_, hash)| !baseline.contains(hash))
        .map(|(finding, _)| finding)
        .collect()
}
