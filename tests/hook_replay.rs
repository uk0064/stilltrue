//! Seam: the adapter against each host's payloads and output contracts (ADR-0023).
//!
//! Every payload under tests/hook/payloads is replayed through the real binary, in a
//! session that sees a clean start, a code-only edit that breaks a document, and a
//! repair. Every answer is held to the shapes the host documents for that event: JSON or
//! nothing, never plain text at a Codex Stop, a block only at Stop, and exit 0 always.
//! The packaged hook configurations are checked against the same contracts.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

const HOOK: &str = env!("CARGO_BIN_EXE_stilltrue-hook");
const CHECKER: &str = env!("CARGO_BIN_EXE_stilltrue");

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn payload(host: &str, name: &str, cwd: &Path) -> Value {
    let path = root().join("tests/hook/payloads").join(host).join(name);
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    value["cwd"] = Value::String(cwd.to_string_lossy().into_owned());
    value
}

fn replay(host: &str, value: &Value, state: &Path, block: bool) -> String {
    let mut command = Command::new(HOOK);
    command
        .arg(host)
        .args(["--checker", CHECKER, "--state-dir", state.to_str().unwrap()]);
    if block {
        command.arg("--block");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{host} {value}");
    String::from_utf8(output.stdout).unwrap()
}

/// The answer, held to what `host` documents for `event`.
fn conforms(host: &str, event: &str, out: &str) {
    if out.is_empty() {
        return; // Exit 0 with no output is success on both hosts.
    }
    let value: Value = serde_json::from_str(out)
        .unwrap_or_else(|e| panic!("{host} {event}: not JSON ({e}): {out:?}"));
    let object = value.as_object().expect("a JSON object");
    let keys: Vec<&str> = object.keys().map(String::as_str).collect();
    match event {
        "SessionStart" | "UserPromptSubmit" => {
            assert_eq!(keys, ["hookSpecificOutput"], "{out}");
            let specific = &value["hookSpecificOutput"];
            assert_eq!(specific["hookEventName"], event);
            assert!(
                specific["additionalContext"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
            );
            assert!(
                specific["additionalContext"].as_str().unwrap().len() <= 4096,
                "context over 4 KiB"
            );
        }
        "Stop" => match keys.as_slice() {
            ["systemMessage"] => assert!(value["systemMessage"].as_str().is_some()),
            ["decision", "reason"] => {
                assert_eq!(value["decision"], "block");
                // Codex fails a block whose reason is empty or whitespace.
                assert!(
                    value["reason"]
                        .as_str()
                        .is_some_and(|r| !r.trim().is_empty())
                );
                assert!(value["reason"].as_str().unwrap().len() <= 4096);
            }
            other => panic!("{host} Stop answered with {other:?}"),
        },
        other => panic!("{host} {other} should be answered with nothing: {out}"),
    }
}

fn session(host: &str, events: &[(&str, &str)], block: bool) -> Vec<(String, String)> {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let repo = repo.canonicalize().unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(
        repo.join("Makefile"),
        "demo:\n\techo demo\n\nseed:\n\techo seed\n",
    )
    .unwrap();
    std::fs::write(repo.join("AGENTS.md"), "Run `make demo` first.\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "add demo"]);
    let state = dir.path().join("state");
    let mut answers = Vec::new();
    for (file, action) in events {
        match *action {
            "break" => std::fs::write(repo.join("Makefile"), "seed:\n\techo seed\n").unwrap(),
            "repair" => std::fs::write(repo.join("AGENTS.md"), "Run `make seed` first.\n").unwrap(),
            _ => {}
        }
        let value = payload(host, file, &repo);
        let event = value["hook_event_name"].as_str().unwrap().to_string();
        let out = replay(host, &value, &state, block);
        conforms(host, &event, &out);
        answers.push((event, out));
    }
    answers
}

#[test]
fn claude_code_payloads_replay_to_conforming_answers() {
    for block in [false, true] {
        let answers = session(
            "claude-code",
            &[
                ("session-start.json", ""),
                ("user-prompt-submit.json", ""),
                ("stop.json", "break"),
                ("stop-active.json", "repair"),
                ("user-prompt-submit.json", ""),
            ],
            block,
        );
        let stop = &answers[2].1;
        if block {
            assert!(stop.contains("\"decision\":\"block\""), "{stop}");
        } else {
            assert!(stop.contains("systemMessage"), "{stop}");
        }
        assert!(answers[3].1.contains("verified"), "{:?}", answers[3]);
    }
}

#[test]
fn codex_payloads_replay_to_conforming_answers() {
    for block in [false, true] {
        let answers = session(
            "codex",
            &[
                ("session-start.json", ""),
                ("stop.json", "break"),
                ("stop-active.json", "repair"),
                ("interrupt.json", ""),
            ],
            block,
        );
        assert_eq!(answers[1].1.contains("\"decision\":\"block\""), block);
        assert_eq!(answers[3].1, "", "Interrupt is answered with nothing");
    }
}

#[test]
fn every_payload_on_disk_is_replayed_by_a_test() {
    // A payload added without a replay would look covered and be untested.
    for host in ["claude-code", "codex"] {
        let dir = root().join("tests/hook/payloads").join(host);
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json") && !n.starts_with("recorded-"))
            .collect();
        files.sort();
        let expected: &[&str] = match host {
            "claude-code" => &[
                "session-start.json",
                "stop-active.json",
                "stop.json",
                "user-prompt-submit.json",
            ],
            _ => &[
                "interrupt.json",
                "session-start.json",
                "stop-active.json",
                "stop.json",
            ],
        };
        assert_eq!(files, expected, "{host}");
    }
}

#[test]
fn recorded_payloads_parse_and_replay() {
    // Captured from a live host with STILLTRUE_HOOK_RECORD. Whatever the host really
    // sent must still be understood.
    let mut seen = 0;
    for host in ["claude-code", "codex"] {
        let dir = root().join("tests/hook/payloads").join(host);
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("recorded-") {
                continue;
            }
            let state = tempfile::tempdir().unwrap();
            let repo = tempfile::tempdir().unwrap();
            let value = payload(host, &name, repo.path());
            let event = value["hook_event_name"].as_str().unwrap().to_string();
            let out = replay(host, &value, state.path(), false);
            conforms(host, &event, &out);
            seen += 1;
        }
    }
    // Three were captured from Claude Code; a loop that found none would pass silently.
    assert!(seen >= 3, "only {seen} recorded payload(s) found");
}

/// Every hook a packaged configuration registers.
fn registered(config: &Path) -> Vec<(String, String, u64)> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
    let mut out = Vec::new();
    for (event, groups) in value["hooks"].as_object().unwrap() {
        for group in groups.as_array().unwrap() {
            for hook in group["hooks"].as_array().unwrap() {
                assert_eq!(hook["type"], "command");
                out.push((
                    event.clone(),
                    hook["command"].as_str().unwrap().to_string(),
                    hook["timeout"].as_u64().unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_packaged_hook_configurations_register_what_the_adapter_handles() {
    let claude = registered(&root().join("integrations/claude-code/hooks/hooks.json"));
    let events: Vec<&str> = claude.iter().map(|(e, _, _)| e.as_str()).collect();
    assert_eq!(events, ["SessionStart", "Stop", "UserPromptSubmit"]);
    let codex = registered(&root().join("integrations/codex/hooks.json"));
    let events: Vec<&str> = codex.iter().map(|(e, _, _)| e.as_str()).collect();
    assert_eq!(events, ["Interrupt", "SessionStart", "Stop"]);
    for (host, hooks) in [("claude-code", &claude), ("codex", &codex)] {
        for (event, command, timeout) in hooks.iter() {
            assert_eq!(command, &format!("stilltrue-hook {host}"), "{host} {event}");
            // The host's budget has to cover the adapter's own five-second scan plus the
            // identity it computes around it, or the host kills a scan that would have
            // finished and the adapter's own timeout message is never seen.
            if event != "Interrupt" && event != "UserPromptSubmit" {
                assert!(*timeout >= 15, "{host} {event}: {timeout}s");
            }
        }
    }
    // Codex allows Interrupt at most three seconds.
    assert!(codex.iter().all(|(e, _, t)| e != "Interrupt" || *t <= 3));
}

#[test]
fn the_claude_code_plugin_manifest_names_the_plugin() {
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(
            root().join("integrations/claude-code/.claude-plugin/plugin.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["name"], "stilltrue");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    let skill =
        std::fs::read_to_string(root().join("integrations/claude-code/skills/stilltrue/SKILL.md"))
            .unwrap();
    assert!(skill.starts_with("---\nname: stilltrue\ndescription: "));
}
