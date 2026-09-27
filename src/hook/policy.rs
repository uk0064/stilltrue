//! What a completion check does with what it found: stay silent, advise, or ask for one
//! more pass. Pure, so every combination of the conditions in ADR-0023 is a unit test
//! rather than a scenario.

use serde::{Deserialize, Serialize};

/// One finding as a session remembers it. The fingerprint is the identity; the rest is
/// what a message needs to say which claim it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub fingerprint: String,
    pub rule: String,
    pub tier: String,
    pub file: String,
    pub line: u64,
    pub claim: String,
    pub message: String,
    /// `sha subject`, when history proved the claim once true.
    pub commit: Option<String>,
}

impl Finding {
    pub fn is_rot(&self) -> bool {
        self.tier == "rot"
    }
}

/// How the current findings relate to the ones the session started with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comparison {
    /// Present now, absent at the start: newly observed. Not "introduced by the agent" —
    /// a concurrent author, or a pull, can put them there too.
    pub new: Vec<Finding>,
    /// Present at the start and still present.
    pub preexisting: Vec<Finding>,
    /// Present at the start and gone now.
    pub resolved: Vec<Finding>,
}

/// Match by fingerprint. A fingerprint carries no line number, so a reflowed document
/// keeps its findings' identities; a renamed document or a repeated claim does not, and
/// shows as new — which is why "new" never blocks on identity alone (see `decide`).
pub fn compare(initial: &[Finding], current: &[Finding]) -> Comparison {
    let known = |f: &Finding, set: &[Finding]| set.iter().any(|g| g.fingerprint == f.fingerprint);
    Comparison {
        new: current
            .iter()
            .filter(|f| !known(f, initial))
            .cloned()
            .collect(),
        preexisting: current
            .iter()
            .filter(|f| known(f, initial))
            .cloned()
            .collect(),
        resolved: initial
            .iter()
            .filter(|f| !known(f, current))
            .cloned()
            .collect(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Report, never ask for a continuation. The default.
    Advisory,
    /// May ask for one corrective continuation per user turn, under every condition in
    /// `decide`. Opt-in, and not recommended beyond benchmark runs until the promotion
    /// gates pass on the host in question.
    Block,
}

/// How the completion scan ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    /// The checker ran and its report says `complete`.
    Complete,
    /// The checker ran and its report says `incomplete`, with the reasons.
    Incomplete(Vec<String>),
    /// The checker ran and no document matched.
    NoInput,
    /// The checker did not finish in time.
    TimedOut,
    /// No checker binary was found.
    Missing,
    /// The checker's report could not be read.
    Malformed,
    /// The checker exited with this status — never a request to continue, whatever the
    /// number (2 in particular is "unrecoverable fault", and a host that reads exit 2 as
    /// "block" must never see it from here).
    Failed(i32),
}

impl Completion {
    pub fn is_complete(&self) -> bool {
        matches!(self, Completion::Complete)
    }

    /// Whether any report was produced at all.
    pub fn ran(&self) -> bool {
        matches!(
            self,
            Completion::Complete | Completion::Incomplete(_) | Completion::NoInput
        )
    }
}

/// Everything a completion decision depends on.
#[derive(Debug, Clone)]
pub struct Facts {
    pub mode: Mode,
    /// The host says this stop follows a continuation a stop hook asked for.
    pub recursion: bool,
    /// Continuations already requested in this user turn.
    pub continuations: u32,
    /// The user interrupted this turn.
    pub interrupted: bool,
    /// `None` when no valid comparison exists: the initial scan was incomplete or
    /// missing, or configuration, tool or branch changed since.
    pub comparison: Option<Comparison>,
    pub completion: Completion,
    /// Files changed while the completion scan ran, so its result describes a state
    /// that no longer exists.
    pub stale: bool,
    /// How many findings the completion scan reported. When there is no comparison,
    /// these are all it can offer, and they are offered as advice.
    pub current: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Silent,
    Advise,
    Block,
}

/// The newly observed findings that could justify a continuation: rot only. A lie is
/// Tier B, and pre-existing findings are the backlog the session was told about at
/// entry — neither is this turn's to repair.
pub fn blocking(comparison: &Comparison) -> Vec<&Finding> {
    comparison.new.iter().filter(|f| f.is_rot()).collect()
}

pub fn decide(facts: &Facts) -> Decision {
    let eligible = facts.comparison.as_ref().map(blocking).unwrap_or_default();
    // Every condition, not most of them. Each one is a way to interrupt someone for a
    // finding that is not theirs, not real, or not current.
    let may_block = facts.mode == Mode::Block
        && !facts.recursion
        && facts.continuations == 0
        && !facts.interrupted
        && facts.completion.is_complete()
        && !facts.stale
        && facts.comparison.is_some()
        && !eligible.is_empty();
    if may_block {
        return Decision::Block;
    }
    let newly_observed = facts.comparison.as_ref().is_some_and(|c| !c.new.is_empty());
    // Without a comparison nothing can be called new, but the current findings are
    // still worth saying — as current, never as the agent's.
    let uncompared = facts.comparison.is_none() && facts.current > 0;
    // Something the user should hear: a new finding, findings that could not be
    // compared, a check that could not vouch for the state it left, or a verification
    // after a continuation.
    if newly_observed || uncompared || !facts.completion.is_complete() || facts.recursion {
        Decision::Advise
    } else {
        Decision::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(fingerprint: &str, tier: &str) -> Finding {
        Finding {
            fingerprint: fingerprint.into(),
            rule: format!("stilltrue/command/{tier}"),
            tier: tier.into(),
            file: "CLAUDE.md".into(),
            line: 1,
            claim: "make demo".into(),
            message: "command `make demo` has no target".into(),
            commit: None,
        }
    }

    fn facts(comparison: Option<Comparison>) -> Facts {
        Facts {
            mode: Mode::Block,
            recursion: false,
            continuations: 0,
            interrupted: false,
            comparison,
            completion: Completion::Complete,
            stale: false,
            current: 1,
        }
    }

    fn new_rot() -> Option<Comparison> {
        Some(compare(&[], &[finding("a", "rot")]))
    }

    #[test]
    fn compare_sorts_findings_by_fingerprint() {
        let initial = [finding("kept", "rot"), finding("gone", "rot")];
        let current = [finding("kept", "rot"), finding("fresh", "lie")];
        let c = compare(&initial, &current);
        assert_eq!(c.new, [finding("fresh", "lie")]);
        assert_eq!(c.preexisting, [finding("kept", "rot")]);
        assert_eq!(c.resolved, [finding("gone", "rot")]);
    }

    #[test]
    fn new_rot_under_every_condition_blocks() {
        assert_eq!(decide(&facts(new_rot())), Decision::Block);
    }

    #[test]
    fn each_condition_alone_prevents_a_block() {
        let mut advisory = facts(new_rot());
        advisory.mode = Mode::Advisory;
        let mut recursion = facts(new_rot());
        recursion.recursion = true;
        let mut again = facts(new_rot());
        again.continuations = 1;
        let mut interrupted = facts(new_rot());
        interrupted.interrupted = true;
        let mut incomplete = facts(new_rot());
        incomplete.completion = Completion::Incomplete(vec!["history-unavailable".into()]);
        let mut stale = facts(new_rot());
        stale.stale = true;
        let unavailable = facts(None);
        for (name, case) in [
            ("advisory mode", advisory),
            ("recursion", recursion),
            ("second continuation", again),
            ("interrupted", interrupted),
            ("incomplete completion scan", incomplete),
            ("stale scan", stale),
            ("no comparison", unavailable),
        ] {
            assert_ne!(decide(&case), Decision::Block, "{name} blocked");
        }
    }

    #[test]
    fn a_lie_or_a_preexisting_finding_never_blocks() {
        let lie = facts(Some(compare(&[], &[finding("a", "lie")])));
        assert_eq!(decide(&lie), Decision::Advise);
        let backlog = facts(Some(compare(
            &[finding("a", "rot")],
            &[finding("a", "rot")],
        )));
        assert_eq!(decide(&backlog), Decision::Silent);
    }

    #[test]
    fn a_checker_that_did_not_run_is_advised_never_blocked() {
        for completion in [
            Completion::TimedOut,
            Completion::Missing,
            Completion::Malformed,
            Completion::Failed(2),
            Completion::NoInput,
        ] {
            let mut case = facts(new_rot());
            case.completion = completion.clone();
            assert_eq!(decide(&case), Decision::Advise, "{completion:?}");
        }
    }

    #[test]
    fn findings_without_a_comparison_are_advised_and_silence_is_kept_without_them() {
        let mut uncompared = facts(None);
        uncompared.current = 2;
        assert_eq!(decide(&uncompared), Decision::Advise);
        uncompared.current = 0;
        assert_eq!(decide(&uncompared), Decision::Silent);
    }

    #[test]
    fn a_clean_complete_check_is_silent_and_a_verification_is_not() {
        assert_eq!(
            decide(&facts(Some(Comparison::default()))),
            Decision::Silent
        );
        let mut verification = facts(Some(Comparison::default()));
        verification.recursion = true;
        assert_eq!(decide(&verification), Decision::Advise);
    }

    #[test]
    fn completion_reports_whether_the_checker_ran() {
        assert!(Completion::Complete.ran());
        assert!(Completion::Incomplete(vec![]).ran());
        assert!(Completion::NoInput.ran());
        assert!(!Completion::TimedOut.ran());
        assert!(!Completion::Failed(1).ran());
        assert!(Completion::Complete.is_complete());
        assert!(!Completion::NoInput.is_complete());
    }
}
