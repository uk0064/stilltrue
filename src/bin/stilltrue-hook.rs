//! `stilltrue-hook`: the Claude Code and Codex adapter (ADR-0023).
//!
//! Reads one hook payload on stdin and writes the host's answer on stdout. It always
//! exits 0. That is the contract, not a convenience: Claude Code reads a Stop hook's
//! exit status of 2 as "keep going", so an adapter that let any failure — a bad flag, a
//! missing checker, a panic — surface as 2 could loop an agent on nothing.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

use stilltrue::hook::{self, DEFAULT_TIMEOUT, Settings, host::Host, policy::Mode};

/// Run stilltrue at an agent's session entry and completion.
///
/// Install it as a command hook for SessionStart, UserPromptSubmit and Stop (Claude
/// Code), or SessionStart, Stop and Interrupt (Codex); see docs/agents.md. Advisory by
/// default. Every flag can also be set in the environment, for hosts whose hook command
/// lines are awkward to vary: STILLTRUE_HOOK_MODE=block, STILLTRUE_HOOK_TIMEOUT,
/// STILLTRUE_HOOK_BASELINE, STILLTRUE_HOOK_STATE and STILLTRUE_BIN. STILLTRUE_HOOK_RECORD
/// names a directory to keep each payload in, for diagnosis.
#[derive(Debug, Parser)]
#[command(name = "stilltrue-hook", version, about, long_about)]
struct Cli {
    /// The host whose protocol to speak: claude-code or codex.
    host: String,

    /// Ask for one corrective continuation when a completion check newly observes rot.
    /// Off by default; see docs/agents.md before turning it on.
    #[arg(long)]
    block: bool,

    /// Seconds a scan may take before it is abandoned as unverified.
    #[arg(long, value_name = "SECONDS")]
    timeout: Option<f64>,

    /// A baseline file, relative to the repository root, applied to every scan.
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,

    /// Where session state is kept. Defaults to the platform cache directory.
    #[arg(long, value_name = "DIR")]
    state_dir: Option<PathBuf>,

    /// The stilltrue binary to run. Defaults to the one beside this binary, then PATH.
    #[arg(long, value_name = "PATH")]
    checker: Option<PathBuf>,
}

/// Terminated by the host — a timeout, the user stopping the agent — the adapter takes
/// the scan it started with it, then exits 0: silence, never a request to continue.
#[cfg(unix)]
fn clean_up_when_terminated() {
    extern "C" fn terminated(_: libc::c_int) {
        hook::scan::kill_group(hook::scan::RUNNING.load(std::sync::atomic::Ordering::SeqCst));
        // SAFETY: _exit is async-signal-safe, and is the only way out of a handler.
        unsafe { libc::_exit(0) };
    }
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: the handler touches only an atomic, killpg and _exit, all of which
        // are async-signal-safe.
        unsafe {
            libc::signal(signal, terminated as *const () as libc::sighandler_t);
        }
    }
}

fn main() -> ExitCode {
    #[cfg(unix)]
    clean_up_when_terminated();
    // `--help` and `--version` are the only reasons to print clap's own output, and they
    // exit 0 anyway. Any other parse error is reported on stderr and answered with
    // silence — never with clap's status of 2.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
    };
    let Some(host) = Host::parse(&cli.host) else {
        eprintln!(
            "stilltrue-hook: unknown host `{}`; expected claude-code or codex",
            cli.host
        );
        return ExitCode::SUCCESS;
    };
    let env = hook::environment();
    let mode = if cli.block || env.get("STILLTRUE_HOOK_MODE").is_some_and(|m| m == "block") {
        Mode::Block
    } else {
        Mode::Advisory
    };
    let timeout = cli
        .timeout
        .or_else(|| {
            env.get("STILLTRUE_HOOK_TIMEOUT")
                .and_then(|t| t.parse().ok())
        })
        .and_then(|t: f64| Duration::try_from_secs_f64(t).ok())
        .filter(|t| !t.is_zero())
        .unwrap_or(DEFAULT_TIMEOUT);
    let checker = cli
        .checker
        .or_else(|| env.get("STILLTRUE_BIN").map(PathBuf::from));
    let settings = Settings {
        mode,
        timeout,
        baseline: cli
            .baseline
            .or_else(|| env.get("STILLTRUE_HOOK_BASELINE").map(PathBuf::from)),
        state_dir: cli
            .state_dir
            .or_else(|| env.get("STILLTRUE_HOOK_STATE").map(PathBuf::from))
            .unwrap_or_else(hook::state::Store::default_dir),
        checker: hook::scan::locate(checker.as_deref()),
        now: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    };

    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("stilltrue-hook: could not read the hook input: {error}");
        return ExitCode::SUCCESS;
    }
    record(host, &input);
    // A panic is a bug, and still not a request to continue.
    match std::panic::catch_unwind(|| hook::handle(host, &input, &settings)) {
        Ok(output) => print!("{output}"),
        Err(_) => eprintln!("stilltrue-hook: internal error; the check was skipped"),
    }
    ExitCode::SUCCESS
}

/// With STILLTRUE_HOOK_RECORD set to a directory, keep each payload exactly as the host
/// sent it. This is how the replay fixtures under tests/hook/payloads are captured from
/// a live host, rather than written from its documentation.
fn record(host: Host, input: &str) {
    let Some(dir) = std::env::var_os("STILLTRUE_HOOK_RECORD") else {
        return;
    };
    let event = serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|v| v["hook_event_name"].as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(
        dir.join(format!("{}-{event}-{stamp}.json", host.slug())),
        input,
    );
}
