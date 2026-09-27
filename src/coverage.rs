//! What a run examined, what it could not, and why (ADR-0019).
//!
//! A quiet run means two different things — every claim was checked and none is
//! rotten, or the run could not look — and before this module the output was the same
//! for both. Every claim extracted lands in exactly one bucket here, and `reconcile`
//! says so out loud, so the run report can distinguish a complete silence from an
//! incomplete one without anyone parsing prose.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::claim::ClaimKind;
use crate::gate::{Judgement, Tier};
use crate::resolution::{AmbiguityReason, Resolution};

/// Whether the run did what it set out to do. Distinct from coverage: a complete run
/// can still have abstained on most of what it read, and says so separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Every selected analysis operation succeeded.
    Complete,
    /// A read, a parse, or a required history operation failed, or history was
    /// degraded. A quiet incomplete run is not a clean one.
    Incomplete,
    /// No document matched. Nothing was examined.
    NoInput,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Status::Complete => "complete",
            Status::Incomplete => "incomplete",
            Status::NoInput => "no-input",
        }
    }

    /// Incomplete wins over no-input: if selection itself failed, "nothing matched" is
    /// not something the run can honestly say.
    pub fn of(diagnostics: &[Diagnostic], history_complete: bool, documents: usize) -> Self {
        if !history_complete || diagnostics.iter().any(|d| d.incomplete) {
            Status::Incomplete
        } else if documents == 0 {
            Status::NoInput
        } else {
            Status::Complete
        }
    }
}

/// An operational issue. Not a finding, and never a second way to report an
/// ambiguous claim — those are counted, by reason, in `Coverage`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    /// Stable, kebab-case, a public interface like a rule id.
    pub code: &'static str,
    pub message: String,
    pub path: Option<PathBuf>,
    /// Whether this issue makes the run incomplete.
    pub incomplete: bool,
}

impl Diagnostic {
    /// Something that should have worked and did not.
    pub fn failure(code: &'static str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            code,
            message: message.into(),
            path,
            incomplete: true,
        }
    }

    /// Worth recording, but not a failure of the run.
    pub fn note(code: &'static str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            code,
            message: message.into(),
            path,
            incomplete: false,
        }
    }
}

/// Where the claims of one kind went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaimCounts {
    pub extracted: usize,
    /// Named in `[ignore]` in `stilltrue.toml`.
    pub ignored: usize,
    pub true_: usize,
    pub broken: usize,
    pub skipped: usize,
    pub ambiguous: usize,
}

impl ClaimCounts {
    fn add(&mut self, other: &ClaimCounts) {
        self.extracted += other.extracted;
        self.ignored += other.ignored;
        self.true_ += other.true_;
        self.broken += other.broken;
        self.skipped += other.skipped;
        self.ambiguous += other.ambiguous;
    }

    fn accounted(&self) -> usize {
        self.ignored + self.true_ + self.broken + self.skipped + self.ambiguous
    }
}

/// Where the broken claims went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GateCounts {
    pub rot: usize,
    pub lie_reported: usize,
    /// Lies the flags do not report: without `--strict`, every one of them.
    pub lie_unreported: usize,
    /// Tier C at the gate: no definition-shaped needle.
    pub abstained: usize,
    /// The history search failed; no tier could be assigned.
    pub history_failed: usize,
}

impl GateCounts {
    pub fn record(&mut self, judgement: &Judgement) {
        match judgement {
            Judgement::Reported(finding) if finding.tier == Tier::Rot => self.rot += 1,
            Judgement::Reported(_) => self.lie_reported += 1,
            Judgement::Unreported(_) => self.lie_unreported += 1,
            Judgement::Abstained => self.abstained += 1,
            Judgement::HistoryFailed => self.history_failed += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.rot + self.lie_reported + self.lie_unreported + self.abstained + self.history_failed
    }

    fn add(&mut self, other: &GateCounts) {
        self.rot += other.rot;
        self.lie_reported += other.lie_reported;
        self.lie_unreported += other.lie_unreported;
        self.abstained += other.abstained;
        self.history_failed += other.history_failed;
    }
}

/// Every count a run can honestly state about what it examined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    pub documents_matched: usize,
    pub documents_read: usize,
    pub documents_unreadable: usize,
    /// Read, but the parser produced nothing to extract from.
    pub documents_unparseable: usize,
    /// Suppression markers found. A suppressed block is not a measured number of
    /// claims — nothing inside it was extracted — so markers are counted instead, and
    /// nothing here claims coverage of all prose.
    pub suppression_markers: usize,
    /// Of those, markers that covered no claim.
    pub suppression_unused: usize,
    /// Documents a whole-file marker removed from extraction.
    pub files_suppressed: usize,
    /// By claim-kind slug.
    pub claims: BTreeMap<&'static str, ClaimCounts>,
    /// `(kind, reason code)` for every skipped claim.
    pub skipped: BTreeMap<(&'static str, &'static str), usize>,
    /// `(kind, reason code)` for every claim that was ambiguous at resolution.
    pub ambiguous: BTreeMap<(&'static str, &'static str), usize>,
    /// `(kind, reason code)` for every broken claim the gate abstained on.
    pub abstained: BTreeMap<(&'static str, &'static str), usize>,
    pub gate: GateCounts,
}

impl Coverage {
    pub fn extracted(&mut self, kind: &ClaimKind) {
        self.claims.entry(kind.slug()).or_default().extracted += 1;
    }

    pub fn ignored(&mut self, kind: &ClaimKind) {
        self.claims.entry(kind.slug()).or_default().ignored += 1;
    }

    /// Record where resolution put one claim.
    pub fn resolved(&mut self, kind: &ClaimKind, resolution: &Resolution) {
        let slug = kind.slug();
        let counts = self.claims.entry(slug).or_default();
        match resolution {
            Resolution::True => counts.true_ += 1,
            Resolution::Broken(_) => counts.broken += 1,
            Resolution::Skip(reason) => {
                counts.skipped += 1;
                *self.skipped.entry((slug, reason.code())).or_default() += 1;
            }
            Resolution::Ambiguous(reason) => {
                counts.ambiguous += 1;
                *self.ambiguous.entry((slug, reason.code())).or_default() += 1;
            }
        }
    }

    /// Record what the gate decided about one broken claim.
    pub fn judged(&mut self, kind: &ClaimKind, judgement: &Judgement) {
        self.gate.record(judgement);
        if matches!(judgement, Judgement::Abstained) {
            *self
                .abstained
                .entry((kind.slug(), AmbiguityReason::NoNeedle.code()))
                .or_default() += 1;
        }
    }

    /// Fold another tally into this one. Addition throughout, so the order documents
    /// are merged in cannot change the result.
    pub fn merge(&mut self, other: Coverage) {
        self.documents_matched += other.documents_matched;
        self.documents_read += other.documents_read;
        self.documents_unreadable += other.documents_unreadable;
        self.documents_unparseable += other.documents_unparseable;
        self.suppression_markers += other.suppression_markers;
        self.suppression_unused += other.suppression_unused;
        self.files_suppressed += other.files_suppressed;
        for (kind, counts) in other.claims {
            self.claims.entry(kind).or_default().add(&counts);
        }
        for (key, n) in other.skipped {
            *self.skipped.entry(key).or_default() += n;
        }
        for (key, n) in other.ambiguous {
            *self.ambiguous.entry(key).or_default() += n;
        }
        for (key, n) in other.abstained {
            *self.abstained.entry(key).or_default() += n;
        }
        self.gate.add(&other.gate);
    }

    /// The claim counts summed across kinds.
    pub fn totals(&self) -> ClaimCounts {
        let mut total = ClaimCounts::default();
        for counts in self.claims.values() {
            total.add(counts);
        }
        total
    }

    /// Check that every claim is in exactly one place. A report whose numbers do not
    /// add up is a report nobody should act on, so this is asserted over every fixture
    /// rather than trusted.
    pub fn reconcile(&self) -> Result<(), String> {
        if self.documents_matched != self.documents_read + self.documents_unreadable {
            return Err(format!(
                "documents: {} matched != {} read + {} unreadable",
                self.documents_matched, self.documents_read, self.documents_unreadable
            ));
        }
        for (kind, counts) in &self.claims {
            if counts.extracted != counts.accounted() {
                return Err(format!(
                    "{kind}: {} extracted != {} accounted for",
                    counts.extracted,
                    counts.accounted()
                ));
            }
            let skipped: usize = self
                .skipped
                .iter()
                .filter(|((k, _), _)| k == kind)
                .map(|(_, n)| n)
                .sum();
            let ambiguous: usize = self
                .ambiguous
                .iter()
                .filter(|((k, _), _)| k == kind)
                .map(|(_, n)| n)
                .sum();
            if skipped != counts.skipped || ambiguous != counts.ambiguous {
                return Err(format!("{kind}: reasons do not sum to their totals"));
            }
        }
        let totals = self.totals();
        if totals.broken != self.gate.total() {
            return Err(format!(
                "{} broken != {} judged at the gate",
                totals.broken,
                self.gate.total()
            ));
        }
        let abstained: usize = self.abstained.values().sum();
        if abstained != self.gate.abstained {
            return Err("gate abstentions do not sum to their total".to_string());
        }
        Ok(())
    }
}

/// Wall-clock time spent in each stage, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timings {
    /// Walking the repository and selecting documents.
    pub select_ms: u64,
    /// Extraction and resolution, in parallel across documents.
    pub extract_resolve_ms: u64,
    /// History search, in the bounded pool.
    pub history_ms: u64,
    pub total_ms: u64,
}

/// What the history stage did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryStats {
    /// History lookups requested: one per broken claim with a needle.
    pub searches: usize,
    /// Lookups that failed rather than answering.
    pub failed_searches: usize,
    /// git processes actually spawned, cache hits excluded.
    pub git_processes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimKind;
    use crate::resolution::SkipReason;

    #[test]
    fn incomplete_takes_precedence_over_no_input() {
        let failed = [Diagnostic::failure("walk-error", "x", None)];
        let noted = [Diagnostic::note("no-documents", "x", None)];
        assert_eq!(Status::of(&failed, true, 0), Status::Incomplete);
        assert_eq!(Status::of(&noted, true, 0), Status::NoInput);
        assert_eq!(Status::of(&noted, false, 0), Status::Incomplete);
        assert_eq!(Status::of(&[], true, 1), Status::Complete);
        assert_eq!(Status::of(&[], false, 1), Status::Incomplete);
        assert_eq!(Status::of(&noted, true, 3), Status::Complete);
    }

    #[test]
    fn status_codes_are_the_published_ones() {
        assert_eq!(Status::Complete.code(), "complete");
        assert_eq!(Status::Incomplete.code(), "incomplete");
        assert_eq!(Status::NoInput.code(), "no-input");
    }

    fn balanced() -> Coverage {
        let mut c = Coverage {
            documents_matched: 2,
            documents_read: 1,
            documents_unreadable: 1,
            ..Coverage::default()
        };
        let kind = ClaimKind::Command {
            runner: "make".into(),
            args: vec![],
        };
        for _ in 0..3 {
            c.extracted(&kind);
        }
        c.resolved(&kind, &Resolution::True);
        c.resolved(&kind, &Resolution::Skip(SkipReason::NoManifest));
        c.resolved(
            &kind,
            &Resolution::Ambiguous(AmbiguityReason::EcosystemFilename),
        );
        c
    }

    #[test]
    fn a_balanced_tally_reconciles() {
        balanced().reconcile().unwrap();
    }

    #[test]
    fn reconcile_catches_every_kind_of_imbalance() {
        let mut documents = balanced();
        documents.documents_read += 1;
        assert!(documents.reconcile().unwrap_err().starts_with("documents"));

        let mut claims = balanced();
        claims.claims.get_mut("command").unwrap().extracted += 1;
        assert!(claims.reconcile().unwrap_err().contains("extracted"));

        let mut reasons = balanced();
        *reasons
            .skipped
            .get_mut(&("command", "no-manifest"))
            .unwrap() += 1;
        assert!(reasons.reconcile().unwrap_err().contains("reasons"));

        let mut ambiguous = balanced();
        *ambiguous
            .ambiguous
            .get_mut(&("command", "ecosystem-filename"))
            .unwrap() += 1;
        assert!(ambiguous.reconcile().unwrap_err().contains("reasons"));

        let mut gate = balanced();
        gate.gate.rot += 1;
        assert!(gate.reconcile().unwrap_err().contains("judged"));

        let mut abstained = balanced();
        abstained
            .abstained
            .insert(("command", "no-history-needle"), 1);
        assert!(abstained.reconcile().unwrap_err().contains("abstentions"));
    }

    #[test]
    fn merging_adds_every_field() {
        let mut total = balanced();
        total.merge(balanced());
        assert_eq!(total.documents_matched, 4);
        assert_eq!(total.documents_read, 2);
        assert_eq!(total.documents_unreadable, 2);
        assert_eq!(total.claims["command"].extracted, 6);
        assert_eq!(total.claims["command"].true_, 2);
        assert_eq!(total.skipped[&("command", "no-manifest")], 2);
        assert_eq!(total.ambiguous[&("command", "ecosystem-filename")], 2);
        total.reconcile().unwrap();
        let mut markers = Coverage {
            suppression_markers: 3,
            suppression_unused: 1,
            files_suppressed: 1,
            documents_unparseable: 1,
            ..Coverage::default()
        };
        // Distinct values, so a field added to the wrong place, or subtracted, shows —
        // and none of them 2, where doubling and squaring agree.
        markers.gate = GateCounts {
            rot: 1,
            lie_reported: 3,
            lie_unreported: 4,
            abstained: 5,
            history_failed: 6,
        };
        markers.abstained.insert(("path", "no-history-needle"), 1);
        let mut into = markers.clone();
        into.merge(markers);
        assert_eq!(
            into.gate,
            GateCounts {
                rot: 2,
                lie_reported: 6,
                lie_unreported: 8,
                abstained: 10,
                history_failed: 12
            }
        );
        assert_eq!(into.suppression_markers, 6);
        assert_eq!(into.suppression_unused, 2);
        assert_eq!(into.files_suppressed, 2);
        assert_eq!(into.documents_unparseable, 2);
        assert_eq!(into.abstained[&("path", "no-history-needle")], 2);
    }

    #[test]
    fn every_judgement_lands_in_its_own_bucket() {
        use crate::gate::{Finding, Judgement, Tier};
        let kind = ClaimKind::Path;
        let finding = |tier| Finding {
            claim: crate::claim::Claim {
                kind: ClaimKind::Path,
                text: "x".into(),
                file: "README.md".into(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 2,
                span: 0..1,
            },
            tier,
            rule_id: String::new(),
            message: String::new(),
            breaking_commit: None,
            suggestions: vec![],
        };
        let mut c = Coverage::default();
        c.judged(&kind, &Judgement::Reported(Box::new(finding(Tier::Rot))));
        c.judged(&kind, &Judgement::Reported(Box::new(finding(Tier::Lie))));
        c.judged(&kind, &Judgement::Unreported(Tier::Lie));
        c.judged(&kind, &Judgement::Abstained);
        c.judged(&kind, &Judgement::HistoryFailed);
        assert_eq!(
            c.gate,
            GateCounts {
                rot: 1,
                lie_reported: 1,
                lie_unreported: 1,
                abstained: 1,
                history_failed: 1
            }
        );
        assert_eq!(c.gate.total(), 5);
        assert_eq!(c.abstained[&("path", "no-history-needle")], 1);
    }
}
