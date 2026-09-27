//! Agent harness adapters: stilltrue at session entry and at completion (ADR-0023).
//!
//! At entry the adapter scans, remembers what it found as the session's starting point,
//! and gives the agent compact evidence about instructions that may be stale. At each
//! completion it scans again — every configured document, touched or not — and compares:
//! a finding present now and absent at the start is *newly observed*. That is advice by
//! default. Blocking is opt-in, asks for at most one continuation per user turn, and
//! only for newly observed rot found by complete scans that still describe the working
//! tree. CI stays the independent gate; nothing here trusts or replaces it.
//!
//! The adapter never runs autofix and never runs a command a document names. It only
//! reports, and lets the agent repair with its ordinary permissions.

pub mod host;
pub mod identity;
pub mod message;
pub mod policy;
pub mod scan;
pub mod state;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use host::{Event, Host, Kind, Response, Source};
use policy::{Completion, Decision, Facts, Finding, Mode};
use state::{Epoch, Load, Scan, State, Store, Turn};

/// The initial per-scan budget (ADR-0023).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Settings {
    pub mode: Mode,
    pub timeout: Duration,
    /// A baseline file, relative to the repository root, passed to every scan.
    pub baseline: Option<PathBuf>,
    pub state_dir: PathBuf,
    /// The checker binary, already located. `None` means none was found.
    pub checker: Option<PathBuf>,
    /// Unix seconds, for epoch and update times.
    pub now: i64,
}

/// Handle one hook invocation: the host's payload in, the bytes for stdout out.
pub fn handle(host: Host, input: &str, settings: &Settings) -> String {
    let event = match host::parse(input) {
        Ok(event) => event,
        Err(error) => {
            // Silence, not an error status: a malformed payload is not a reason to stop
            // anyone's session, and on Claude Code a status of 2 would ask it to go on.
            eprintln!("stilltrue-hook: {error}");
            return String::new();
        }
    };
    let response = match &event.kind {
        Kind::SessionStart(source) => session_start(host, &event, source, settings),
        Kind::UserPrompt => user_prompt(host, &event, settings),
        Kind::Stop { recursion, turn } => stop(host, &event, *recursion, turn.as_deref(), settings),
        Kind::Interrupt { turn } => interrupt(host, &event, turn.as_deref(), settings),
        Kind::Other(_) => Response::Nothing,
    };
    host::render(&event.kind, &response)
}

/// One invocation's view of its session.
struct Session<'a> {
    host: Host,
    settings: &'a Settings,
    store: Store,
    key: String,
    workspace: identity::Workspace,
    analysis: String,
    config: String,
}

/// A scan and the state it was taken in.
struct Scanned {
    outcome: scan::Outcome,
    scan: Scan,
    /// Files changed while it ran.
    stale: bool,
    report: Option<PathBuf>,
}

impl<'a> Session<'a> {
    fn open(host: Host, event: &Event, settings: &'a Settings) -> Self {
        let workspace = identity::workspace(&event.cwd);
        let store = Store::new(settings.state_dir.clone());
        store.prune();
        Self {
            key: Store::key(&workspace.root, host.slug(), &event.session),
            analysis: identity::analysis(settings.checker.as_deref()),
            config: identity::configuration(&workspace.root, settings.baseline.as_deref()),
            host,
            settings,
            store,
            workspace,
        }
    }

    /// The stored state, or why there is none.
    fn load(&self) -> Result<State, &'static str> {
        match self.store.load(&self.key) {
            Load::Found(state) => Ok(*state),
            Load::Missing => Err("state-missing"),
            Load::Unusable => Err("state-unusable"),
        }
    }

    fn save(&self, mut state: State) {
        state.updated = self.settings.now;
        if let Err(error) = self.store.save(&self.key, &state) {
            eprintln!("stilltrue-hook: could not save session state: {error}");
        }
    }

    /// A new comparison epoch. What was still unresolved is carried as advice, never
    /// folded into the new starting point as backlog.
    fn fresh(&self, event: &Event, prior: Option<&State>, reason: &str) -> State {
        State {
            schema: state::SCHEMA,
            host: self.host.slug().to_string(),
            session: event.session.clone(),
            worktree: self.workspace.root.clone(),
            analysis: self.analysis.clone(),
            config: self.config.clone(),
            branch: self.workspace.branch.clone(),
            epoch: Epoch {
                id: prior.map_or(1, |p| p.epoch.id + 1),
                started: self.settings.now,
                reason: reason.to_string(),
            },
            initial: None,
            carried: prior.map(unresolved).unwrap_or_default(),
            last: None,
            turn: prior.map(|p| p.turn.clone()).unwrap_or_default(),
            // Not carried over. A new epoch follows a cleared or new conversation, or a
            // changed tool, config or branch; what the agent was told before may be
            // gone from its context, so the same evidence is worth saying again.
            last_message: None,
            pending: None,
            events: prior.map(|p| p.events.clone()).unwrap_or_default(),
            updated: self.settings.now,
        }
    }

    /// Why a stored state cannot be compared with now, if it cannot.
    fn incompatible(&self, state: &State) -> Option<&'static str> {
        if state.analysis != self.analysis {
            Some("tool-changed")
        } else if state.config != self.config {
            Some("config-changed")
        } else if state.branch != self.workspace.branch {
            Some("branch-changed")
        } else {
            None
        }
    }

    fn identity(&self) -> String {
        let head = identity::workspace(&self.workspace.root).head;
        identity::identity(&self.workspace.root, head.as_deref())
    }

    /// Scan, given the identity of the state it starts from.
    fn scan(&self, which: &str, before: String) -> Scanned {
        let report = self.store.report_path(&self.key, which);
        let outcome = scan::run(
            self.settings.checker.as_deref(),
            &self.workspace.root,
            self.settings.baseline.as_deref(),
            &report,
            self.settings.timeout,
        );
        // A result is only about the state it read. If anything moved underneath it,
        // it has no authority over the state that exists now.
        let stale = self.identity() != before;
        let completion = match (&outcome.completion, stale) {
            (_, true) => "stale",
            (Completion::Complete, false) => "complete",
            (Completion::Incomplete(_), false) => "incomplete",
            (Completion::NoInput, false) => "no-input",
            (Completion::TimedOut, false) => "timed-out",
            (Completion::Missing, false) => "missing",
            (Completion::Malformed, false) => "malformed",
            (Completion::Failed(_), false) => "failed",
        };
        Scanned {
            report: outcome.completion.ran().then_some(report),
            scan: Scan {
                completion: completion.to_string(),
                identity: before,
                head: identity::workspace(&self.workspace.root).head,
                findings: outcome.findings.clone(),
            },
            outcome,
            stale,
        }
    }
}

fn count(state: &mut State, event: &str) {
    *state.events.entry(event.to_string()).or_default() += 1;
}

/// Findings still present at the last scan that the epoch did not start with.
fn unresolved(state: &State) -> Vec<Finding> {
    let initial: &[Finding] = state.initial.as_ref().map_or(&[], |s| &s.findings);
    let mut out: Vec<Finding> = state
        .last
        .as_ref()
        .map(|last| policy::compare(initial, &last.findings).new)
        .unwrap_or_default();
    for carried in &state.carried {
        let still = state.last.as_ref().is_some_and(|l| {
            l.findings
                .iter()
                .any(|f| f.fingerprint == carried.fingerprint)
        });
        if still && !out.iter().any(|f| f.fingerprint == carried.fingerprint) {
            out.push(carried.clone());
        }
    }
    out
}

/// A message, and the evidence it states. Two messages stating the same evidence are
/// the same message, however they are worded or wherever their report was written.
struct Said {
    evidence: String,
    text: String,
}

impl Said {
    fn new(kind: &str, findings: &[&[Finding]], text: String) -> Self {
        let mut evidence = kind.to_string();
        for set in findings {
            let mut prints: Vec<&str> = set.iter().map(|f| f.fingerprint.as_str()).collect();
            prints.sort_unstable();
            evidence.push('|');
            evidence.push_str(&prints.join(","));
        }
        Self { evidence, text }
    }
}

/// Say it unless this session was last told exactly the same evidence at the same kind
/// of boundary. Entry context goes to the agent and completion advice to the user, so
/// the one never stands in for the other.
fn once(state: &mut State, boundary: &str, said: Said) -> Option<String> {
    let digest = message::digest(&format!("{boundary}\u{1f}{}", said.evidence));
    if state.last_message.as_deref() == Some(digest.as_str()) {
        return None;
    }
    state.last_message = Some(digest);
    Some(said.text)
}

fn unverified(completion: &Completion, settings: &Settings) -> Option<String> {
    Some(match completion {
        Completion::Complete | Completion::Incomplete(_) | Completion::NoInput => return None,
        Completion::TimedOut => format!(
            "stilltrue: unverified — the check did not finish within {}s, so nothing was \
             confirmed. On a large repository the first scan searches all of its history; \
             running `stilltrue` there once fills the cache these checks read, or raise \
             --timeout.",
            settings.timeout.as_secs_f64()
        ),
        Completion::Missing => "stilltrue: unverified — the stilltrue checker was not found. \
             Install it, or set STILLTRUE_BIN to its path."
            .to_string(),
        Completion::Malformed => {
            "stilltrue: unverified — the checker's report could not be read.".to_string()
        }
        Completion::Failed(code) => {
            format!("stilltrue: unverified — the checker exited with status {code}.")
        }
    })
}

fn session_start(host: Host, event: &Event, source: &Source, settings: &Settings) -> Response {
    let session = Session::open(host, event, settings);
    let prior = session.load();
    let resuming = matches!(source, Source::Resume | Source::Compact);
    // Resume keeps a compatible session's starting point — newly observed findings
    // included, so they do not turn into backlog by surviving a restart. Anything else
    // begins a new comparison, and records why.
    let mut state = match &prior {
        Ok(prior) if resuming && session.incompatible(prior).is_none() => prior.clone(),
        Ok(prior) => {
            let reason = if resuming {
                session.incompatible(prior).unwrap_or("state-missing")
            } else {
                match source {
                    Source::Clear => "cleared",
                    Source::Fork => "forked",
                    _ => "new-session",
                }
            };
            session.fresh(event, Some(prior), reason)
        }
        Err(reason) => session.fresh(event, None, if resuming { reason } else { "new-session" }),
    };
    count(&mut state, "session-start");
    let new_epoch = state.initial.is_none();

    let scanned = session.scan(
        if new_epoch { "entry" } else { "latest" },
        session.identity(),
    );
    let usable = scanned.scan.complete();
    match &state.initial {
        None => state.initial = Some(scanned.scan.clone()),
        Some(initial) if !initial.complete() && usable => {
            state.initial = Some(scanned.scan.clone());
            state.epoch.reason = "baseline-established".to_string();
        }
        Some(_) => {}
    }
    let said = entry_message(&state, &scanned, new_epoch, settings);
    state.last = Some(scanned.scan);
    let response = match once(&mut state, "entry", said) {
        Some(text) => Response::Context(text),
        None => Response::Nothing,
    };
    session.save(state);
    response
}

fn entry_message(state: &State, scanned: &Scanned, new_epoch: bool, settings: &Settings) -> Said {
    let report = scanned.report.as_deref();
    let findings = &scanned.scan.findings;
    let carried_note = if state.carried.is_empty() {
        String::new()
    } else {
        format!(
            "{} finding(s) were unresolved when this session's previous comparison ended ({}); \
             they are advisory context, not backlog this session started with.",
            state.carried.len(),
            state.epoch.reason
        )
    };
    let carried: &[Finding] = &state.carried;
    if let Some(text) = unverified(&scanned.outcome.completion, settings) {
        let kind = format!("unverified:{}", scanned.scan.completion);
        return Said::new(
            &kind,
            &[carried],
            message::compose(&text, &[], &carried_note, None),
        );
    }
    if scanned.stale {
        let text = message::compose(
            "stilltrue: files changed while the entry check ran, so it vouches for nothing; \
             the next check will look again.",
            &[],
            &carried_note,
            report,
        );
        return Said::new("stale", &[carried], text);
    }
    match &scanned.outcome.completion {
        Completion::NoInput => Said::new(
            "no-input",
            &[],
            "stilltrue: no documents matched, so nothing was checked.".into(),
        ),
        Completion::Incomplete(reasons) => Said::new(
            &format!("incomplete:{}", reasons.join(",")),
            &[findings, carried],
            message::compose(
                &format!(
                    "stilltrue: the check was incomplete ({}); its silence is not evidence that \
                     the instructions are current.",
                    reasons.join(", ")
                ),
                findings,
                &carried_note,
                report,
            ),
        ),
        _ if findings.is_empty() => Said::new(
            "clean",
            &[carried],
            message::compose(
                &format!(
                    "stilltrue: checked {} document(s); no reportable rot in this repository's \
                     instructions or docs.",
                    scanned.outcome.documents
                ),
                &[],
                &carried_note,
                None,
            ),
        ),
        _ => {
            let initial: &[Finding] = state.initial.as_ref().map_or(&[], |s| &s.findings);
            let comparison = policy::compare(initial, findings);
            if new_epoch || comparison.new.is_empty() {
                // Everything here predates the session: evidence, not a task.
                let text = message::compose(
                    &format!(
                        "stilltrue (advisory): {} finding(s) in this repository's documents. \
                         These instructions may be stale: check them before relying on them. \
                         They were here before this session began, so repair them only if the \
                         task calls for it.",
                        findings.len()
                    ),
                    findings,
                    &carried_note,
                    report,
                );
                return Said::new("backlog", &[findings, carried], text);
            }
            let text = message::compose(
                &format!(
                    "stilltrue (advisory): resumed. {} finding(s) newly observed since this \
                     session began, and {} it began with.",
                    comparison.new.len(),
                    comparison.preexisting.len()
                ),
                &comparison.new,
                &carried_note,
                report,
            );
            Said::new(
                "resumed",
                &[&comparison.new, &comparison.preexisting, carried],
                text,
            )
        }
    }
}

fn user_prompt(host: Host, event: &Event, settings: &Settings) -> Response {
    let session = Session::open(host, event, settings);
    let Ok(mut state) = session.load() else {
        return Response::Nothing;
    };
    count(&mut state, "user-prompt");
    // A new user turn: its continuation allowance is fresh.
    state.turn = Turn {
        id: format!("prompt-{}", state.events["user-prompt"]),
        continuations: 0,
        interrupted: false,
    };
    // Advice is only ever left pending for a host that can carry it (see `stop`).
    let pending = state.pending.take();
    session.save(state);
    match pending {
        Some(text) => Response::Context(text),
        None => Response::Nothing,
    }
}

fn interrupt(host: Host, event: &Event, turn: Option<&str>, settings: &Settings) -> Response {
    let session = Session::open(host, event, settings);
    let Ok(mut state) = session.load() else {
        return Response::Nothing;
    };
    count(&mut state, "interrupt");
    if let Some(turn) = turn
        && state.turn.id != turn
    {
        state.turn = Turn {
            id: turn.to_string(),
            ..Turn::default()
        };
    }
    state.turn.interrupted = true;
    session.save(state);
    Response::Nothing
}

fn stop(
    host: Host,
    event: &Event,
    recursion: bool,
    turn: Option<&str>,
    settings: &Settings,
) -> Response {
    let session = Session::open(host, event, settings);
    let (mut state, mut unavailable) = match session.load() {
        Ok(state) => (state, None),
        Err(reason) => (session.fresh(event, None, reason), Some(reason)),
    };
    count(&mut state, "stop");
    if let Some(turn) = turn
        && state.turn.id != turn
    {
        state.turn = Turn {
            id: turn.to_string(),
            ..Turn::default()
        };
    }

    // Skip only when a trustworthy comparison says nothing relevant changed: the same
    // content identity as the last complete check, under the same tool, config and
    // branch. An edit event alone is not that — a shell command can change files no
    // event reported — which is why the identity is computed rather than inferred.
    let incompatible = session.incompatible(&state);
    let current = session.identity();
    if !recursion
        && incompatible.is_none()
        && state
            .last
            .as_ref()
            .is_some_and(|last| last.complete() && last.identity == current)
    {
        session.save(state);
        return Response::Nothing;
    }

    let scanned = session.scan("latest", current);
    let comparison = if let Some(reason) = incompatible {
        state = session.fresh(event, Some(&state), reason);
        if scanned.scan.complete() {
            state.initial = Some(scanned.scan.clone());
        }
        unavailable = Some(reason);
        None
    } else {
        match &state.initial {
            Some(initial) if initial.complete() && scanned.scan.complete() => {
                Some(policy::compare(&initial.findings, &scanned.scan.findings))
            }
            initial => {
                unavailable = unavailable.or(Some(if initial.is_some() {
                    "initial-scan-incomplete"
                } else {
                    "state-missing"
                }));
                // A starting point is only ever established from a complete scan.
                if scanned.scan.complete() {
                    state.initial = Some(scanned.scan.clone());
                    state.epoch.reason = "baseline-established".to_string();
                }
                None
            }
        }
    };

    let facts = Facts {
        mode: settings.mode,
        recursion,
        continuations: state.turn.continuations,
        interrupted: state.turn.interrupted,
        comparison: comparison.clone(),
        completion: scanned.outcome.completion.clone(),
        stale: scanned.stale,
        current: scanned.scan.findings.len(),
    };
    let decision = policy::decide(&facts);
    let response = match decision {
        Decision::Block => {
            state.turn.continuations += 1;
            let blocking: Vec<Finding> = comparison
                .as_ref()
                .map(|c| policy::blocking(c).into_iter().cloned().collect())
                .unwrap_or_default();
            Response::Block(block_message(&blocking, scanned.report.as_deref()))
        }
        Decision::Advise => {
            let said = completion_message(&state, &facts, &scanned, unavailable, settings);
            match once(&mut state, "completion", said) {
                Some(text) => {
                    if host.carries_prompt_context() {
                        state.pending = Some(text.clone());
                    }
                    Response::Advise(text)
                }
                None => Response::Nothing,
            }
        }
        Decision::Silent => Response::Nothing,
    };
    state.last = Some(scanned.scan);
    session.save(state);
    response
}

fn block_message(findings: &[Finding], report: Option<&std::path::Path>) -> String {
    message::compose(
        &format!(
            "stilltrue: {} documentation finding(s) newly observed since this session began. \
             The repository no longer matches what these documents say.",
            findings.len()
        ),
        findings,
        "Update each document so the claim describes the repository again, or restore what \
         it names if removing it was a mistake. Do not delete or weaken the instruction, add a \
         suppression marker, or disable stilltrue to make this pass. If you cannot repair it, \
         say so and stop: CI checks again on its own. Newly observed is not the same as \
         introduced by you — someone else may have changed the repository.",
        report,
    )
}

fn completion_message(
    state: &State,
    facts: &Facts,
    scanned: &Scanned,
    unavailable: Option<&str>,
    settings: &Settings,
) -> Said {
    let report = scanned.report.as_deref();
    let current = &scanned.scan.findings;
    let carried: Vec<Finding> = state
        .carried
        .iter()
        .filter(|c| current.iter().any(|f| f.fingerprint == c.fingerprint))
        .cloned()
        .collect();
    let carried_note = if carried.is_empty() {
        String::new()
    } else {
        format!(
            "{} finding(s) carried from before this session's comparison was reset are still present.",
            carried.len()
        )
    };
    let with_carried = |kind: &str, sets: &[&[Finding]], text: String| {
        let mut all: Vec<&[Finding]> = sets.to_vec();
        all.push(&carried);
        Said::new(kind, &all, text)
    };
    if let Some(text) = unverified(&facts.completion, settings) {
        let text = if facts.recursion {
            format!("{text} The repair after the last check is unverified.")
        } else {
            text
        };
        let kind = format!("unverified:{}", scanned.scan.completion);
        return with_carried(
            &kind,
            &[],
            message::compose(&text, &[], &carried_note, None),
        );
    }
    if facts.stale {
        let text = message::compose(
            "stilltrue: files changed while the check ran, so its result is advisory only and \
             certifies nothing; the next check will look again.",
            current,
            &carried_note,
            report,
        );
        return with_carried("stale", &[current], text);
    }
    if let Completion::Incomplete(reasons) = &facts.completion {
        let text = message::compose(
            &format!(
                "stilltrue: the check was incomplete ({}); these findings are advisory, and its \
                 silence is not evidence the documents are current.",
                reasons.join(", ")
            ),
            current,
            &carried_note,
            report,
        );
        return with_carried(
            &format!("incomplete:{}", reasons.join(",")),
            &[current],
            text,
        );
    }
    if let Completion::NoInput = facts.completion {
        return Said::new(
            "no-input",
            &[],
            "stilltrue: no documents matched, so nothing was checked.".to_string(),
        );
    }
    let Some(comparison) = &facts.comparison else {
        let reason = unavailable.unwrap_or("unavailable");
        let text = message::compose(
            &format!(
                "stilltrue (advisory): no comparison with the start of this session is available \
                 ({reason}), so these are the current findings, not necessarily new ones."
            ),
            current,
            &carried_note,
            report,
        );
        return with_carried(&format!("uncompared:{reason}"), &[current], text);
    };
    if facts.recursion {
        let remaining: Vec<Finding> = policy::blocking(comparison).into_iter().cloned().collect();
        if remaining.is_empty() {
            let text = message::compose(
                "stilltrue: verified — the findings newly observed this turn are resolved.",
                &comparison.new,
                &carried_note,
                None,
            );
            return with_carried("verified", &[&comparison.new], text);
        }
        let text = message::compose(
            &format!(
                "stilltrue: {} newly observed finding(s) remain unresolved after the repair \
                 attempt. CI keeps its own gate.",
                remaining.len()
            ),
            &remaining,
            &carried_note,
            report,
        );
        return with_carried("unresolved", &[&remaining], text);
    }
    let text = message::compose(
        &format!(
            "stilltrue (advisory): {} documentation finding(s) newly observed since this session \
             began — from this session's edits, or from someone else's.",
            comparison.new.len()
        ),
        &comparison.new,
        &carried_note,
        report,
    );
    with_carried("new", &[&comparison.new], text)
}

/// Settings from the environment, for hosts whose hook command lines are awkward to
/// vary. Flags given on the command line win.
pub fn environment() -> BTreeMap<&'static str, String> {
    [
        "STILLTRUE_HOOK_MODE",
        "STILLTRUE_HOOK_TIMEOUT",
        "STILLTRUE_HOOK_STATE",
        "STILLTRUE_HOOK_BASELINE",
        "STILLTRUE_BIN",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok().map(|value| (name, value)))
    .collect()
}
