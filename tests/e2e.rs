//! Seam: the CLI, end to end, over the fixture matrix.
//!
//! Every axis ships a pair: the finding it must report, and the near-miss it must stay
//! silent on. The near-miss fixtures are the regression suite that protects precision.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Build a fixture repository into a fresh temporary directory.
fn build(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("repo");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .join("build.sh");
    let output = Command::new("bash")
        .arg(&script)
        .arg(&target)
        .output()
        .expect("run build.sh");
    assert!(
        output.status.success(),
        "{name}/build.sh failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    dir
}

/// Run the real binary in `repo`, returning rendered output and exit code.
fn stilltrue(repo: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_stilltrue"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run stilltrue");
    format!(
        "$ stilltrue {}\n{}{}\nexit: {}\n",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
        output.status.code().unwrap_or(-1)
    )
}

/// Commit shas and relative dates are not stable; everything else is.
macro_rules! snapshot {
    ($name:expr, $body:expr) => {
        insta::with_settings!({
            filters => vec![
                (r"\b[0-9a-f]{7,40}\b", "<sha>"),
                (r"· [^\n]*ago", "· <when>"),
                (r#"/private/var/folders[^\s"]*"#, "<tmp>"),
                (r#"/var/folders[^\s"]*"#, "<tmp>"),
            ],
            snapshot_suffix => "",
        }, {
            insta::assert_snapshot!($name, $body);
        });
    };
}

fn run(fixture: &str, args: &[&str]) -> String {
    let dir = build(fixture);
    stilltrue(&dir.path().join("repo"), args)
}

#[test]
fn command_rot_is_reported_with_the_commit_that_likely_broke_it() {
    snapshot!("command-rot", run("command-rot", &[]));
}

#[test]
fn a_removed_cargo_alias_is_rot_and_not_a_lie() {
    // The tier is the assertion. A cargo alias is `xtask = "..."`, so its needle ends
    // in `=` where every other runner's ends in `:`. With the wrong needle the pickaxe
    // finds nothing, history reports the alias never existed, and the finding degrades
    // from rot to a lie — reported only under `--strict`, and so invisible in the run
    // that matters. Both runs are captured to hold that distinction in place.
    snapshot!("cargo-alias-rot", run("cargo-alias-rot", &[]));
    snapshot!(
        "cargo-alias-rot-strict",
        run("cargo-alias-rot", &["--strict"])
    );
}

#[test]
fn a_manifest_that_stopped_parsing_stays_silent() {
    // The pair of `command-rot`. Before ADR-0019 this reported `npm run build` as rot,
    // blamed on the commit that introduced the script — the manifest's history proved
    // it had existed, and the unparseable manifest proved nothing at all.
    snapshot!(
        "manifest-unparseable",
        run("manifest-unparseable", &["--strict"])
    );
}

#[test]
fn a_rust_edition_stays_silent_beside_a_rust_version_that_rotted() {
    // ADR-0024: one rot for the version the pin left behind, nothing for the edition.
    snapshot!("version-edition", run("version-edition", &["--strict"]));
}

#[test]
fn a_worktree_inside_the_checkout_is_another_repository() {
    // ADR-0025: the rename's rot is reported, and the worktree's documents are not read.
    snapshot!(
        "nested-worktree",
        run("nested-worktree", &["--strict", "--summary"])
    );
}

#[test]
fn a_target_that_survived_a_refactor_stays_silent() {
    snapshot!("command-near-miss", run("command-near-miss", &["--strict"]));
}

#[test]
fn a_path_that_never_existed_is_silent_by_default_and_a_lie_under_strict() {
    snapshot!("path-lie-default", run("path-lie", &[]));
    snapshot!("path-lie-strict", run("path-lie", &["--strict"]));
}

#[test]
fn the_nested_manifest_anchors_the_nested_document() {
    snapshot!(
        "anchoring-monorepo",
        run("anchoring-monorepo", &["--strict"])
    );
}

#[test]
fn a_toml_fence_stays_silent_while_a_bash_fence_speaks() {
    snapshot!("fence-near-miss", run("fence-near-miss", &["--strict"]));
}

#[test]
fn a_placeholder_stays_silent_beside_a_real_path_that_rotted() {
    // The Placeholders axis: `packages/<name>/index.ts` is documentation of a
    // shape, not a claim about a file. The real sibling path proves the document is
    // being read at all, so silence here cannot be silence by accident.
    snapshot!(
        "placeholder-vs-real-path",
        run("placeholder-vs-real-path", &["--strict"])
    );
}

#[test]
fn a_bare_manifest_name_is_ambiguous_while_a_qualified_one_is_not() {
    // ADR-0011, and the Ambiguity axis. Both paths rotted in the same commit; only
    // the one that says which repository it means is reported.
    snapshot!(
        "ambiguous-manifest",
        run("ambiguous-manifest", &["--strict"])
    );
}

#[test]
fn a_changelog_is_a_record_of_the_past_and_is_not_linted() {
    // ADR-0012: 34% of the surviving corpus findings sat in changelogs and release
    // notes, where naming something that has since moved is the point of the document.
    snapshot!("historical-record", run("historical-record", &["--strict"]));
}

#[test]
fn a_cut_history_trail_degrades_to_silence_rather_than_mislabelling() {
    snapshot!("history-cut-rename", run("history-cut-rename", &[]));
}

#[test]
fn a_shallow_clone_says_so_loudly_and_reports_nothing() {
    snapshot!("degraded-shallow", run("degraded-shallow", &[]));
}

#[test]
fn a_partial_clone_is_degraded_even_though_it_is_not_shallow() {
    // `--filter=blob:none` keeps every commit, so `--is-shallow-repository` says false
    // while the pickaxe reads an incomplete tree and answers "never existed".
    snapshot!("degraded-partial", run("degraded-partial", &[]));
}

#[test]
fn a_shallow_clone_can_be_made_fatal() {
    snapshot!(
        "degraded-require-history",
        run("degraded-shallow", &["--require-history"])
    );
}

#[test]
fn a_renamed_python_definition_is_rot() {
    // The reporting half of the Symbols axis; `symbol-mixed-language` is the
    // silent half, and until now the pair existed only as unit tests.
    snapshot!("symbol-rot", run("symbol-rot", &[]));
}

#[test]
fn a_renamed_typescript_definition_is_rot() {
    // The second language, end to end. The `LanguageResolver` trait existed from v1 to
    // prove this seam; this is the seam being used.
    snapshot!("symbol-typescript", run("symbol-typescript", &[]));
}

#[test]
fn a_symbol_is_silent_when_an_unsupported_language_is_present() {
    snapshot!(
        "symbol-mixed-language",
        run("symbol-mixed-language", &["--strict"])
    );
}

#[test]
fn an_env_var_named_only_in_documents_does_not_satisfy_itself() {
    snapshot!("envvar-doc-only", run("envvar-doc-only", &[]));
}

#[test]
fn a_pin_contradicts_prose_while_a_range_says_nothing() {
    snapshot!("version-pin-vs-range", run("version-pin-vs-range", &[]));
}

#[test]
fn config_excludes_subtract_and_the_defaults_still_apply() {
    // `exclude` subtracts from what `include` produced, and ADR-0012's historical
    // records stay excluded alongside it rather than being replaced by it.
    snapshot!("configured-exclude", run("configured-exclude", &[]));
}

#[test]
fn a_configured_include_replaces_the_defaults() {
    // The documented footgun, pinned as behaviour: naming `docs/**` stops linting
    // CLAUDE.md. If this snapshot ever shows CLAUDE.md, `include` started merging.
    snapshot!(
        "configured-include-replaces",
        run("configured-include-replaces", &[])
    );
}

#[test]
fn positional_paths_filter_the_configured_set_rather_than_replacing_it() {
    // `stilltrue README.md` still honours excludes.
    snapshot!(
        "configured-positional",
        run("configured-exclude", &["docs/legacy/old.md"])
    );
}

#[test]
fn an_unused_suppression_underlines_the_marker_itself() {
    // The human format shows a line and a column; the span that says how much of the
    // line the finding covers only reaches a reader through SARIF's endColumn and the
    // Action's annotation. For an unused marker there is no claim to measure, so the
    // span is the marker's own text — and nothing was checking it.
    snapshot!(
        "suppression-unused-github",
        run("suppression-unused", &["--strict", "--format", "github"])
    );
}

#[test]
fn being_matched_by_a_glob_is_not_the_same_as_being_prose() {
    // The default `docs/**` matches whatever is under docs/. Only Markdown and
    // reStructuredText are extracted from: a docstring, a shell comment and a JSON
    // string each contain the same backticked command here, and reading any of them
    // would invent a claim nobody made.
    snapshot!(
        "document-extensions",
        run("document-extensions", &["--strict"])
    );
}

#[test]
fn the_ignore_table_names_a_symbol_by_its_name() {
    // `[ignore]`: a symbol is named by its name, not by the spelling a document
    // happened to use: `fold_events` covers `fold_events()`, and `params` covers the
    // `Context.params` that Sphinx writes. Matching raw claim text instead would make
    // the documented example — a bare `LegacyThing` — impossible to write, because
    // ADR-0017 means a bare name is never a symbol claim in the first place.
    snapshot!("configured-ignore", run("configured-ignore", &["--strict"]));
}

#[test]
fn fail_on_decides_the_exit_code_without_changing_the_findings() {
    // `--strict` and `--fail-on` are independent. Same findings, three exit codes.
    snapshot!("fail-on-none", run("command-rot", &["--fail-on", "none"]));
    snapshot!("fail-on-any", run("command-rot", &["--fail-on", "any"]));
    snapshot!(
        "fail-on-any-lie",
        run("path-lie", &["--strict", "--fail-on", "any"])
    );
}

#[test]
fn disabling_the_cache_changes_nothing_a_user_can_see() {
    snapshot!("no-cache", run("command-rot", &["--no-cache"]));
}

#[test]
fn machine_formats_obey_the_same_gate_as_human_output() {
    // ADR-0008: `--strict` means the same thing everywhere.
    snapshot!("sarif-default", run("path-lie", &["--format", "sarif"]));
    snapshot!("json-rot", run("command-rot", &["--format", "json"]));
    // The empty case above is not enough: it was the only SARIF snapshot, and with no
    // results there is nothing to get wrong. This one carries a result, a breaking
    // commit, suggestions, and a declared rule.
    snapshot!("sarif-rot", run("command-rot", &["--format", "sarif"]));
    snapshot!(
        "sarif-lie",
        run("path-lie", &["--strict", "--format", "sarif"])
    );
}

#[test]
fn a_positional_path_is_understood_relative_to_the_working_directory() {
    // A silent false negative is the worst possible failure for this tool: a CI step
    // passing `$GITHUB_WORKSPACE/README.md` must not run green over rot.
    let dir = build("paths-from-subdirectory");
    let repo = dir.path().join("repo");

    let from_root = stilltrue(&repo, &["--strict", "docs/guide"]);
    let from_subdir = stilltrue(&repo.join("docs"), &["--strict", "guide"]);
    let absolute = stilltrue(
        &repo,
        &[
            "--strict",
            repo.join("docs/guide/README.md").to_str().unwrap(),
        ],
    );

    for (label, output) in [
        ("from the root", &from_root),
        ("from a subdirectory", &from_subdir),
        ("as an absolute path", &absolute),
    ] {
        assert!(
            output.contains("never-existed"),
            "{label}: expected the finding, got:\n{output}"
        );
    }

    snapshot!(
        "paths-outside-the-repository",
        stilltrue(&repo, &["--strict", "/etc"])
    );
}

#[test]
fn a_baseline_silences_what_it_records_and_still_reports_what_it_does_not() {
    // The adoption story (ADR-0014): a repository with a backlog records it once and
    // then only hears about new rot.
    let dir = build("configured-exclude");
    let repo = dir.path().join("repo");

    let before = stilltrue(&repo, &[]);
    assert!(before.contains("2 rot"), "{before}");

    let written = stilltrue(&repo, &["--write-baseline", "baseline.txt"]);
    assert!(written.contains("recorded 2 findings"), "{written}");
    assert!(written.ends_with("exit: 0\n"), "{written}");

    let after = stilltrue(&repo, &["--baseline", "baseline.txt"]);
    assert!(after.contains("no findings"), "{after}");
    assert!(after.ends_with("exit: 0\n"), "{after}");

    // A claim added afterwards is not in the baseline, so it is still reported.
    std::fs::write(repo.join("docs/new.md"), "Run `make demo` to start.\n").unwrap();
    let fresh = stilltrue(&repo, &["--baseline", "baseline.txt"]);
    assert!(fresh.contains("docs/new.md"), "{fresh}");
    assert!(fresh.contains("1 rot"), "{fresh}");
}

#[test]
fn a_baseline_survives_a_reflow_of_the_document() {
    // The property the fingerprint exists for: no line number goes in, so rewrapping a
    // paragraph must not un-suppress it.
    let dir = build("configured-exclude");
    let repo = dir.path().join("repo");
    stilltrue(&repo, &["--write-baseline", "baseline.txt"]);

    let guide = repo.join("docs/guide.md");
    let original = std::fs::read_to_string(&guide).unwrap();
    std::fs::write(&guide, format!("# Heading\n\nAdded prose.\n\n{original}")).unwrap();

    let after = stilltrue(&repo, &["--baseline", "baseline.txt"]);
    assert!(
        after.contains("no findings"),
        "reflow un-suppressed it:\n{after}"
    );
}

#[test]
fn a_missing_baseline_judges_everything_rather_than_nothing() {
    // The safe direction: a typo in the path must not silence the run.
    let out = run("command-rot", &["--baseline", "does-not-exist.txt"]);
    assert!(out.contains("could not read baseline"), "{out}");
    assert!(out.contains("1 rot"), "{out}");
}

#[test]
fn fix_rewrites_a_claim_with_exactly_one_candidate() {
    // ADR-0015: one candidate is the only case where enumeration and certainty are the
    // same thing. `configured-exclude` has a single surviving target, `seed`.
    let dir = build("configured-exclude");
    let repo = dir.path().join("repo");

    let out = stilltrue(&repo, &["--fix"]);
    assert!(out.contains("`make demo` -> `make seed`"), "{out}");
    assert!(out.contains("rewrote 2 of 2 findings"), "{out}");
    // Exits on what it found, so CI cannot go green by editing the repository.
    assert!(out.ends_with("exit: 1\n"), "{out}");

    let guide = std::fs::read_to_string(repo.join("docs/guide.md")).unwrap();
    assert!(guide.contains("`make seed`"), "{guide}");

    // And the rewrite is real: a second run is clean.
    let after = stilltrue(&repo, &[]);
    assert!(after.contains("no findings"), "{after}");
}

#[test]
fn fix_leaves_a_claim_that_could_mean_two_things_alone() {
    // command-rot's Makefile keeps `seed` and `serve`, so the finding has two
    // candidates and choosing between them is a human's job.
    let dir = build("command-rot");
    let repo = dir.path().join("repo");
    let before = std::fs::read_to_string(repo.join("CLAUDE.md")).unwrap();

    let out = stilltrue(&repo, &["--fix"]);
    assert!(out.contains("rewrote 0 of 1 findings"), "{out}");
    assert_eq!(
        std::fs::read_to_string(repo.join("CLAUDE.md")).unwrap(),
        before,
        "an ambiguous finding was rewritten"
    );
}

#[test]
fn fix_does_not_rewrite_a_document_that_moved_under_it() {
    // The span is where the claim was when it was read; if the bytes there are not the
    // claim any more, something else edited the file and it is not ours to rewrite.
    let dir = build("configured-exclude");
    let repo = dir.path().join("repo");
    std::fs::write(repo.join("docs/guide.md"), "Totally different content.\n").unwrap();

    let out = stilltrue(&repo, &["--fix"]);
    assert!(
        out.contains("moved under us") || out.contains("rewrote 1 of"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("docs/guide.md")).unwrap(),
        "Totally different content.\n"
    );
}

/// The same run, but stdout alone — which is what a `--format json` consumer reads.
fn stdout_of(repo: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_stilltrue"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run stilltrue");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn fix_rewrites_every_claim_type_into_something_that_resolves() {
    // `--fix` was tested only on commands, and every other claim type wrote text that
    // did not resolve. The property that catches all of them at once is the one a user
    // assumes: fix it, run again, and it is clean.
    let dir = build("fix-every-claim-type");
    let repo = dir.path().join("repo");

    let out = stilltrue(&repo, &["--fix"]);
    assert!(out.contains("rewrote 3 of 3 findings"), "{out}");

    let readme = std::fs::read_to_string(repo.join("README.md")).unwrap();
    // The tool name survives: `Node 18` becomes `Node 24`, never a bare `24`.
    assert!(readme.contains("Needs Node 24 to build."), "{readme}");
    // The destination survives: only the fragment moves.
    assert!(
        readme.contains("[the guide](docs/guide.md#setup)"),
        "{readme}"
    );
    // And a link is respelled from the document's own directory, not the root.
    let a = std::fs::read_to_string(repo.join("docs/a.md")).unwrap();
    assert!(a.contains("[x](new/x.md)"), "{a}");

    let after = stilltrue(&repo, &["--strict"]);
    assert!(after.contains("no findings"), "{after}");
}

#[test]
fn fix_leaves_stdout_parseable_by_the_consumer_that_asked_for_json() {
    // The rewrite lines went to stdout, after the report. Under `--format json` that
    // is not a trailing comment, it is a second document, and the result is neither.
    let dir = build("fix-every-claim-type");
    let repo = dir.path().join("repo");

    let out = stdout_of(&repo, &["--format", "json", "--fix"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n{out}"));
    assert_eq!(
        parsed["findings"].as_array().map(Vec::len),
        Some(3),
        "{out}"
    );
}

#[test]
fn an_unused_suppression_is_reported_only_under_strict() {
    // ADR-0016: the original design was right that this fires the moment someone fixes
    // their documentation, which is why it waits for the tier that asked for noise.
    snapshot!("suppression-unused-default", run("suppression-unused", &[]));
    snapshot!(
        "suppression-unused-strict",
        run("suppression-unused", &["--strict"])
    );
}

#[test]
fn a_restructuredtext_document_is_linted_like_any_other() {
    // One claim model and one gate; only the reader differs. The toml directive is the
    // near-miss — ADR-0002's rule holds in reStructuredText too, so only a block that
    // says it is shell is read as shell — and the suppression covers what it says.
    snapshot!("rst-document", run("rst-document", &["--strict"]));
}
