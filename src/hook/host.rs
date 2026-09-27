//! The two hosts' hook protocols: what arrives on stdin, and what may be written back.
//!
//! Similarly named events do not share contracts. Claude Code reads exit 2 from a Stop
//! hook as "keep going" and accepts plain text or nothing on stdout; Codex requires
//! JSON from a Stop hook that exits 0, and has no project directory variable but sends
//! `turn_id`. The adapter therefore always exits 0 and only ever writes JSON a host
//! documents — so no exit status of its own, or of the checker, can reach a host as a
//! request to continue.

use std::path::PathBuf;

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    ClaudeCode,
    Codex,
}

impl Host {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude-code" | "claude" => Some(Host::ClaudeCode),
            "codex" => Some(Host::Codex),
            _ => None,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Host::ClaudeCode => "claude-code",
            Host::Codex => "codex",
        }
    }

    /// Whether this host can carry context into a user turn at `UserPromptSubmit`.
    /// Claude Code documents it. Codex's contract for it was not confirmed when this was
    /// written, so completion advice there stays a user-visible warning only.
    pub fn carries_prompt_context(self) -> bool {
        matches!(self, Host::ClaudeCode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Startup,
    Resume,
    Clear,
    Compact,
    Fork,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    SessionStart(Source),
    UserPrompt,
    Stop {
        /// `stop_hook_active`: this stop follows a continuation a stop hook requested.
        recursion: bool,
        /// Codex's `turn_id`. Claude Code sends none; its turns are counted at
        /// `UserPromptSubmit` instead.
        turn: Option<String>,
    },
    /// Codex's `Interrupt`: the user stopped the turn.
    Interrupt {
        turn: Option<String>,
    },
    /// An event this adapter does not handle. Answered with nothing.
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    pub session: String,
    pub cwd: PathBuf,
}

/// Read a hook payload. Unknown fields are ignored — both hosts add them release by
/// release — and a missing required one is an error the caller answers with silence.
pub fn parse(input: &str) -> Result<Event, String> {
    let value: Value =
        serde_json::from_str(input).map_err(|e| format!("the hook input is not JSON: {e}"))?;
    let text = |key: &str| {
        value[key]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("the hook input has no `{key}`"))
    };
    let session = text("session_id")?;
    let cwd = PathBuf::from(text("cwd")?);
    let kind = match text("hook_event_name")?.as_str() {
        "SessionStart" => Kind::SessionStart(match value["source"].as_str().unwrap_or("") {
            "startup" => Source::Startup,
            "resume" => Source::Resume,
            "clear" => Source::Clear,
            "compact" => Source::Compact,
            "fork" => Source::Fork,
            other => Source::Other(other.to_string()),
        }),
        "UserPromptSubmit" => Kind::UserPrompt,
        "Stop" => Kind::Stop {
            recursion: value["stop_hook_active"].as_bool().unwrap_or(false),
            turn: value["turn_id"].as_str().map(str::to_string),
        },
        "Interrupt" => Kind::Interrupt {
            turn: value["turn_id"].as_str().map(str::to_string),
        },
        other => Kind::Other(other.to_string()),
    };
    Ok(Event { kind, session, cwd })
}

/// What the adapter wants to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Nothing,
    /// Context for the agent: at session entry, or at the next user prompt.
    Context(String),
    /// A warning for the user at a completion boundary. Does not continue the turn.
    Advise(String),
    /// Ask for one more pass, with this reason.
    Block(String),
}

/// The bytes to write on stdout. Empty means "nothing to say", which both hosts read as
/// success; everything else is one JSON object in a shape both hosts document for that
/// event. Where the hosts differ — Codex has no confirmed prompt-context contract — the
/// engine does not produce the response at all (see `carries_prompt_context`).
pub fn render(kind: &Kind, response: &Response) -> String {
    let value = match (kind, response) {
        (Kind::SessionStart(_), Response::Context(text) | Response::Advise(text)) => json!({
            "hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}
        }),
        (Kind::UserPrompt, Response::Context(text)) => json!({
            "hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": text}
        }),
        (Kind::Stop { .. }, Response::Advise(text) | Response::Context(text)) => {
            json!({"systemMessage": text})
        }
        (Kind::Stop { .. }, Response::Block(reason)) => {
            json!({"decision": "block", "reason": reason})
        }
        // Nothing to say; a block anywhere but Stop; advice at an event with no place
        // to show it: say nothing rather than something the host did not ask for.
        _ => return String::new(),
    };
    format!("{value}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_with_extra_fields_parses() {
        let event = parse(
            r#"{"session_id":"s","cwd":"/w","hook_event_name":"Stop","stop_hook_active":true,
                "turn_id":"t9","model":"m","last_assistant_message":"done","permission_mode":"default"}"#,
        )
        .unwrap();
        assert_eq!(
            event.kind,
            Kind::Stop {
                recursion: true,
                turn: Some("t9".into())
            }
        );
        assert_eq!(event.session, "s");
    }

    #[test]
    fn a_payload_missing_a_required_field_is_an_error() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"cwd":"/w","hook_event_name":"Stop"}"#).is_err());
        assert!(parse(r#"{"session_id":"s","hook_event_name":"Stop"}"#).is_err());
        assert!(parse(r#"{"session_id":"s","cwd":"/w"}"#).is_err());
    }

    #[test]
    fn every_source_is_read() {
        for (text, source) in [
            ("startup", Source::Startup),
            ("resume", Source::Resume),
            ("clear", Source::Clear),
            ("compact", Source::Compact),
            ("fork", Source::Fork),
            ("later", Source::Other("later".into())),
        ] {
            let event = parse(&format!(
                r#"{{"session_id":"s","cwd":"/w","hook_event_name":"SessionStart","source":"{text}"}}"#
            ))
            .unwrap();
            assert_eq!(event.kind, Kind::SessionStart(source));
        }
    }

    #[test]
    fn a_block_is_only_ever_written_at_stop() {
        let block = Response::Block("r".into());
        let stop = Kind::Stop {
            recursion: false,
            turn: None,
        };
        let out: Value = serde_json::from_str(&render(&stop, &block)).unwrap();
        assert_eq!(out["decision"], "block");
        assert_eq!(out["reason"], "r");
        for kind in [
            Kind::SessionStart(Source::Startup),
            Kind::UserPrompt,
            Kind::Interrupt { turn: None },
        ] {
            assert_eq!(render(&kind, &block), "", "{kind:?}");
        }
    }

    #[test]
    fn only_claude_code_carries_advice_into_the_next_prompt() {
        assert!(Host::ClaudeCode.carries_prompt_context());
        assert!(!Host::Codex.carries_prompt_context());
        assert_eq!(Host::parse("claude"), Some(Host::ClaudeCode));
        assert_eq!(
            Host::parse("claude-code").map(Host::slug),
            Some("claude-code")
        );
        assert_eq!(Host::parse("codex").map(Host::slug), Some("codex"));
        assert_eq!(Host::parse("cursor"), None);
    }

    #[test]
    fn nothing_is_an_empty_stdout() {
        for kind in [
            Kind::SessionStart(Source::Startup),
            Kind::Stop {
                recursion: false,
                turn: None,
            },
        ] {
            assert_eq!(render(&kind, &Response::Nothing), "");
        }
    }
}
