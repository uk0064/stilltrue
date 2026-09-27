//! Stage 4: render findings.
//!
//! Machine formats obey exactly the same gate as human output: one gate, one truth
//! (ADR-0008).

use std::collections::BTreeMap;
use std::path::Path;

use crate::claim::ClaimKind;
use crate::gate::{Finding, Tier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
    Sarif,
    /// GitHub workflow commands, one annotation per line.
    Github,
}

/// Why history could not be searched, if it could not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degraded {
    No,
    NoRepository,
    ShallowClone,
    PartialClone,
}

impl Degraded {
    pub fn is_degraded(self) -> bool {
        self != Degraded::No
    }

    /// The history state as the run report names it.
    pub fn slug(self) -> &'static str {
        match self {
            Degraded::No => "complete",
            Degraded::NoRepository => "no-repository",
            Degraded::ShallowClone => "shallow",
            Degraded::PartialClone => "partial",
        }
    }

    /// Loud, never a footnote (ADR-0006).
    pub fn banner(self) -> Option<String> {
        let cause = match self {
            Degraded::No => return None,
            Degraded::NoRepository => "no git repository",
            Degraded::ShallowClone => "shallow clone",
            Degraded::PartialClone => "partial clone",
        };
        let fix = match self {
            Degraded::PartialClone => {
                "stilltrue: fix: check out without `--filter`, or pass --require-history to fail instead."
            }
            _ => {
                "stilltrue: fix: check out with `fetch-depth: 0`, or pass --require-history to fail instead."
            }
        };
        Some(format!(
            "stilltrue: history unavailable ({cause}) — nothing can be reported as rot.\n{fix}"
        ))
    }
}

/// The degraded banner is not rendered here: it goes to stderr for every format, so
/// that a machine-format run is still told, and a human run is not told twice.
/// Git's `%cr` phrasing, computed at render time rather than stored.
pub fn relative_date(timestamp: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(timestamp);
    relative_to(timestamp, now)
}

/// The same, against a stated present. Every boundary here is one second wide, so a
/// test that read the clock for itself would race the one this function reads.
pub fn relative_to(timestamp: i64, now: i64) -> String {
    let seconds = (now - timestamp).max(0);
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    // Git's own rounding: a "month" is a twelfth of a 365.25-day year.
    const MONTH: i64 = 2_629_800;
    const YEAR: i64 = 12 * MONTH;
    match seconds {
        s if s < 90 => plural(s.max(1), "second"),
        s if s < 90 * MINUTE => plural((s + MINUTE / 2) / MINUTE, "minute"),
        s if s < 36 * HOUR => plural((s + HOUR / 2) / HOUR, "hour"),
        s if s < 14 * DAY => plural((s + DAY / 2) / DAY, "day"),
        s if s < 10 * 7 * DAY => plural((s + 7 * DAY / 2) / (7 * DAY), "week"),
        s if s < YEAR => plural((s + MONTH / 2) / MONTH, "month"),
        s => plural((s + YEAR / 2) / YEAR, "year"),
    }
}

/// Whether human output carries ANSI colour. Machine formats never do, whatever this
/// says: a colour code in SARIF is a corrupt document, not a prettier one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Colour {
    #[default]
    Never,
    Always,
}

impl Colour {
    /// Colour when stdout is a terminal and the reader has not asked otherwise.
    ///
    /// `NO_COLOR` is honoured whatever its value, per no-color.org: the variable being
    /// present is the signal. The terminal check is what keeps `stilltrue > out.txt`
    /// and every pipe into `grep` free of escape codes without anyone configuring it.
    pub fn detect() -> Self {
        use std::io::IsTerminal;
        Self::from_signals(
            std::env::var_os("NO_COLOR").is_some(),
            std::io::stdout().is_terminal(),
        )
    }

    /// The decision itself, separated from reading the process it is about.
    ///
    /// `detect` cannot be tested: under `cargo test` stdout is never a terminal, so it
    /// returns `Never` whatever the rule says, and a mutant replacing the whole
    /// function with that answer is indistinguishable. The same seam is already used
    /// for the clock in `relative_to`.
    pub fn from_signals(no_color: bool, is_terminal: bool) -> Self {
        if no_color || !is_terminal {
            Self::Never
        } else {
            Self::Always
        }
    }

    fn paint(self, code: &str, text: &str) -> String {
        match self {
            Self::Never => text.to_string(),
            Self::Always => format!("\x1b[{code}m{text}\x1b[0m"),
        }
    }
}

pub fn render(findings: &[Finding], degraded: Degraded, format: Format) -> String {
    render_with(findings, degraded, format, Colour::Never)
}

pub fn render_with(
    findings: &[Finding],
    degraded: Degraded,
    format: Format,
    colour: Colour,
) -> String {
    match format {
        Format::Human => human(findings, degraded, colour),
        Format::Json => json(findings),
        Format::Sarif => sarif(findings),
        Format::Github => github(findings),
    }
}

/// GitHub workflow-command annotations.
///
/// This lives in the binary rather than in the Action's shell because the Action would
/// otherwise need a runtime to transform JSON, and because escaping is the whole
/// correctness story here: an unescaped `,` in a path silently moves the annotation to
/// another file, and an unescaped newline in a message ends the command and lets the
/// rest be read as a new one.
fn github(findings: &[Finding]) -> String {
    let mut out = String::new();
    for f in findings {
        let level = match f.tier {
            Tier::Rot => "error",
            Tier::Lie => "warning",
        };
        let message = match &f.breaking_commit {
            Some(c) => format!(
                "{} — likely broke in {} \"{}\"",
                f.message, c.sha, c.subject
            ),
            None => f.message.clone(),
        };
        out.push_str(&format!(
            "::{level} file={},line={},col={},endLine={},endColumn={}::{}\n",
            escape_property(&f.claim.file.to_string_lossy()),
            f.claim.line,
            f.claim.column,
            f.claim.end_line,
            f.claim.end_column,
            escape_data(&message),
        ));
    }
    out
}

/// A workflow-command message body.
fn escape_data(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// A workflow-command property value. Properties are separated by `,` and terminated by
/// `:`, so both need escaping on top of the data rules.
///
/// Exactly these five and no more: the runner decodes `%25`, `%0D`, `%0A`, `%3A` and
/// `%2C`, so escaping anything else — a space, say — arrives as the literal escape text
/// and breaks the path it was meant to protect.
fn escape_property(value: &str) -> String {
    escape_data(value).replace(':', "%3A").replace(',', "%2C")
}

/// Suggestions are shown the way the reader would type them.
/// The suggestions as a reader sees them — a command's target carries its runner, so
/// `seed` is offered as `make seed`.
///
/// `fix` uses this too, so what gets written is exactly what was offered.
pub fn suggestions_of(finding: &Finding) -> Vec<String> {
    match &finding.claim.kind {
        ClaimKind::Command { runner, .. } => finding
            .suggestions
            .iter()
            .map(|s| format!("{runner} {s}"))
            .collect(),
        // A version claim is a tool *and* a number, and the candidate is only the
        // number. Offering `22.3.0` for `Node 20` reads as an answer and is one, but
        // `--fix` writes what is offered, so the document became "Use 22.3.0 here."
        ClaimKind::Version { tool, .. } => finding
            .suggestions
            .iter()
            .map(|s| format!("{tool} {s}"))
            .collect(),
        // An anchor candidate is a heading slug, and the claim it replaces is the whole
        // destination. `setup` for `other.md#install` sent `--fix` at the target as
        // well as the fragment, leaving a link to a file named `setup`.
        ClaimKind::Link {
            target,
            anchor: Some(_),
        } => finding
            .suggestions
            .iter()
            .map(|s| format!("{target}#{s}"))
            .collect(),
        _ => finding.suggestions.clone(),
    }
}

fn human(findings: &[Finding], degraded: Degraded, colour: Colour) -> String {
    let mut out = String::new();

    for finding in findings {
        let claim = &finding.claim;
        let head = format!("{}:{}:{}", claim.file.display(), claim.line, claim.column);
        // The tier is the word the eye should land on, and rot fails the build by
        // default, so it is the one that reads as an error.
        let tier_code = match finding.tier {
            Tier::Rot => "31;1",
            Tier::Lie => "33",
        };
        out.push_str(&format!(
            "{}  {}  {}\n",
            colour.paint("36", &head),
            colour.paint(tier_code, finding.tier.slug()),
            finding.message
        ));
        // The width is measured on the unpainted text: escape codes occupy no columns,
        // and counting them would push every continuation line out of alignment.
        let indent = " ".repeat(head.chars().count() + 2 + finding.tier.slug().len() + 2);
        if let Some(commit) = &finding.breaking_commit {
            out.push_str(&format!(
                "{indent}{}\n",
                colour.paint(
                    "2",
                    &format!(
                        "likely broke in {} \"{}\" · {}",
                        commit.sha,
                        commit.subject,
                        relative_date(commit.timestamp)
                    )
                )
            ));
        }
        let suggestions = suggestions_of(finding);
        if !suggestions.is_empty() {
            out.push_str(&format!(
                "{indent}{}\n",
                colour.paint("2", &format!("did you mean: {}?", suggestions.join(", ")))
            ));
        }
    }

    if findings.is_empty() && !degraded.is_degraded() {
        out.push_str(&format!(
            "{}\n",
            colour.paint("32", "stilltrue: no findings")
        ));
    } else if !findings.is_empty() {
        let rot = findings.iter().filter(|f| f.tier == Tier::Rot).count();
        let lie = findings.len() - rot;
        out.push_str(&format!(
            "\n{}\n",
            colour.paint("1", &format!("stilltrue: {rot} rot, {lie} lie"))
        ));
    }
    out
}

/// One finding as `--format json` writes it. The run report reuses it, so the two can
/// never disagree about what a finding says.
pub fn finding_json(f: &Finding) -> serde_json::Value {
    serde_json::json!({
        "ruleId": f.rule_id,
        "tier": f.tier.slug(),
        "claim": f.claim.text,
        "kind": f.claim.kind.slug(),
        "file": f.claim.file.to_string_lossy(),
        "line": f.claim.line,
        "column": f.claim.column,
        "span": {"start": f.claim.span.start, "end": f.claim.span.end},
        "message": f.message,
        "likelyBrokeIn": f.breaking_commit.as_ref().map(|c| serde_json::json!({
            "sha": c.sha,
            "subject": c.subject,
            "relativeDate": relative_date(c.timestamp),
        })),
        "suggestions": suggestions_of(f),
    })
}

fn json(findings: &[Finding]) -> String {
    let results: Vec<serde_json::Value> = findings.iter().map(finding_json).collect();
    let mut out = serde_json::to_string_pretty(&serde_json::json!({"findings": results}))
        .unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

fn sarif(findings: &[Finding]) -> String {
    let mut declared: BTreeMap<&str, &Finding> = BTreeMap::new();
    for finding in findings {
        declared.entry(finding.rule_id.as_str()).or_insert(finding);
    }
    let rules: Vec<serde_json::Value> = declared
        .iter()
        .map(|(id, f)| {
            serde_json::json!({
                "id": id,
                "name": rule_name(f),
                "shortDescription": {"text": rule_description(f)},
                "defaultConfiguration": {"level": level_of(f.tier)},
                "properties": {"tags": ["documentation", f.tier.slug()]},
            })
        })
        .collect();

    // The same fingerprints a baseline records (ADR-0014): stable under reflow because
    // no line number goes in, and unique per occurrence because otherwise code scanning
    // treats the second of two identical claims as a duplicate and drops it.
    let fingerprints = crate::baseline::fingerprints(findings);

    let results: Vec<serde_json::Value> = findings
        .iter()
        .zip(fingerprints)
        .map(|(f, fingerprint)| {
            let uri = uri_for(&f.claim.file);

            let message = match &f.breaking_commit {
                // The breaking commit is the whole Tier A argument; a reviewer reading
                // the annotation needs it as much as one reading the terminal.
                Some(c) => format!(
                    "{} — likely broke in {} \"{}\"",
                    f.message, c.sha, c.subject
                ),
                None => f.message.clone(),
            };

            let mut result = serde_json::json!({
                "ruleId": f.rule_id,
                // Tier A fails the build by default, so it is an error; Tier B is not.
                "level": level_of(f.tier),
                "message": {"text": message},
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {"uri": uri, "uriBaseId": "%SRCROOT%"},
                        "region": {
                            "startLine": f.claim.line,
                            "startColumn": f.claim.column,
                            "endLine": f.claim.end_line,
                            "endColumn": f.claim.end_column,
                        },
                    }
                }],
                "partialFingerprints": {"stilltrue/v1": fingerprint},
            });
            if !f.suggestions.is_empty() {
                result["properties"] = serde_json::json!({"suggestions": f.suggestions});
            }
            result
        })
        .collect();

    let mut out = serde_json::to_string_pretty(&serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "stilltrue",
                "version": env!("CARGO_PKG_VERSION"),
                "semanticVersion": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/uk0064/stilltrue",
                "rules": rules,
            }},
            // Required whenever a run over text artifacts has results (SARIF §3.14.27),
            // and `extract` counts columns in code points.
            "columnKind": "unicodeCodePoints",
            // Without this, two uploads against one commit overwrite each other.
            "automationDetails": {"id": "stilltrue"},
            "originalUriBaseIds": {"%SRCROOT%": {"description": {
                "text": "The repository root, as discovered by git."
            }}},
            "results": results,
        }]
    }))
    .unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

fn level_of(tier: Tier) -> &'static str {
    match tier {
        Tier::Rot => "error",
        Tier::Lie => "warning",
    }
}

/// An end-user-understandable identifier (SARIF §3.49.5), rather than a second copy of
/// the opaque id.
fn rule_name(f: &Finding) -> String {
    let capitalise = |s: &str| {
        let mut chars = s.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    };
    format!(
        "{}{}",
        capitalise(f.claim.kind.slug()),
        capitalise(f.tier.slug())
    )
}

fn rule_description(f: &Finding) -> String {
    let subject = match f.claim.kind.slug() {
        "path" => "A path named in a document",
        "command" => "A command named in a document",
        "env" => "An environment variable named in a document",
        "symbol" => "A symbol named in a document",
        "version" => "A tool version stated in a document",
        "link" => "A link target in a document",
        _ => "A claim made in a document",
    };
    match f.tier {
        Tier::Rot => format!("{subject} no longer resolves, and history shows it once did."),
        Tier::Lie => format!("{subject} does not resolve, and there is no sign it ever did."),
    }
}

/// A repository-relative URI, percent-encoded per RFC 3986.
///
/// SARIF §3.4.3 requires a relative-path reference, and a raw path is not one: a space
/// is illegal and a `#` is read as the start of a fragment, so `doc #1.md` arrives at a
/// consumer as `doc ` with fragment `1.md`. Walking components also normalises the
/// separator, which matters on Windows.
fn uri_for(path: &Path) -> String {
    path.components()
        .map(|c| encode_segment(&c.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
