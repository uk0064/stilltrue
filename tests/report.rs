//! Seam: rendering, for the formats whose correctness is about escaping rather than
//! about the gate.

use std::path::Path;

use stilltrue::claim::{Claim, ClaimKind};
use stilltrue::gate::{Finding, Tier};
use stilltrue::report::{Colour, Degraded, Format, render, render_with};

fn finding(file: &str, message: &str, tier: Tier) -> Finding {
    Finding {
        claim: Claim {
            kind: ClaimKind::Path,
            text: "docs/ghost.md".into(),
            file: Path::new(file).to_path_buf(),
            line: 3,
            column: 5,
            end_line: 3,
            end_column: 18,
            span: 0..13,
        },
        tier,
        rule_id: "stilltrue/path/rot".into(),
        message: message.into(),
        breaking_commit: None,
        suggestions: Vec::new(),
    }
}

fn github(findings: &[Finding]) -> String {
    render(findings, Degraded::No, Format::Github)
}

#[test]
fn an_annotation_names_the_file_line_and_span() {
    let out = github(&[finding(
        "CLAUDE.md",
        "path `docs/ghost.md` does not exist",
        Tier::Rot,
    )]);
    assert_eq!(
        out.trim_end(),
        "::error file=CLAUDE.md,line=3,col=5,endLine=3,endColumn=18::path `docs/ghost.md` does not exist"
    );
}

#[test]
fn a_lie_is_a_warning_not_an_error() {
    let out = github(&[finding("CLAUDE.md", "nope", Tier::Lie)]);
    assert!(out.starts_with("::warning "), "got {out}");
}

#[test]
fn a_comma_or_colon_in_a_path_is_escaped_as_a_property() {
    // GitHub splits command properties on `,` and terminates the name on `:`. A raw
    // path with either lands the annotation on the wrong file, or drops it.
    let out = github(&[finding("docs/a,b: notes.md", "nope", Tier::Rot)]);
    assert!(out.contains("file=docs/a%2Cb%3A notes.md"), "got {out}");
    assert!(!out.contains("file=docs/a,b"), "unescaped comma: {out}");
    // And only those: the runner decodes five escapes, so a space must stay a space.
    assert!(!out.contains("%20"), "over-escaped: {out}");
}

#[test]
fn a_newline_or_percent_in_a_message_is_escaped_as_data() {
    // A raw newline would end the workflow command and let the remainder of a message
    // be read as a new one.
    let out = github(&[finding(
        "CLAUDE.md",
        "first\nsecond 100% done\rx",
        Tier::Rot,
    )]);
    assert!(out.contains("first%0Asecond 100%25 done%0Dx"), "got {out}");
    assert_eq!(out.lines().count(), 1, "one annotation, one line: {out}");
}

#[test]
fn no_findings_renders_nothing_at_all() {
    // The Action reads this on stdout; a "no findings" line would become an annotation.
    assert_eq!(github(&[]), "");
}

// ---------------------------------------------------------------------------------
// Relative dates. Every finding carries one, and every boundary below is one second
// wide — `4 months ago` against `17 weeks ago` is the difference between a reader
// believing the blame and going to check it.
// ---------------------------------------------------------------------------------

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const MONTH: i64 = 2_629_800;
const YEAR: i64 = 12 * MONTH;

/// `ago(n)` is what a commit `n` seconds old renders as.
fn ago(seconds: i64) -> String {
    stilltrue::report::relative_to(0, seconds)
}

#[test]
fn each_unit_gives_way_to_the_next_at_its_own_boundary() {
    // One second either side of every threshold in the ladder.
    for (seconds, expected) in [
        (89, "89 seconds ago"),
        (90, "2 minutes ago"),
        (90 * MINUTE - 1, "90 minutes ago"),
        (90 * MINUTE, "2 hours ago"),
        (36 * HOUR - 1, "36 hours ago"),
        (36 * HOUR, "2 days ago"),
        (14 * DAY - 1, "14 days ago"),
        (14 * DAY, "2 weeks ago"),
        (10 * 7 * DAY - 1, "10 weeks ago"),
        (10 * 7 * DAY, "2 months ago"),
        (YEAR - 1, "12 months ago"),
        (YEAR, "1 year ago"),
    ] {
        assert_eq!(ago(seconds), expected, "at {seconds}s");
    }
}

#[test]
fn a_relative_date_rounds_to_the_nearest_unit_rather_than_truncating() {
    // Git rounds; truncating would call a commit from 59 days ago "1 month".
    assert_eq!(ago(3 * DAY), "3 days ago");
    assert_eq!(
        ago(3 * DAY + HOUR * 13),
        "4 days ago",
        "rounds up past half"
    );
    assert_eq!(ago(4 * MONTH), "4 months ago");
    assert_eq!(ago(4 * MONTH + MONTH / 2 + 1), "5 months ago");
    assert_eq!(ago(2 * YEAR), "2 years ago");

    // Weeks had the same gap the years note below describes, and nothing caught it:
    // every week case in this file sits on an exact multiple or one second under the
    // top of the bucket, and at those points rounding to nearest and truncating agree.
    // Eighteen days is two and four-sevenths of a week, which is the only kind of
    // input the half-week offset can be read from.
    assert_eq!(ago(18 * DAY), "3 weeks ago", "2.6 weeks rounds up");
    assert_eq!(ago(16 * DAY), "2 weeks ago", "2.3 weeks rounds down");

    // Years round too, and an exact multiple cannot show it: at one and a half years
    // the two behaviours agree by coincidence, so the case has to sit past the half.
    assert_eq!(ago(YEAR * 8 / 5), "2 years ago", "1.6 years rounds up");
    assert_eq!(ago(YEAR * 7 / 5), "1 year ago", "1.4 years rounds down");
}

#[test]
fn the_singular_is_used_where_it_is_reachable_at_all() {
    assert_eq!(ago(1), "1 second ago");
    assert_eq!(ago(0), "1 second ago", "never `0 seconds ago`");
    assert_eq!(ago(YEAR), "1 year ago");
}

#[test]
fn the_middle_units_never_render_a_singular() {
    // Git's ladder, reproduced: each bucket opens well past its own unit and then
    // rounds to nearest, so the smallest minute value is (90+30)/60 = 2 and the
    // smallest week is (14 days + half a week)/week = 2. `1 minute ago` is not a
    // string this tool can produce, and a reader will never see one.
    assert_eq!(
        ago(MINUTE),
        "60 seconds ago",
        "still inside the seconds bucket"
    );
    for (opens_at, unit) in [
        (90, "minute"),
        (90 * MINUTE, "hour"),
        (36 * HOUR, "day"),
        (14 * DAY, "week"),
        (10 * 7 * DAY, "month"),
    ] {
        assert_eq!(
            ago(opens_at),
            format!("2 {unit}s ago"),
            "the {unit} bucket opens at 2, not 1"
        );
        assert_ne!(ago(opens_at), format!("1 {unit} ago"));
    }
    assert_eq!(ago(3 * 7 * DAY), "3 weeks ago");
}

#[test]
fn a_commit_from_the_future_is_not_negative() {
    // Clock skew between a CI runner and a committer's laptop is ordinary, and
    // `-3 seconds ago` would look like a bug in the tool rather than in the clock.
    assert_eq!(stilltrue::report::relative_to(1000, 0), "1 second ago");
}

// ---------------------------------------------------------------------------------
// SARIF carries a rule object per rule id, and code scanning shows its description as
// the explanation beside every alert. One per claim type, and the wrong one is worse
// than none: it tells the reader the alert is about something it is not.
// ---------------------------------------------------------------------------------

fn of_kind(kind: ClaimKind, text: &str, rule: &str) -> Finding {
    let mut f = finding("README.md", "whatever", Tier::Rot);
    f.claim.kind = kind;
    f.claim.text = text.into();
    f.rule_id = rule.into();
    f
}

#[test]
fn every_claim_type_carries_its_own_sarif_rule() {
    let findings = vec![
        of_kind(ClaimKind::Path, "docs/ghost.md", "stilltrue/path/rot"),
        of_kind(
            ClaimKind::Command {
                runner: "make".into(),
                args: vec!["demo".into()],
            },
            "make demo",
            "stilltrue/command/rot",
        ),
        of_kind(ClaimKind::EnvVar, "API_KEY", "stilltrue/env/rot"),
        of_kind(ClaimKind::Symbol, "fold_events()", "stilltrue/symbol/rot"),
        of_kind(
            ClaimKind::Version {
                tool: "Rust".into(),
                version: "1.98".into(),
            },
            "Rust 1.98",
            "stilltrue/version/rot",
        ),
        of_kind(
            ClaimKind::Link {
                target: "setup.md".into(),
                anchor: None,
            },
            "./setup.md",
            "stilltrue/link/rot",
        ),
    ];

    let out = render(&findings, Degraded::No, Format::Sarif);
    let doc: serde_json::Value = serde_json::from_str(&out).expect("valid SARIF");
    let rules = doc["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .expect("rules");

    // Every rule id present, exactly once.
    let ids: Vec<&str> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
    for expected in [
        "stilltrue/path/rot",
        "stilltrue/command/rot",
        "stilltrue/env/rot",
        "stilltrue/symbol/rot",
        "stilltrue/version/rot",
        "stilltrue/link/rot",
    ] {
        assert_eq!(
            ids.iter().filter(|id| **id == expected).count(),
            1,
            "{expected} should appear exactly once in {ids:?}"
        );
    }

    // And each description names the thing that rule is actually about. A deleted or
    // duplicated arm shows up here as two rules sharing one sentence.
    let described = |id: &str| -> String {
        rules
            .iter()
            .find(|r| r["id"] == id)
            .and_then(|r| r["shortDescription"]["text"].as_str())
            .unwrap_or_default()
            .to_string()
    };
    for (id, expected) in [
        ("stilltrue/path/rot", "A path named in a document"),
        ("stilltrue/command/rot", "A command named in a document"),
        (
            "stilltrue/env/rot",
            "An environment variable named in a document",
        ),
        ("stilltrue/symbol/rot", "A symbol named in a document"),
        (
            "stilltrue/version/rot",
            "A tool version stated in a document",
        ),
        ("stilltrue/link/rot", "A link"),
    ] {
        assert!(
            described(id).starts_with(expected),
            "{id}: expected a description starting {expected:?}, got {:?}",
            described(id)
        );
    }

    let descriptions: std::collections::BTreeSet<String> =
        ids.iter().map(|id| described(id)).collect();
    assert_eq!(
        descriptions.len(),
        ids.len(),
        "each rule needs its own sentence: {descriptions:?}"
    );
}

// ---------------------------------------------------------------------------------
// Colour. A linter is read in a terminal, and the tier is the word the eye should
// land on. None of it may reach a pipe, a file, or a machine format.
// ---------------------------------------------------------------------------------

fn rot_and_lie() -> Vec<Finding> {
    vec![
        finding("README.md", "command `make demo` has no target", Tier::Rot),
        finding(
            "CLAUDE.md",
            "path `docs/ghost.md` does not exist",
            Tier::Lie,
        ),
    ]
}

#[test]
fn human_output_is_plain_unless_colour_is_asked_for() {
    let plain = render_with(&rot_and_lie(), Degraded::No, Format::Human, Colour::Never);
    assert!(
        !plain.contains('\u{1b}'),
        "no escape codes by default: {plain:?}"
    );
    assert_eq!(
        plain,
        render(&rot_and_lie(), Degraded::No, Format::Human),
        "the uncoloured path is what `render` has always produced"
    );
}

#[test]
fn colour_marks_the_tier_and_leaves_the_words_alone() {
    let painted = render_with(&rot_and_lie(), Degraded::No, Format::Human, Colour::Always);
    assert!(painted.contains("\u{1b}[31;1mrot\u{1b}[0m"), "{painted:?}");
    assert!(painted.contains("\u{1b}[33mlie\u{1b}[0m"), "{painted:?}");
    assert!(
        painted.contains("\u{1b}[36mREADME.md:3:5\u{1b}[0m"),
        "{painted:?}"
    );

    // Stripping every escape sequence gives back the plain rendering exactly. Colour
    // is a coat of paint, never a different report.
    let stripped = strip_ansi(&painted);
    let plain = render_with(&rot_and_lie(), Degraded::No, Format::Human, Colour::Never);
    assert_eq!(
        stripped, plain,
        "colour changed the text, not just its colour"
    );
}

#[test]
fn colour_does_not_disturb_the_alignment_of_a_continuation_line() {
    // The indent is measured on the unpainted head: escape codes occupy no columns, so
    // counting them would push every "likely broke in" line out of line with the one
    // above it. The stripped output being identical is what proves it.
    let mut f = finding("README.md", "command `make demo` has no target", Tier::Rot);
    f.suggestions = vec!["make seed".into(), "make serve".into()];
    let painted = strip_ansi(&render_with(
        std::slice::from_ref(&f),
        Degraded::No,
        Format::Human,
        Colour::Always,
    ));
    let plain = render_with(
        std::slice::from_ref(&f),
        Degraded::No,
        Format::Human,
        Colour::Never,
    );
    assert_eq!(painted, plain);

    let lines: Vec<&str> = plain.lines().collect();
    let head_width = lines[0].find("  rot  ").expect("a tier on the first line") + 7;
    assert_eq!(
        lines[1].len() - lines[1].trim_start().len(),
        head_width,
        "the suggestion lines up under the message"
    );
}

#[test]
fn no_machine_format_is_ever_coloured() {
    // A colour code in SARIF is a corrupt document, not a prettier one.
    for format in [Format::Sarif, Format::Json, Format::Github] {
        let out = render_with(&rot_and_lie(), Degraded::No, format, Colour::Always);
        assert!(
            !out.contains('\u{1b}'),
            "{format:?} must never carry escape codes: {out:?}"
        );
    }
}

#[test]
fn a_clean_run_says_so_in_green() {
    let painted = render_with(&[], Degraded::No, Format::Human, Colour::Always);
    assert!(
        painted.contains("\u{1b}[32mstilltrue: no findings\u{1b}[0m"),
        "{painted:?}"
    );
}

/// Remove every ANSI SGR sequence, so the text underneath can be compared.
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn colour_is_for_terminals_and_no_color_wins() {
    use stilltrue::report::Colour;

    // Both signals, all four ways round. `NO_COLOR` is honoured whatever its value
    // (no-color.org: presence is the signal), and a pipe is never coloured — which is
    // what keeps `stilltrue > out.txt` and `stilltrue | grep` readable with nothing
    // configured.
    assert_eq!(
        Colour::from_signals(false, true),
        Colour::Always,
        "a terminal"
    );
    assert_eq!(
        Colour::from_signals(false, false),
        Colour::Never,
        "a pipe or a file",
    );
    assert_eq!(
        Colour::from_signals(true, true),
        Colour::Never,
        "NO_COLOR overrides a terminal",
    );
    assert_eq!(
        Colour::from_signals(true, false),
        Colour::Never,
        "and agrees with a pipe",
    );
}

#[test]
fn machine_formats_are_never_coloured_whatever_the_terminal_says() {
    // A colour code in SARIF is a corrupt document, not a prettier one. `Always` is
    // passed deliberately here: the guard has to be in the renderer, not in the
    // caller's judgement about where output is going.
    let findings = [finding(
        "README.md",
        "path `docs/x.md` does not exist",
        Tier::Rot,
    )];
    for format in [Format::Json, Format::Sarif, Format::Github] {
        let out = stilltrue::report::render_with(
            &findings,
            Degraded::No,
            format,
            stilltrue::report::Colour::Always,
        );
        assert!(
            !out.contains('\x1b'),
            "{format:?} carried an escape code: {out:?}",
        );
    }
}
