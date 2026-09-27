//! Stage 3: decide whether a broken claim deserves a human's attention.
//!
//! `gate` existing as a separate stage is the thesis. It performs the same search for
//! every claim type; it does not know what a Makefile or a `package.json` is.

use crate::claim::Claim;
use crate::git::{Commit, Git};
use crate::resolution::Evidence;

/// How much confidence a broken claim carries. Not a severity — a confidence class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Broken, and history proves it was once true.
    Rot,
    /// Broken, with no evidence it was ever true.
    Lie,
}

impl Tier {
    pub fn slug(self) -> &'static str {
        match self {
            Tier::Rot => "rot",
            Tier::Lie => "lie",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub claim: Claim,
    pub tier: Tier,
    pub rule_id: String,
    pub message: String,
    pub breaking_commit: Option<Commit>,
    pub suggestions: Vec<String>,
}

/// At most this many suggestions, and never from a candidate set large enough for
/// coincidental near-matches.
const MAX_SUGGESTIONS: usize = 3;
const MAX_CANDIDATES: usize = 200;
/// At or below this many candidates, naming them all is an enumeration of what exists
/// rather than a guess at what was meant.
const SMALL_CANDIDATE_SET: usize = 5;

/// What `gate` decided about one broken claim, in enough detail for the run report to
/// account for every claim that did not become a finding.
#[derive(Debug, Clone)]
pub enum Judgement {
    /// Worth a human's attention under the flags given.
    Reported(Box<Finding>),
    /// A real finding of this tier that the flags do not report — a lie without
    /// `--strict`.
    Unreported(Tier),
    /// No definition-shaped needle: Tier C (ADR-0005).
    Abstained,
    /// The history search itself failed, so no tier can be assigned. Never a finding:
    /// a failed search is not evidence that the claim never existed (ADR-0020).
    HistoryFailed,
}

pub fn gate(claim: &Claim, evidence: &Evidence, git: &dyn Git, strict: bool) -> Option<Finding> {
    match judge(claim, evidence, git, strict) {
        Judgement::Reported(finding) => Some(*finding),
        Judgement::Unreported(_) | Judgement::Abstained | Judgement::HistoryFailed => None,
    }
}

pub fn judge(claim: &Claim, evidence: &Evidence, git: &dyn Git, strict: bool) -> Judgement {
    // No definition-shaped needle means Tier C: an abstention, not a finding.
    let Some(needle) = evidence.needle.as_ref() else {
        return Judgement::Abstained;
    };

    let Ok(breaking_commit) = git.try_search(needle) else {
        return Judgement::HistoryFailed;
    };
    let tier = if breaking_commit.is_some() {
        Tier::Rot
    } else {
        Tier::Lie
    };

    if tier == Tier::Lie && !strict {
        return Judgement::Unreported(Tier::Lie);
    }

    Judgement::Reported(Box::new(Finding {
        claim: claim.clone(),
        tier,
        rule_id: format!("stilltrue/{}/{}", claim.kind.slug(), tier.slug()),
        message: evidence.message.clone(),
        breaking_commit,
        suggestions: suggestions(&claim.text, &evidence.candidates),
    }))
}

/// A wrong suggestion costs more trust than no suggestion buys.
fn suggestions(needle: &str, candidates: &[String]) -> Vec<String> {
    if candidates.len() > MAX_CANDIDATES {
        return Vec::new();
    }
    // A small set is enumerated, not guessed at, and enumerating means naming all of
    // them — showing three of five would be back to guessing, just silently.
    if candidates.len() <= SMALL_CANDIDATE_SET {
        let mut all: Vec<String> = candidates.to_vec();
        all.sort();
        return all;
    }

    // Compare against the last word: `make demo` should be measured against `demo`.
    let target = needle.split_whitespace().next_back().unwrap_or(needle);
    // At most 2 for a short name, otherwise 30% of its length.
    let limit = if target.len() <= 8 {
        2
    } else {
        target.len() * 3 / 10
    };

    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .map(|c| (strsim::levenshtein(target, c), c))
        .filter(|(distance, _)| *distance <= limit)
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, c)| c.clone())
        .collect()
}
