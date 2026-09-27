//! The `stilltrue` command.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

use std::collections::BTreeSet;

use stilltrue::baseline;
use stilltrue::coverage::Diagnostic;
use stilltrue::fix::{self, EditStatus};
use stilltrue::gate::{Finding, Tier};
use stilltrue::pipeline::{self, Options, Outcome};
use stilltrue::repo::Repo;
use stilltrue::report::{Degraded, Format};
use stilltrue::runreport;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    /// One finding per line, for a terminal.
    Human,
    /// SARIF 2.1.0, for GitHub code scanning.
    Sarif,
    /// Structured findings, for your own tooling.
    Json,
    /// GitHub workflow-command annotations, for use inside an Action.
    Github,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FailOn {
    /// Exit 1 on any Tier A finding.
    Rot,
    /// Exit 1 on any reported finding.
    Any,
    /// Never exit 1, for advisory runs.
    None,
}

/// A linter for the instructions you give your coding agent — and for your README.
///
/// Reads README*, docs/**, CLAUDE.md, AGENTS.md, .cursor/rules/**, **/SKILL.md and
/// .github/copilot-instructions.md, minus changelogs and release notes. Configure it in
/// stilltrue.toml if you must — but note that `include` there *replaces* that default
/// set rather than extending it, so `include = ["docs/**"]` silently stops linting
/// CLAUDE.md. `exclude` does extend the defaults.
#[derive(Debug, Parser)]
#[command(name = "stilltrue", version, about, long_about)]
struct Cli {
    /// Filter the document set to these paths. Excludes still apply.
    paths: Vec<PathBuf>,

    /// Also report Tier B (lie): claims that are broken with no sign they ever worked.
    #[arg(long)]
    strict: bool,

    /// Output format. All of them obey the same gate.
    #[arg(long, value_enum, default_value = "human")]
    format: FormatArg,

    /// Which findings make the exit code 1.
    #[arg(long, value_enum, default_value = "rot")]
    fail_on: FailOn,

    // Declared as the positive it is used as, so the negation lives in clap rather than
    // in a `!` at the use site. `main` cannot be unit-tested, so a hand-written
    // inversion there is a correctness claim no test in `cargo test` can reach.
    /// Ignore the result cache and re-check every claim.
    #[arg(long = "no-cache", action = clap::ArgAction::SetFalse)]
    cache: bool,

    /// Exit 2 when git history is unavailable, instead of running blind.
    #[arg(long)]
    require_history: bool,

    /// Stay silent on findings already recorded in this file.
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,

    /// Record the current findings to this file and exit 0.
    #[arg(long, value_name = "FILE", conflicts_with = "baseline")]
    write_baseline: Option<PathBuf>,

    /// Rewrite claims that have exactly one candidate, and print every change.
    #[arg(long, conflicts_with = "fix_dry_run")]
    fix: bool,

    /// Print the rewrites `--fix` would make, as a unified diff on stderr, and write
    /// nothing. The exit code is decided by the findings, exactly as without it.
    #[arg(long)]
    fix_dry_run: bool,

    /// Say what was examined and what was not, on stderr: documents read, claims
    /// checked, skipped and ambiguous, and whether the run was complete.
    #[arg(long)]
    summary: bool,

    /// Also write a versioned JSON run report to FILE: status, coverage, diagnostics
    /// and the same findings as stdout.
    #[arg(long, value_name = "FILE")]
    report_file: Option<PathBuf>,

    /// Also write SARIF to FILE, from the same analysis as stdout.
    #[arg(long, value_name = "FILE")]
    sarif_file: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let repo = Repo::discover(&cwd);

    let repo_root = repo.root().to_path_buf();
    let outcome = pipeline::run(&Options {
        root: repo.root().to_path_buf(),
        cwd: cwd.clone(),
        paths: cli.paths.clone(),
        strict: cli.strict,
        cache: cli.cache,
    });
    let mut diagnostics = outcome.diagnostics.clone();

    let format = match cli.format {
        FormatArg::Human => Format::Human,
        FormatArg::Sarif => Format::Sarif,
        FormatArg::Json => Format::Json,
        FormatArg::Github => Format::Github,
    };

    if let Some(banner) = outcome.degraded.banner() {
        eprintln!("{banner}");
        // Inside an Action, stderr is a footnote in a collapsed log. ADR-0006 says a
        // degraded run is loud, so it becomes an annotation like anything else.
        if matches!(format, Format::Github) {
            println!("::warning::{}", banner.replace('\n', " "));
        }
    }

    // Every success path comes after this, baseline writing included: a baseline
    // recorded from a run that could not see history records nothing but lies, and
    // `--require-history` exists to refuse exactly that run.
    if cli.require_history && outcome.degraded.is_degraded() {
        let everything = baseline::partition(&outcome.findings, &BTreeSet::new()).0;
        return finish(&cli, &outcome, &everything, 0, &diagnostics, 2, None);
    }

    if let Some(path) = &cli.write_baseline {
        let code = match baseline::write(path, &outcome.findings) {
            Ok(count) => {
                eprintln!("stilltrue: recorded {count} findings in {}", path.display());
                0
            }
            Err(error) => {
                eprintln!("stilltrue: could not write {}: {error}", path.display());
                2
            }
        };
        let everything = baseline::partition(&outcome.findings, &BTreeSet::new()).0;
        return finish(&cli, &outcome, &everything, 0, &diagnostics, code, None);
    }

    // The baseline is applied once, here, and every output below reads the result:
    // stdout, the SARIF file, the report and the summary cannot disagree (ADR-0021).
    let known = match &cli.baseline {
        Some(path) => baseline::try_load(path).unwrap_or_else(|error| {
            eprintln!(
                "stilltrue: warning: could not read baseline {}: {error}",
                path.display()
            );
            diagnostics.push(Diagnostic::failure(
                "baseline-unreadable",
                format!("the baseline could not be read, so nothing was suppressed: {error}"),
                Some(path.clone()),
            ));
            BTreeSet::new()
        }),
        None => BTreeSet::new(),
    };
    let (shown, suppressed) = baseline::partition(&outcome.findings, &known);
    let findings: Vec<Finding> = shown.iter().map(|(f, _)| f.clone()).collect();

    // The banner already went to stderr, for every format.
    print!(
        "{}",
        stilltrue::report::render_with(
            &findings,
            Degraded::No,
            format,
            stilltrue::report::Colour::detect(),
        )
    );

    let mut output_failed = false;
    if let Some(path) = &cli.sarif_file {
        // The same findings as stdout, from the same analysis: never a second scan.
        let sarif = stilltrue::report::render(&findings, Degraded::No, Format::Sarif);
        if let Err(error) = std::fs::write(path, sarif) {
            eprintln!("stilltrue: could not write {}: {error}", path.display());
            diagnostics.push(Diagnostic::note(
                "output-unwritable",
                format!("the SARIF file could not be written: {error}"),
                Some(path.clone()),
            ));
            output_failed = true;
        }
    }

    // Both modes exit on what was *found*, not on what was changed: a CI job must not
    // be able to turn green by editing the repository (ADR-0015). Preview and rewrite
    // share one plan, so the diff is exactly what `--fix` would write (ADR-0022).
    let mut edits = None;
    if cli.fix || cli.fix_dry_run {
        let plan = fix::plan(&findings, &repo_root);
        let records = if cli.fix_dry_run {
            eprint!("{}", fix::diff(&plan));
            let records = plan.records();
            let eligible = records
                .iter()
                .filter(|r| r.status == EditStatus::Eligible)
                .count();
            eprintln!(
                "stilltrue: would rewrite {eligible} of {} findings; nothing was written",
                findings.len()
            );
            records
        } else {
            let records = fix::apply(&plan, &repo_root);
            let applied = records
                .iter()
                .filter(|r| r.status == EditStatus::Applied)
                .count();
            eprintln!(
                "stilltrue: rewrote {applied} of {} findings",
                findings.len()
            );
            records
        };
        for record in &records {
            if let EditStatus::Rejected(why) = record.status {
                eprintln!(
                    "stilltrue: warning: not rewriting `{}` in {}: {}",
                    record.edit.expected,
                    record.edit.file.display(),
                    why.code()
                );
                diagnostics.push(Diagnostic::note(
                    "fix-rejected",
                    format!(
                        "`{}` was not rewritten: {}",
                        record.edit.expected,
                        why.code()
                    ),
                    Some(record.edit.file.clone()),
                ));
            }
        }
        edits = Some(fix::records_json(&records));
    }

    let fatal = match cli.fail_on {
        FailOn::None => false,
        FailOn::Any => !findings.is_empty(),
        FailOn::Rot => findings.iter().any(|f| f.tier == Tier::Rot),
    };
    // Failing to write an output that was explicitly asked for is an output failure,
    // not a finding: the caller cannot tell a missing file from a clean run.
    let code = if output_failed { 2 } else { u8::from(fatal) };
    finish(
        &cli,
        &outcome,
        &shown,
        suppressed,
        &diagnostics,
        code,
        edits,
    )
}

/// Write the run report and the summary, if asked for, and exit with `code` — or with
/// 2, if the report itself could not be written.
fn finish(
    cli: &Cli,
    outcome: &Outcome,
    shown: &[(Finding, String)],
    suppressed: usize,
    diagnostics: &[Diagnostic],
    code: u8,
    edits: Option<serde_json::Value>,
) -> ExitCode {
    let fail_on = match cli.fail_on {
        FailOn::Rot => "rot",
        FailOn::Any => "any",
        FailOn::None => "none",
    };
    let view = runreport::View {
        outcome,
        findings: shown,
        baseline_suppressed: suppressed,
        diagnostics,
        fail_on,
        baseline: cli.baseline.as_deref(),
        paths: &cli.paths,
        exit_code: code,
        edits,
    };
    let mut code = code;
    if let Some(path) = &cli.report_file {
        let mut text = serde_json::to_string_pretty(&runreport::envelope(&view))
            .unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        if let Err(error) = std::fs::write(path, text) {
            eprintln!("stilltrue: could not write {}: {error}", path.display());
            code = 2;
        }
    }
    if cli.summary {
        eprint!("{}", runreport::summary(&view));
    }
    ExitCode::from(code)
}
