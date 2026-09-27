//! The run report: a versioned envelope for machines, and a summary for people
//! (ADR-0019).
//!
//! Both are rendered from one `View` — the analysis with the baseline applied once — so
//! the report, the summary and whatever went to stdout cannot disagree about what was
//! found. The existing formats stay exactly as they were; only this envelope is
//! versioned, and a consumer checks `version` before reading anything else.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::coverage::{ClaimCounts, Diagnostic, Status};
use crate::gate::{Finding, Tier};
use crate::pipeline::Outcome;

/// Bumped on any change a consumer could trip over. Additions that an old reader can
/// ignore do not bump it.
pub const REPORT_VERSION: u32 = 1;

/// The analysis as presented: what was found, after the baseline.
pub struct View<'a> {
    pub outcome: &'a Outcome,
    /// Post-baseline findings, each with the fingerprint a baseline would record.
    pub findings: &'a [(Finding, String)],
    pub baseline_suppressed: usize,
    /// The analysis' diagnostics plus any the CLI added after it.
    pub diagnostics: &'a [Diagnostic],
    pub fail_on: &'a str,
    pub baseline: Option<&'a Path>,
    pub paths: &'a [PathBuf],
    pub exit_code: u8,
    /// Edit records, when `--fix` or `--fix-dry-run` planned any.
    pub edits: Option<Value>,
}

impl View<'_> {
    pub fn status(&self) -> Status {
        Status::of(
            self.diagnostics,
            !self.outcome.degraded.is_degraded(),
            self.outcome.documents,
        )
    }

    fn reported(&self, tier: Tier) -> usize {
        self.findings
            .iter()
            .filter(|(f, _)| f.tier == tier && f.rule_id != UNUSED_SUPPRESSION)
            .count()
    }
}

const UNUSED_SUPPRESSION: &str = "stilltrue/suppression/unused";

/// A limitation of this run's coverage: something it did not judge, and why. Never a
/// failure — a complete run can have several — and never a finding.
struct Limitation {
    code: &'static str,
    count: usize,
    clause: String,
}

fn limitations(view: &View) -> Vec<Limitation> {
    let coverage = &view.outcome.coverage;
    let mut out = Vec::new();
    let unsupported: usize = coverage
        .ambiguous
        .iter()
        .filter(|((kind, reason), _)| *kind == "symbol" && *reason == "unsupported-language")
        .map(|(_, n)| n)
        .sum();
    if unsupported > 0 {
        out.push(Limitation {
            code: "symbols-unsupported-language",
            count: unsupported,
            clause: "symbol checking unavailable because this repository contains a \
                     language no resolver reads"
                .to_string(),
        });
    }
    let other_ambiguous = coverage.totals().ambiguous - unsupported;
    if other_ambiguous > 0 {
        out.push(Limitation {
            code: "claims-ambiguous",
            count: other_ambiguous,
            clause: format!("{other_ambiguous} ambiguous claim(s) were not judged"),
        });
    }
    let skipped = coverage.totals().skipped;
    if skipped > 0 {
        out.push(Limitation {
            code: "claims-skipped",
            count: skipped,
            clause: format!("{skipped} claim(s) had nothing to be checked against"),
        });
    }
    if coverage.gate.lie_unreported > 0 {
        out.push(Limitation {
            code: "lies-not-reported",
            count: coverage.gate.lie_unreported,
            clause: format!(
                "{} broken claim(s) with no history behind them were not reported \
                 (--strict reports them)",
                coverage.gate.lie_unreported
            ),
        });
    }
    if view.baseline_suppressed > 0 {
        out.push(Limitation {
            code: "baseline-suppressed",
            count: view.baseline_suppressed,
            clause: format!(
                "{} finding(s) were suppressed by the baseline",
                view.baseline_suppressed
            ),
        });
    }
    if coverage.suppression_markers > 0 {
        out.push(Limitation {
            code: "suppression-markers",
            count: coverage.suppression_markers,
            clause: format!(
                "{} suppression marker(s) removed text from extraction",
                coverage.suppression_markers
            ),
        });
    }
    out
}

fn counts_json(counts: &ClaimCounts) -> Value {
    json!({
        "extracted": counts.extracted,
        "ignored": counts.ignored,
        "true": counts.true_,
        "broken": counts.broken,
        "skipped": counts.skipped,
        "ambiguous": counts.ambiguous,
    })
}

fn reasons_json(reasons: &std::collections::BTreeMap<(&str, &str), usize>) -> Value {
    reasons
        .iter()
        .map(|((kind, reason), count)| json!({"kind": kind, "reason": reason, "count": count}))
        .collect()
}

/// The versioned run report.
pub fn envelope(view: &View) -> Value {
    let outcome = view.outcome;
    let coverage = &outcome.coverage;
    let by_kind: serde_json::Map<String, Value> = coverage
        .claims
        .iter()
        .map(|(kind, counts)| ((*kind).to_string(), counts_json(counts)))
        .collect();
    let findings: Vec<Value> = view
        .findings
        .iter()
        .map(|(finding, fingerprint)| {
            let mut value = crate::report::finding_json(finding);
            value["fingerprint"] = json!(fingerprint);
            value
        })
        .collect();
    json!({
        "schema": "stilltrue/run-report",
        "version": REPORT_VERSION,
        "tool": {"name": "stilltrue", "version": env!("CARGO_PKG_VERSION")},
        "status": view.status().code(),
        "exitCode": view.exit_code,
        "run": {
            "head": outcome.head,
            "strict": outcome.strict,
            "failOn": view.fail_on,
            "baseline": view.baseline.map(|p| p.to_string_lossy().into_owned()),
            "paths": view.paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
            "timings": {
                "selectMs": outcome.timings.select_ms,
                "extractResolveMs": outcome.timings.extract_resolve_ms,
                "historyMs": outcome.timings.history_ms,
                "totalMs": outcome.timings.total_ms,
            },
        },
        "history": {
            "state": outcome.degraded.slug(),
            "complete": !outcome.degraded.is_degraded() && outcome.history.failed_searches == 0,
            "searches": outcome.history.searches,
            "failedSearches": outcome.history.failed_searches,
            "gitProcesses": outcome.history.git_processes,
        },
        "coverage": {
            "documents": {
                "matched": coverage.documents_matched,
                "read": coverage.documents_read,
                "unreadable": coverage.documents_unreadable,
                "unparseable": coverage.documents_unparseable,
            },
            "suppressionMarkers": {
                "total": coverage.suppression_markers,
                "unused": coverage.suppression_unused,
                "filesSuppressed": coverage.files_suppressed,
            },
            "claims": counts_json(&coverage.totals()),
            "byKind": by_kind,
            "skipped": reasons_json(&coverage.skipped),
            "ambiguous": reasons_json(&coverage.ambiguous),
            "gate": {
                "rot": coverage.gate.rot,
                "lieReported": coverage.gate.lie_reported,
                "lieUnreported": coverage.gate.lie_unreported,
                "abstained": coverage.gate.abstained,
                "historyFailed": coverage.gate.history_failed,
                "abstainedReasons": reasons_json(&coverage.abstained),
            },
            "baseline": {"suppressed": view.baseline_suppressed},
            "reported": {
                "total": view.findings.len(),
                "rot": view.reported(Tier::Rot),
                "lie": view.reported(Tier::Lie),
                "unusedSuppressions": view.findings.iter().filter(|(f, _)| f.rule_id == UNUSED_SUPPRESSION).count(),
            },
            "limitations": limitations(view).iter().map(|l| json!({
                "code": l.code,
                "count": l.count,
                "message": l.clause,
            })).collect::<Vec<_>>(),
        },
        "diagnostics": view.diagnostics.iter().map(|d| json!({
            "code": d.code,
            "message": d.message,
            "path": d.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "incomplete": d.incomplete,
        })).collect::<Vec<_>>(),
        "findings": findings,
        "edits": view.edits.clone(),
    })
}

/// The human summary, for stderr. It describes extracted claims and their limits; it
/// never says the documentation is correct, because nothing here can know that.
pub fn summary(view: &View) -> String {
    let outcome = view.outcome;
    let coverage = &outcome.coverage;
    let totals = coverage.totals();
    let status = view.status();
    let mut out = format!("stilltrue: summary ({})\n", status.code());
    out.push_str(&format!(
        "  documents   {} matched · {} read · {} unreadable\n",
        coverage.documents_matched, coverage.documents_read, coverage.documents_unreadable
    ));
    out.push_str(&format!(
        "  claims      {} extracted · {} ignored · {} true · {} broken · {} skipped · {} ambiguous\n",
        totals.extracted,
        totals.ignored,
        totals.true_,
        totals.broken,
        totals.skipped,
        totals.ambiguous
    ));
    out.push_str(&format!(
        "  gate        {} rot · {} lie reported · {} lie not reported · {} abstained · {} history failed\n",
        coverage.gate.rot,
        coverage.gate.lie_reported,
        coverage.gate.lie_unreported,
        coverage.gate.abstained,
        coverage.gate.history_failed
    ));
    out.push_str(&format!(
        "  baseline    {} suppressed\n",
        view.baseline_suppressed
    ));
    out.push_str(&format!(
        "  history     {} · {} searches · {} git processes\n",
        outcome.degraded.slug(),
        outcome.history.searches,
        outcome.history.git_processes
    ));
    if !coverage.skipped.is_empty() {
        out.push_str(&format!(
            "  skipped     {}\n",
            reasons_line(&coverage.skipped)
        ));
    }
    if !coverage.ambiguous.is_empty() {
        out.push_str(&format!(
            "  ambiguous   {}\n",
            reasons_line(&coverage.ambiguous)
        ));
    }
    for diagnostic in view.diagnostics {
        let place = diagnostic
            .path
            .as_ref()
            .map(|p| format!("{}: ", p.display()))
            .unwrap_or_default();
        out.push_str(&format!(
            "  diagnostic  {place}{} — {}\n",
            diagnostic.code, diagnostic.message
        ));
    }
    out.push_str(&format!("stilltrue: {}\n", verdict(view, status)));
    out
}

fn reasons_line(reasons: &std::collections::BTreeMap<(&str, &str), usize>) -> String {
    reasons
        .iter()
        .map(|((kind, reason), count)| format!("{kind}/{reason} {count}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// One sentence a reader can act on. Actual counts and reasons, never "verified".
fn verdict(view: &View, status: Status) -> String {
    let documents = view.outcome.documents;
    let findings = view.findings.len();
    let found = match findings {
        0 => None,
        n => Some(format!(
            "{n} finding(s) reported ({} rot, {} lie)",
            view.reported(Tier::Rot),
            view.reported(Tier::Lie)
        )),
    };
    match status {
        Status::NoInput => "Nothing was examined: no documents matched.".to_string(),
        Status::Incomplete => {
            let mut reasons: Vec<&str> = view
                .diagnostics
                .iter()
                .filter(|d| d.incomplete)
                .map(|d| d.code)
                .collect();
            reasons.dedup();
            let reasons = if reasons.is_empty() {
                "history-unavailable".to_string()
            } else {
                reasons.join(", ")
            };
            match found {
                Some(found) => format!("{found}; the run was incomplete ({reasons})."),
                None => format!(
                    "Incomplete ({reasons}). A quiet result from this run is not a clean one."
                ),
            }
        }
        Status::Complete => {
            let clauses: Vec<String> = limitations(view).into_iter().map(|l| l.clause).collect();
            let head = match found {
                Some(found) => format!("{found}. Checked {documents} document(s)"),
                None if view.outcome.strict => {
                    format!("No reportable findings. Checked {documents} document(s)")
                }
                None => format!("No reportable rot. Checked {documents} document(s)"),
            };
            if clauses.is_empty() {
                format!("{head}.")
            } else {
                format!("{head}; {}.", clauses.join("; "))
            }
        }
    }
}
