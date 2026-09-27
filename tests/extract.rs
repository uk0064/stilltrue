//! Seam: `extract` turns a Markdown document into typed claims with exact spans.

use std::path::Path;
use stilltrue::claim::ClaimKind;
use stilltrue::extract;

#[test]
fn inline_code_span_holding_a_runner_becomes_a_command_claim() {
    let doc = "Run `make demo` to see it.\n";

    let claims = extract::claims(doc, Path::new("README.md"));

    assert_eq!(
        claims.len(),
        1,
        "expected exactly one claim, got {claims:?}"
    );
    let claim = &claims[0];
    assert_eq!(claim.text, "make demo");
    assert_eq!(claim.line, 1);
    assert_eq!(
        claim.column, 6,
        "column is 1-based and points at the span text"
    );
    assert!(
        matches!(&claim.kind, ClaimKind::Command { runner, args }
            if runner == "make" && args == &["demo"]),
        "expected a make/demo Command, got {:?}",
        claim.kind
    );
}

#[test]
fn a_span_containing_a_slash_is_a_path() {
    let claims = extract::claims(
        "See `pipeline/format-bible.md` for detail.\n",
        Path::new("R.md"),
    );
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].kind, ClaimKind::Path);
    assert_eq!(claims[0].text, "pipeline/format-bible.md");
}

#[test]
fn a_span_ending_in_an_allowlisted_extension_is_a_path() {
    let claims = extract::claims("Edit `config.toml`.\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].kind, ClaimKind::Path);
}

#[test]
fn a_dotted_span_without_an_allowlisted_extension_is_not_a_path() {
    // The extension allowlist is what keeps `graph.query` a Symbol while
    // `config.toml` is a Path.
    let claims = extract::claims("Call `graph.query`.\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1);
    assert_ne!(claims[0].kind, ClaimKind::Path);
}

#[test]
fn placeholders_are_unclassified_before_any_other_rule_runs() {
    // Each of these would otherwise classify: the first two contain a slash.
    for doc in [
        "See `packages/<name>/index.ts`.\n",
        "See `path/to/file`.\n",
        "Set `YOUR_API_KEY`.\n",
        "See `docs/...`.\n",
        "Use `{{project}}`.\n",
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert!(
            claims.is_empty(),
            "{doc:?} should yield no claim, got {claims:?}"
        );
    }
}

#[test]
fn every_placeholder_clause_is_load_bearing_on_its_own() {
    // The test above uses inputs that trip several clauses at once — `<name>` has both
    // angle brackets, `{{project}}` has both brace rules — so breaking any single clause
    // leaves it passing. Mutation testing found nine survivors in `is_placeholder`
    // for exactly that reason. These inputs isolate one clause each, so each one is
    // the only thing standing between the claim and a false positive.
    for (doc, clause) in [
        ("See `docs/a<b.md`.\n", "`<` alone, no closing bracket"),
        ("See `docs/a>b.md`.\n", "`>` alone, no opening bracket"),
        // Both end in a real extension: a trailing-dot spelling like `docs/more...`
        // is suppressed by the bare-reference rule instead, which masks this clause.
        ("See `docs/a...b.md`.\n", "an ASCII ellipsis"),
        (
            "See `docs/a\u{2026}b.md`.\n",
            "a Unicode ellipsis character",
        ),
        ("Use `{{unclosed`.\n", "`{{` with no closing brace"),
        ("Use `config/{env}.toml`.\n", "a single braced group"),
        (
            "See `https://example.com/a.md`.\n",
            "an external URL is lychee's job",
        ),
        ("See `docs/*.md`.\n", "a glob star"),
        ("See `docs/a?.md`.\n", "a glob question mark"),
        ("See `docs/[ab].md`.\n", "a glob bracket group"),
        ("See `path/to/file.md`.\n", "the path/to convention"),
        ("Set `YOUR_API_KEY`.\n", "the YOUR_ prefix"),
        ("See `your-app/config.toml`.\n", "the your- prefix"),
        ("See `my-service/index.ts`.\n", "the my- prefix"),
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert!(
            claims.is_empty(),
            "{clause}: {doc:?} should yield no claim, got {claims:?}"
        );
    }
}

#[test]
fn a_placeholder_rule_stops_where_the_shape_stops() {
    // The near-misses for the rule above. Each of these is one character away from a
    // placeholder and is a real claim, so a clause widened by accident shows up here as
    // silence — which is the expensive direction for a linter that exists to speak up.
    for (doc, why) in [
        (
            "See `config/{env.toml`.\n",
            "an unpaired brace is not a braced group",
        ),
        ("See `docs/[ab.md`.\n", "an unpaired bracket is not a glob"),
        ("See `yourapp/config.toml`.\n", "`your-` needs the hyphen"),
        ("See `mystery/index.ts`.\n", "`my-` needs the hyphen"),
        ("See `paths/to.md`.\n", "`path/to` is not `paths/to`"),
        (
            "See `path/from/file.md`.\n",
            "a `path` segment alone is not the convention",
        ),
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "{why}: {doc:?} should be a claim");
    }
}

#[test]
fn no_claims_are_extracted_from_inside_a_non_shell_fence() {
    // ADR-0002. The near-miss that protects it: the identical path in prose *is* a claim.
    let fenced = "Example:\n\n```toml\ninclude = [\"docs/missing.md\"]\n```\n";
    assert!(extract::claims(fenced, Path::new("R.md")).is_empty());

    let prose = "See `docs/missing.md`.\n";
    assert_eq!(extract::claims(prose, Path::new("R.md")).len(), 1);
}

#[test]
fn a_screaming_snake_case_span_is_an_env_var() {
    let claims = extract::claims("Set `GEMINI_API_KEY` first.\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].kind, ClaimKind::EnvVar);
}

#[test]
fn identifier_shaped_spans_are_symbols() {
    // A call or a namespace: ADR-0017. Both say "this is a name in a program".
    for doc in [
        "Call `fold_events()`.\n",
        "Call `graph.query`.\n",
        "See `Repo::files`.\n",
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "{doc:?}");
        assert_eq!(claims[0].kind, ClaimKind::Symbol, "{doc:?}");
    }
}

#[test]
fn a_bare_identifier_shaped_word_is_not_a_symbol() {
    // The near-miss for ADR-0017. flask's tutorials name `download_file` and
    // `simple_page` while explaining an example; poetry names `long_description`. None
    // of them claims this repository defines that name.
    for doc in [
        "See `download_file` in the example.\n",
        "The `long_description` field.\n",
        "A `LanguageResolver` is a trait.\n",
    ] {
        assert!(
            extract::claims(doc, Path::new("R.md")).is_empty(),
            "{doc:?}"
        );
    }
}

#[test]
fn a_single_lowercase_word_is_never_a_claim() {
    assert!(extract::claims("The `config` is read.\n", Path::new("R.md")).is_empty());
    assert!(extract::claims("See the `README`.\n", Path::new("R.md")).is_empty());
}

#[test]
fn a_relative_link_destination_becomes_a_link_claim() {
    let claims = extract::claims("See [setup](./docs/setup.md#install).\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert!(
        matches!(&claims[0].kind, ClaimKind::Link { target, anchor }
            if target == "./docs/setup.md" && anchor.as_deref() == Some("install")),
        "got {:?}",
        claims[0].kind
    );
}

#[test]
fn absolute_and_external_link_destinations_are_not_claims() {
    // External URL checking is `lychee`'s job, not ours.
    assert!(extract::claims("[x](https://example.com/a).\n", Path::new("R.md")).is_empty());
    assert!(extract::claims("[x](mailto:a@b.com).\n", Path::new("R.md")).is_empty());
}

fn commands(doc: &str) -> Vec<String> {
    extract::claims(doc, Path::new("R.md"))
        .into_iter()
        .filter(|c| matches!(c.kind, ClaimKind::Command { .. }))
        .map(|c| c.text)
        .collect()
}

#[test]
fn a_shell_fence_yields_one_command_per_segment() {
    let doc = "```bash\n# set up\nFOO=1 make demo && npm run build\nsudo just release\n```\n";
    assert_eq!(
        commands(doc),
        vec!["make demo", "npm run build", "just release"]
    );
}

#[test]
fn an_untagged_fence_yields_no_command() {
    assert!(commands("```\nmake demo\n```\n").is_empty());
}

#[test]
fn a_command_computed_at_runtime_is_never_a_claim() {
    // It cannot be provably broken, so we say nothing about it.
    assert!(commands("```sh\nmake $TARGET\n```\n").is_empty());
    assert!(commands("```sh\n$(which make) demo\n```\n").is_empty());
}

#[test]
fn an_unrecognised_runner_yields_no_command() {
    assert!(commands("```sh\nfrobnicate --all\n```\n").is_empty());
}

#[test]
fn a_line_continuation_is_joined_before_segmentation() {
    assert_eq!(commands("```sh\nmake \\\n  demo\n```\n"), vec!["make demo"]);
}

fn kinds(doc: &str) -> Vec<ClaimKind> {
    extract::claims(doc, Path::new("R.md"))
        .into_iter()
        .map(|c| c.kind)
        .collect()
}

#[test]
fn a_version_in_prose_is_a_claim() {
    let claims = extract::claims("This needs Node 24 to build.\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert!(
        matches!(&claims[0].kind, ClaimKind::Version { tool, version }
            if tool == "Node" && version == "24"),
        "got {:?}",
        claims[0].kind
    );
}

#[test]
fn a_version_inside_any_fence_is_a_claim() {
    // Versions are the one claim type read from every fence, not just shell ones.
    let claims = extract::claims("```text\nPython 3.13\n```\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert!(
        matches!(&claims[0].kind, ClaimKind::Version { tool, version }
        if tool == "Python" && version == "3.13")
    );
}

#[test]
fn an_ignore_comment_suppresses_the_next_block_only() {
    let doc = "<!-- stilltrue:ignore -->\n\nSee `first/one.md`.\n\nSee `second/two.md`.\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].text, "second/two.md");
}

#[test]
fn an_ignore_comment_suppresses_a_whole_fence() {
    // Anchoring to the block, not the line, is what makes a claim inside a fence
    // suppressible at all.
    let doc = "<!-- stilltrue:ignore -->\n\n```bash\nmake demo\n```\n";
    assert!(extract::claims(doc, Path::new("R.md")).is_empty());
}

#[test]
fn ignore_file_suppresses_the_whole_document() {
    let doc = "# Title\n\n<!-- stilltrue:ignore-file reason goes here -->\n\nSee `a/b.md`.\n";
    assert!(extract::claims(doc, Path::new("R.md")).is_empty());
}

#[test]
fn a_trailing_reason_does_not_break_the_ignore_marker() {
    let doc = "<!-- stilltrue:ignore fixture is intentionally broken -->\n\nSee `a/b.md`.\n";
    assert!(kinds(doc).is_empty());
}

#[test]
fn a_glob_pattern_is_not_a_path_claim() {
    // Found by running stilltrue on itself: `docs/**` is a pattern, not a path.
    for doc in [
        "Set `docs/**`.\n",
        "Set `README*`.\n",
        "Set `**/SKILL.md`.\n",
        "Set `a[0].py`.\n",
    ] {
        assert!(
            extract::claims(doc, Path::new("R.md")).is_empty(),
            "{doc:?}"
        );
    }
}

#[test]
fn a_bare_separator_is_not_a_path() {
    // Found by dogfooding: this project's own prose about "contains `/`" was flagged.
    for doc in ["Contains `/`.\n", "Contains `//`.\n", "Contains `::`.\n"] {
        assert!(
            extract::claims(doc, Path::new("R.md")).is_empty(),
            "{doc:?}"
        );
    }
}

#[test]
fn a_bare_extensionless_reference_is_not_a_path_claim() {
    // `actions/checkout`, `@scope/pkg`, and this tool's own rule ids all have the shape
    // of a path and none of the substance. Real READMEs are full of them.
    for doc in [
        "Uses `actions/checkout` first.\n",
        "See `fiberplane/drift`.\n",
        "Rule `stilltrue/command/rot`.\n",
    ] {
        assert!(
            extract::claims(doc, Path::new("R.md")).is_empty(),
            "{doc:?}"
        );
    }
}

#[test]
fn a_directory_reference_is_still_a_path_when_marked_as_one() {
    // The near-miss: an author who wants a directory checked says so, with a trailing
    // slash or a relative prefix.
    for doc in [
        "See `docs/adr/`.\n",
        "See `./docs/adr`.\n",
        "See `docs/setup.md`.\n",
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "{doc:?}");
        assert_eq!(claims[0].kind, ClaimKind::Path, "{doc:?}");
    }
}

#[test]
fn a_segment_that_is_not_an_identifier_is_not_a_symbol() {
    // poetry documents `project.requires-python`, a pyproject table key. A hyphen
    // cannot appear in a Python name, so this was never a symbol claim.
    let claims = extract::claims(
        "See `project.requires-python` for detail.\n",
        Path::new("CLAUDE.md"),
    );
    assert!(claims.is_empty(), "got {claims:?}");
}

#[test]
fn a_dotted_identifier_is_still_a_symbol() {
    // The near-miss: the rule tightens what counts as a name, not what counts as a dot.
    let claims = extract::claims(
        "See `project.maintainers` for detail.\n",
        Path::new("CLAUDE.md"),
    );
    assert_eq!(claims.len(), 1, "got {claims:?}");
}

#[test]
fn a_code_span_wrapped_across_a_line_becomes_one_claim() {
    // CommonMark converts a line ending inside a code span to a space. ripgrep's README
    // wraps `cargo binstall` across two lines; keeping the newline corrupts the claim
    // text and breaks the one-finding-per-line contract of human output.
    let claims = extract::claims(
        "Alternatively use `cargo\nbinstall` to install it.\n",
        Path::new("README.md"),
    );
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].text, "cargo binstall");
}

#[test]
fn no_claim_text_ever_contains_a_newline() {
    // The general form of the rule above, across every claim type.
    let doc = "See `docs/\nguide.md`, run `make\ndemo`, set `API_\nKEY`.\n";
    let claims = extract::claims(doc, Path::new("README.md"));
    for claim in &claims {
        assert!(!claim.text.contains('\n'), "newline in {claim:?}");
    }
}

#[test]
fn columns_are_counted_in_characters_not_bytes() {
    // A byte column misplaces the annotation on every line containing non-ASCII — in
    // human output, in SARIF, and in the Action's `::error col=` alike. SARIF permits
    // only utf16CodeUnits or unicodeCodePoints (§3.14.27); bytes is not a legal unit.
    let claims = extract::claims(
        "Café — naïve `docs/ghost.md` here.\n",
        Path::new("CLAUDE.md"),
    );
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].line, 1);
    assert_eq!(claims[0].column, 15);
}

#[test]
fn an_ignore_comment_before_a_heading_covers_the_heading_only() {
    // Stepping out of a section to find the next block must not land on the *section*,
    // which contains the heading and everything under it. One marker silently
    // disabling the rest of a document is the clean-bill-of-health failure again.
    let doc = "<!-- stilltrue:ignore -->\n# Heading `first/one.md`\n\nSee `second/two.md`.\n\nAnd `third/three.md`.\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    let texts: Vec<&str> = claims.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["second/two.md", "third/three.md"],
        "got {claims:?}"
    );
}

#[test]
fn an_ignore_comment_at_the_end_of_a_section_still_reaches_the_next_block() {
    // The near-miss for the rule above: the fallback exists because an HTML block
    // swallows the blank line after it, and it must keep working.
    let doc = "# Title\n\n<!-- stilltrue:ignore -->\n\n```bash\nmake demo\n```\n";
    assert!(extract::claims(doc, Path::new("R.md")).is_empty());
}

#[test]
fn a_shell_prompt_does_not_hide_the_command() {
    // The commonest way to write a command in a README is a console fence with a `$`
    // prompt. Reading that `$` as runtime-computed text erased 83% of the command
    // surface across a fifteen-repository corpus — 366 claims in typer alone.
    // All three prompt characters, not just the one: `%` is zsh's default and `>` is
    // what a continuation or a Windows transcript writes. Mutation testing found that
    // only `$` was held down, so the other two worked by luck rather than by test.
    for prompt in ["$", "%", ">"] {
        let doc = format!("```console\n{prompt} just fmt\n```\n");
        let claims = extract::claims(&doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "prompt {prompt:?}: got {claims:?}");
        assert_eq!(claims[0].text, "just fmt", "prompt {prompt:?}");
    }

    // The near-miss: a prompt character is stripped, but a runner computed at runtime
    // is still no claim — stripping must not reach past the prompt itself.
    let computed = "```console\n$ $(which just) fmt\n```\n";
    assert!(extract::claims(computed, Path::new("R.md")).is_empty());

    // A leading `VAR=value` is skipped to reach the runner behind it, but the name has
    // to look like a variable. An empty name is not one, so the token is the runner —
    // an unrecognised runner, which yields nothing rather than reaching past it.
    let empty_name = "```console\n$ =oops just fmt\n```\n";
    assert!(
        extract::claims(empty_name, Path::new("R.md")).is_empty(),
        "an empty assignment name must not be skipped over"
    );
}

#[test]
fn degenerate_backticks_yield_nothing_and_never_panic() {
    // Whatever the grammar hands back for these, the span arithmetic inside a code
    // span must not index outside the source. These are the shapes that would make
    // `start + ticks` pass `end - ticks`.
    for doc in [
        "`\n",
        "``\n",
        "```\n",
        "`` ``\n",
        "a ` b ` c\n",
        "`` `make demo` ``\n",
        "```` ```` \n",
        "`\u{00e9}`\n",
        "``\u{4e2d}``\n",
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        for claim in &claims {
            assert!(claim.column >= 1 && claim.line >= 1, "{doc:?}: {claim:?}");
            assert!(claim.span.end <= doc.len(), "{doc:?}: span past the end");
            assert!(doc.is_char_boundary(claim.span.start), "{doc:?}");
            assert!(doc.is_char_boundary(claim.span.end), "{doc:?}");
        }
    }
}

#[test]
fn a_double_backtick_span_is_read_as_its_contents() {
    // A double-backtick span is an ordinary code span, and the surrounding spaces that
    // let it hold a backtick are not part of the claim.
    for doc in ["Run ``make demo`` now.\n", "Run `` make demo `` now.\n"] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "{doc:?}: got {claims:?}");
        assert_eq!(claims[0].text, "make demo", "{doc:?}");
    }

    // The near-miss, and the reason the rule above stops where it does: when the
    // contents are themselves backtick-wrapped, the author is showing the markup
    // rather than naming a command, so there is no claim to judge.
    let showing_syntax =
        extract::claims("Write `` `make demo` `` to show it.\n", Path::new("R.md"));
    assert!(showing_syntax.is_empty(), "got {showing_syntax:?}");
}

#[test]
fn a_shell_fence_locates_each_of_its_commands_separately() {
    // The cursor has to advance past each match it finds, or the second occurrence of
    // an identical command reports the position of the first — and `--fix` would then
    // rewrite the wrong one.
    let doc = "```bash\nmake demo\nmake demo\n```\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 2, "got {claims:?}");
    assert_eq!((claims[0].line, claims[0].column), (2, 1));
    assert_eq!((claims[1].line, claims[1].column), (3, 1));
    assert_ne!(claims[0].span, claims[1].span, "distinct byte spans");
}

#[test]
fn an_indented_fence_command_keeps_its_column() {
    // The offset is built from the fence's content start plus the cursor plus the find,
    // so any one of those going wrong shows up as a column that misses the command.
    let doc = "Intro.\n\n```bash\n    make demo\n```\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!((claims[0].line, claims[0].column), (4, 5));
    assert_eq!(claims[0].end_column - claims[0].column, "make demo".len());
}

#[test]
fn a_fence_offset_is_built_from_three_parts_that_must_all_be_right() {
    // The offset of a command inside a fence is the fence's content start, plus how
    // far the cursor has advanced, plus where the text was found from there. The tests
    // above leave one or another of those at zero, where several wrong arithmetics
    // agree with the right one. Here all three are non-zero and distinct.
    let doc = "Some prose first.\n\nThen:\n\n```bash\n    make seed\n      make serve\n```\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 2, "got {claims:?}");
    assert_eq!(claims[0].text, "make seed");
    assert_eq!((claims[0].line, claims[0].column), (6, 5));
    assert_eq!(claims[1].text, "make serve");
    assert_eq!((claims[1].line, claims[1].column), (7, 7));

    // And the byte spans must land on the commands themselves, since `--fix` writes
    // through them and verifies the text it is replacing first.
    assert_eq!(&doc[claims[0].span.clone()], "make seed");
    assert_eq!(&doc[claims[1].span.clone()], "make serve");
}

#[test]
fn a_padded_code_span_reports_the_text_not_the_padding() {
    // `trim` moves the start of the claim inside the span, so the offset has to be
    // recovered by finding the trimmed text again. Getting that wrong points the
    // column at the backtick, or before it.
    let doc = "Run ` make demo ` now.\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].text, "make demo");
    assert_eq!(claims[0].column, 7, "column should point at the `m`");
    assert_eq!(&doc[claims[0].span.clone()], "make demo");
}

#[test]
fn a_chained_shell_line_is_as_many_commands_as_it_chains() {
    // Each shell operator splits, and each segment is judged on its own runner.
    for (doc, expected) in [
        (
            "```bash\nmake seed && make serve\n```\n",
            vec!["make seed", "make serve"],
        ),
        (
            "```bash\nmake seed || make serve\n```\n",
            vec!["make seed", "make serve"],
        ),
        (
            "```bash\nmake seed ; make serve\n```\n",
            vec!["make seed", "make serve"],
        ),
        (
            "```bash\nmake seed | make serve\n```\n",
            vec!["make seed", "make serve"],
        ),
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        let texts: Vec<&str> = claims.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, expected, "{doc:?}");
    }
}

#[test]
fn a_continuation_on_the_last_line_still_yields_its_command() {
    // Segmentation joins `\` continuations. A fence that ends while a line is still
    // being joined leaves the accumulator holding the only command in the block.
    let doc = "```bash\nmake demo \\\n```\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].text, "make demo");

    // And a continuation that joins two lines is one command, not two.
    let joined = "```bash\nmake \\\n  demo\n```\n";
    let claims = extract::claims(joined, Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].text, "make demo");
}

#[test]
fn a_runner_or_target_computed_at_runtime_is_still_no_claim() {
    // The near-miss: text computed at runtime is no claim, because it cannot be
    // provably broken. Narrowing the rule to the runner and target must not lose that.
    for doc in [
        "```bash\n$(which make) demo\n```\n",
        "```bash\nmake $TARGET\n```\n",
        "```bash\n`which just` fmt\n```\n",
    ] {
        assert!(extract::claims(doc, Path::new("R.md")).is_empty(), "{doc}");
    }
}

#[test]
fn a_computed_argument_does_not_hide_a_real_target() {
    // `make demo FLAGS=$X` names a real target; only the flag is computed.
    let doc = "```bash\nmake demo FLAGS=$X\n```\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
}

#[test]
fn a_computed_script_name_is_not_a_claim() {
    // `npm run $SCRIPT` names its target in the third token, not the second: the
    // second is the literal `run`. Checking a fixed index let the computed name
    // through as a claim about a script called `$SCRIPT`.
    for doc in [
        "```bash\nnpm run $SCRIPT\n```\n",
        "```bash\npnpm run $TASK\n```\n",
        "```bash\nyarn run $NAME\n```\n",
    ] {
        assert!(extract::claims(doc, Path::new("R.md")).is_empty(), "{doc}");
    }
}

#[test]
fn a_named_script_beside_a_computed_flag_is_still_a_claim() {
    // The near-miss: only the target matters.
    let doc = "```bash\nnpm run build -- --out $DIR\n```\n";
    assert_eq!(extract::claims(doc, Path::new("R.md")).len(), 1);
}

#[test]
fn a_version_claim_spans_the_tool_and_the_version() {
    // The end position drives SARIF's endColumn and the Action's `endColumn=`, so a
    // span covering only the tool name underlines the wrong thing.
    let claims = extract::claims("We target Rust 1.98 here.\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].text, "Rust 1.98");
    assert_eq!(
        claims[0].end_column - claims[0].column,
        "Rust 1.98".len(),
        "span covers only part of the claim"
    );
}

#[test]
fn a_version_claim_measures_the_gap_it_actually_spans() {
    // The test above uses one space, where "subtract the trimmed length" and several
    // wrong arithmetics agree. These do not: the separator has to be measured, and the
    // optional `v` counted, or the span underlines the wrong run of characters.
    for (doc, expected) in [
        ("We target Rust 1.98 here.\n", "Rust 1.98"),
        ("We target Rust   1.98 here.\n", "Rust   1.98"),
        ("We target Rust\t1.98 here.\n", "Rust\t1.98"),
        ("We target Rust v1.98 here.\n", "Rust v1.98"),
        ("We target Rust  v1.98 here.\n", "Rust  v1.98"),
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "{doc:?}: got {claims:?}");
        assert_eq!(
            claims[0].end_column - claims[0].column,
            expected.chars().count(),
            "{doc:?}: span should cover {expected:?}"
        );
    }
}

#[test]
fn a_version_needs_a_tool_name_and_a_number_of_its_own() {
    // Each of these is one character away from a version claim. A tool name that is
    // only the tail of a longer word is not a mention of that tool, and a separator
    // with no digits after it is not a version.
    for doc in [
        "We target XNode 22.4 here.\n", // `Node` inside another word
        "We target 3Node 22.4 here.\n", // a digit is a word character too
        "We target Node .5 here.\n",    // no digit starts the version
        "We target Node v here.\n",     // a `v` with nothing after it
        "We target Node\n22.4 here.\n", // a line break is not a separator
        // A family, not a pin. typer's alternatives page says Click was built "using
        // the features available in the language at the time (Python 2.x)", which is a
        // statement about an era, not about what this repository runs on. Same
        // reasoning as the placeholder glob clause: `docs/**` is not a claim that
        // `docs/**` exists, and `2.x` is not a claim to be any particular version.
        "Built for Python 2.x back then.\n",
        "Built for Python 2.X back then.\n",
        "Built for Node 18.* back then.\n",
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        assert!(
            claims.is_empty(),
            "{doc:?} should yield no claim, got {claims:?}"
        );
    }
}

#[test]
fn a_symbol_needs_its_separator_between_two_identifier_characters() {
    // ADR-0017: the separator is the syntactic marker that makes a span a symbol
    // claim rather than a word with punctuation near it. It has to sit *between*
    // identifier characters, which is what these edges are about.
    for text in [
        "graph.query",
        "a.b",
        "mod::name",
        "std::collections::HashMap",
        "Widget.render",
    ] {
        let doc = format!("See `{text}` here.\n");
        let claims = extract::claims(&doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "{text:?} should be a claim: {claims:?}");
        assert!(
            matches!(claims[0].kind, ClaimKind::Symbol),
            "{text:?} should be a Symbol, got {:?}",
            claims[0].kind
        );
    }

    // The near-misses. A separator at either end has nothing on one side of it, and a
    // single colon is not `::`. None of these is a symbol claim.
    for text in [
        ".leading",
        "trailing.",
        "::leading",
        "trailing::",
        "a:b",
        "a::",
        "::b",
        ".",
        "..",
        "a..b",
        "a . b",
        "-.x",
        "x.-",
    ] {
        let doc = format!("See `{text}` here.\n");
        let claims = extract::claims(&doc, Path::new("R.md"));
        assert!(
            !claims.iter().any(|c| matches!(c.kind, ClaimKind::Symbol)),
            "{text:?} should not be a Symbol: {claims:?}"
        );
    }
}

#[test]
fn a_relative_prefix_keeps_a_target_from_being_a_bare_reference() {
    // A slash path whose last segment has no extension is a bare reference and is
    // Unclassified — `actions/checkout` is not a claim that a directory exists. Saying
    // you mean a path is the escape hatch, and each spelling of it has to work.
    for text in ["./docs/guide", "../docs/guide", "docs/guide/"] {
        let doc = format!("See `{text}` here.\n");
        let claims = extract::claims(&doc, Path::new("R.md"));
        assert_eq!(claims.len(), 1, "{text:?} should be a claim: {claims:?}");
        assert!(matches!(claims[0].kind, ClaimKind::Path), "{text:?}");
    }
    // And without one of those spellings it stays Unclassified.
    for text in ["docs/guide", "actions/checkout", "@scope/pkg"] {
        let doc = format!("See `{text}` here.\n");
        assert!(
            extract::claims(&doc, Path::new("R.md")).is_empty(),
            "{text:?} should yield no claim"
        );
    }
}

#[test]
fn a_link_anchor_is_located_where_it_is_written() {
    // The offsets here drive the annotation the reader is shown, and a link's text and
    // its destination are different runs of characters on the same line.
    let doc = "Intro.\n\nSee [the setup section](./docs/setup.md#install) for more.\n";
    let claims = extract::claims(doc, Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0].text, "./docs/setup.md#install");
    assert_eq!(claims[0].line, 3);
    assert_eq!(&doc[claims[0].span.clone()], "./docs/setup.md#install");

    // A same-document anchor is located at the anchor, not at the link text.
    let toc = "Intro.\n\nSee [Configuration](#configuration) below.\n";
    let claims = extract::claims(toc, Path::new("docs/guide.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(&toc[claims[0].span.clone()], "#configuration");
}

#[test]
fn an_empty_anchor_is_not_an_anchor() {
    // `file.md#` addresses the top of that file, and `#` alone addresses the top of
    // this one. Neither names a heading, so neither carries an anchor to check.
    let claims = extract::claims("See [x](./docs/setup.md#).\n", Path::new("R.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert!(
        matches!(&claims[0].kind, ClaimKind::Link { anchor: None, .. }),
        "got {:?}",
        claims[0].kind
    );

    assert!(
        extract::claims("See [top](#).\n", Path::new("R.md")).is_empty(),
        "`#` alone names nothing"
    );
}

#[test]
fn a_version_family_is_not_a_pin_but_a_real_version_still_is() {
    // The near-miss for the clause above: a trailing `x` only makes a family when it
    // stands where a number would. Everything else is an ordinary version claim.
    for (doc, expected) in [
        ("We target Python 3.10 here.\n", "Python 3.10"),
        ("We target Python 3 here.\n", "Python 3"),
        ("We target Python 3.10.x here.\n", ""),
        // A version that ends a sentence: the digits run ends with the full stop, but
        // what follows is a space, not a wildcard. Both halves of the clause have to
        // hold for a family, or every pin written at the end of a sentence vanishes.
        ("Requires Node 18. Then run the build.\n", "Node 18"),
        ("Requires Node 18.4. Then run the build.\n", "Node 18.4"),
    ] {
        let claims = extract::claims(doc, Path::new("R.md"));
        if expected.is_empty() {
            assert!(claims.is_empty(), "{doc:?} is a family: {claims:?}");
        } else {
            assert_eq!(claims.len(), 1, "{doc:?}: got {claims:?}");
            assert_eq!(claims[0].text, expected, "{doc:?}");
        }
    }
}

#[test]
fn a_same_document_anchor_is_a_claim() {
    // A table of contents is the commonest link shape in a README, and every one of
    // its entries points at this document.
    let claims = extract::claims("See [setup](#setup).\n", Path::new("docs/guide.md"));
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert!(matches!(
        &claims[0].kind,
        ClaimKind::Link { target, anchor }
            if target == "guide.md" && anchor.as_deref() == Some("setup")
    ));
    assert_eq!(claims[0].text, "#setup");
}

#[test]
fn a_bare_hash_is_not_a_claim() {
    // The near-miss: `#` alone is a link to the top of the page, naming nothing.
    assert!(extract::claims("See [top](#).\n", Path::new("R.md")).is_empty());
}

#[test]
fn every_command_claim_points_at_its_own_text() {
    // `content_start + cursor + i`, rewound by `offset - content_start`. Three terms,
    // and while every command sat first in its block two of them were zero, so the
    // additions could have been anything. A second command, inside a fence, after
    // prose, is the first input that makes all three non-zero at once.
    //
    // Asserting the position *resolves* to the text beats asserting a column number:
    // it is the property the arithmetic exists to maintain, and it cannot be satisfied
    // by a number that happens to match.
    let doc = concat!(
        "# Build\n\n",
        "Prose first, so the fence does not begin at byte zero.\n\n",
        "```sh\n",
        "make configure && make build\n",
        "npm run lint || npm run fix\n",
        "```\n",
    );

    let claims = extract::claims(doc, Path::new("README.md"));

    assert!(claims.len() >= 4, "expected four commands, got {claims:?}");
    let lines: Vec<&str> = doc.lines().collect();
    for claim in &claims {
        let line = lines[claim.line - 1];
        let at: String = line.chars().skip(claim.column - 1).collect();
        assert!(
            at.starts_with(&claim.text),
            "{:?} is reported at {}:{}, where the document reads {:?}",
            claim.text,
            claim.line,
            claim.column,
            at.chars().take(30).collect::<String>(),
        );
    }
}

#[test]
fn a_command_anchored_on_its_runner_lands_on_the_right_line() {
    // `make   demo` normalises to `make demo`, which does not appear in the block
    // verbatim, so the span falls back to the runner token. That fallback recomputes
    // the offset from `content_start + cursor + i` and it is the only path that does,
    // which left all three terms free: nothing asserted where a fallback-anchored
    // command actually landed.
    //
    // The distances matter. With the first command sitting at the start of the fence,
    // `offset - content_start` and `offset / content_start` are 0 and 1 — close enough
    // that the search still begins past the first command and the right answer comes
    // out of the wrong arithmetic. Pushing both commands deep into the block separates
    // them: 146 against 12, which is the difference between finding the second `make`
    // and finding the first one again.
    let doc = concat!(
        "# Build\n\n",
        "Prose before the fence, so its content does not begin at byte zero.\n\n",
        "```sh\n",
        "# a leading comment, so the first command is not at the content start\n",
        "# a second leading comment, putting real distance between the two\n",
        "make build\n",
        "# a comment between the commands\n",
        "make   demo\n",
        "```\n",
    );

    let claims = extract::claims(doc, Path::new("README.md"));

    assert_eq!(claims.len(), 2, "got {claims:?}");
    assert_eq!(claims[0].text, "make build");
    assert_eq!((claims[0].line, claims[0].column), (8, 1));

    // Line 10, not line 8: the second `make`, not the first one found again.
    assert_eq!(claims[1].text, "make demo");
    assert_eq!(
        (claims[1].line, claims[1].column),
        (10, 1),
        "the fallback must anchor on its own line, got {:?}",
        claims[1]
    );
    // The span covers the runner alone, because that is the only text that survived
    // normalisation unchanged and can be pointed at.
    assert_eq!((claims[1].end_line, claims[1].end_column), (10, 5));

    let line = doc.lines().nth(claims[1].line - 1).unwrap();
    assert!(
        line[claims[1].column - 1..].starts_with("make"),
        "reported position does not point at the runner: {line:?}"
    );
}

#[test]
fn an_angle_bracketed_anchor_points_at_the_anchor_not_the_bracket() {
    // `<...>` is a legal CommonMark destination wrapper, and the brackets are trimmed
    // off the claim while staying in the document. That makes this the only anchor
    // shape where the offset has to step over anything: everywhere else the search
    // finds the destination at position zero, where `start + 0` and `start - 0` are the
    // same number and no arithmetic can be wrong.
    //
    // The end position is pinned for the same reason — nothing else asserted where an
    // anchor claim stops, so its width was free.
    let doc = "See [setup](<#setup>) first.\n";

    let claims = extract::claims(doc, Path::new("docs/guide.md"));

    assert_eq!(claims.len(), 1, "got {claims:?}");
    let claim = &claims[0];
    assert_eq!(
        claim.text, "#setup",
        "the brackets are not part of the claim"
    );

    let line = doc.lines().next().unwrap();
    assert!(
        line[claim.column - 1..].starts_with("#setup"),
        "column {} points at {:?}, not the anchor",
        claim.column,
        &line[claim.column - 1..],
    );
    assert_eq!(
        (claim.end_line, claim.end_column),
        (claim.line, claim.column + "#setup".chars().count()),
        "an anchor spans exactly its own text: {claim:?}",
    );
}

#[test]
fn a_bracketed_file_link_points_at_the_path_not_the_bracket() {
    // The same arithmetic as the anchor branch above, in the branch that handles an
    // ordinary destination — a separate code path, and it had the same gap. Without
    // `<...>` the destination node *is* the destination, the search returns zero, and
    // `start + 0` equals `start - 0`.
    let doc = "See [guide](<./docs/x.md>) first.\n";

    let claims = extract::claims(doc, Path::new("README.md"));

    assert_eq!(claims.len(), 1, "got {claims:?}");
    let claim = &claims[0];
    assert_eq!(
        claim.text, "./docs/x.md",
        "the brackets are not part of the claim"
    );

    let line = doc.lines().next().unwrap();
    assert!(
        line[claim.column - 1..].starts_with("./docs/x.md"),
        "column {} points at {:?}, not the path",
        claim.column,
        &line[claim.column - 1..],
    );
    assert_eq!(
        (claim.end_line, claim.end_column),
        (claim.line, claim.column + "./docs/x.md".chars().count()),
        "a link spans exactly its own destination: {claim:?}",
    );
}

#[test]
fn a_separator_is_two_colons_and_the_name_after_it_is_measured_from_the_right_place() {
    // `has_qualified_separator` walks bytes looking for `.` or `::` between identifier
    // characters, and both of its offsets were free.
    //
    // `i + 1` is what makes it `::` rather than `:`. Turned into `i`, the byte examined
    // is the colon already matched, so a single colon becomes a namespace and
    // `host:port` — a perfectly ordinary thing to write in backticks — becomes a symbol
    // claim this repository is then asked to define.
    //
    // `i + 2` reaches the first character after the separator. Turned into `i * 2` it
    // coincides for a separator at index one and diverges after, so every short example
    // in the suite agreed with it. `tokio::fs` does not: `i * 2` runs off the end, and
    // a real module path stops being a symbol at all.
    for (text, is_symbol) in [
        ("host:port", false),
        ("tokio::fs", true),
        ("serde::de", true),
        ("std::fs", true),
        ("Vec::new", true),
    ] {
        let doc = format!("Set `{text}` here.\n");
        let claims = extract::claims(&doc, Path::new("README.md"));
        let found = claims
            .iter()
            .any(|c| matches!(c.kind, ClaimKind::Symbol) && c.text == text);
        assert_eq!(
            found, is_symbol,
            "{text:?}: expected symbol={is_symbol}, got {claims:?}",
        );
    }
}

#[test]
fn a_rust_edition_is_not_a_rust_version() {
    // ADR-0024. "Rust 2024" names the 2024 edition, which a toolchain pinned at 1.98
    // supports; read as a version it contradicted every 1.x pin, as rot, blamed on the
    // commit that created the pin file.
    for edition in ["2015", "2018", "2021", "2024", "2030"] {
        let text = format!("Written in Rust {edition}.\n");
        assert!(
            extract::claims(&text, Path::new("R.md")).is_empty(),
            "{text}"
        );
    }
    // What stays a version: a Rust version, a dotted number, and another tool's digits.
    for text in [
        "Needs Rust 1.70.\n",
        "Needs Rust 2024.1.\n",
        "Needs Rust 1999.\n",
        "Needs Node 2024.\n",
    ] {
        assert_eq!(extract::claims(text, Path::new("R.md")).len(), 1, "{text}");
    }
}

#[test]
fn suppression_markers_are_counted_for_the_run_report() {
    // ADR-0019: what a marker hides was never extracted, so the markers are what the
    // report can count — including the one that removes a whole file.
    let blocks = "<!-- stilltrue:ignore -->\n\nSee `a/b.md`.\n\n<!-- stilltrue:ignore -->\n\nNothing here.\n";
    let extraction = extract::extract(blocks, Path::new("R.md"));
    assert_eq!(extraction.markers, 2);
    assert!(!extraction.whole_file);
    assert_eq!(extraction.unused_suppressions.len(), 1);
    let whole = "<!-- stilltrue:ignore-file -->\n\nSee `a/b.md`.\n";
    let extraction = extract::extract(whole, Path::new("R.md"));
    assert_eq!(extraction.markers, 1);
    assert!(extraction.whole_file);
    assert!(extraction.claims.is_empty());
    assert!(!extraction.unparseable);
}
