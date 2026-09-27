//! Seam: the reStructuredText reader. Hand-rolled rather than parsed, so its edges are
//! where the bugs would be — every case here pins a mutant that survived without it.

use std::path::Path;

use stilltrue::claim::ClaimKind;
use stilltrue::rst;

fn claims(source: &str) -> Vec<stilltrue::claim::Claim> {
    rst::extract(source, Path::new("docs/guide.rst")).claims
}

fn texts(source: &str) -> Vec<String> {
    claims(source).into_iter().map(|c| c.text).collect()
}

#[test]
fn an_inline_literal_is_located_exactly() {
    // The column drives every annotation, and it is computed by arithmetic that no
    // test constrained.
    let source = "Guide\n=====\n\nRun ``make demo`` now.\n";
    let found = claims(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].text, "make demo");
    assert_eq!(found[0].line, 4);
    assert_eq!(
        found[0].column, 7,
        "column points at the literal, not the backticks"
    );
    assert_eq!(&source[found[0].span.clone()], "make demo");
}

#[test]
fn several_inline_literals_on_one_line_are_all_found() {
    let found = texts("See ``docs/a.md`` and ``docs/b.md`` and ``docs/c.md``.\n");
    assert_eq!(found, vec!["docs/a.md", "docs/b.md", "docs/c.md"]);
}

#[test]
fn an_empty_literal_is_not_a_claim() {
    assert!(claims("An empty ```` literal.\n").is_empty());
}

#[test]
fn a_link_is_located_exactly() {
    let source = "See `the setup <setup.rst>`_ for detail.\n";
    let found = claims(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].text, "setup.rst");
    assert_eq!(&source[found[0].span.clone()], "setup.rst");
}

#[test]
fn several_links_on_one_line_are_all_found() {
    let source = "`a <one.rst>`_ then `b <two.rst>`_ then `c <three.rst>`_.\n";
    let found = claims(source);
    assert_eq!(
        found.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        vec!["one.rst", "two.rst", "three.rst"]
    );
    // Each span has to land on its own target, which is what the offset arithmetic is
    // for: a wrong `+` puts the second link's span inside the first.
    for claim in &found {
        assert_eq!(&source[claim.span.clone()], claim.text);
    }
}

#[test]
fn a_link_anchor_is_carried() {
    let found = claims("See `x <guide.rst#setup>`_.\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        matches!(&found[0].kind, ClaimKind::Link { target, anchor }
            if target == "guide.rst" && anchor.as_deref() == Some("setup")),
        "{:?}",
        found[0].kind
    );
}

#[test]
fn a_link_with_an_empty_anchor_keeps_the_whole_target() {
    let found = claims("See `x <guide.rst#>`_.\n");
    assert!(
        matches!(&found[0].kind, ClaimKind::Link { anchor, .. } if anchor.is_none()),
        "{:?}",
        found[0].kind
    );
}

#[test]
fn an_external_or_absolute_link_is_not_a_claim() {
    for source in [
        "See `x <https://example.com/a>`_.\n",
        "Mail `x <mailto:a@b.c>`_.\n",
        "See `x </absolute.rst>`_.\n",
    ] {
        assert!(claims(source).is_empty(), "{source:?}");
    }
}

#[test]
fn only_a_shell_directive_body_yields_commands() {
    let shell = texts(".. code-block:: bash\n\n    make demo\n");
    assert_eq!(shell, vec!["make demo"]);
    // The near-miss: a body that does not say it is shell is a sample, not a command.
    assert!(texts(".. code-block:: toml\n\n    make demo\n").is_empty());
    assert!(texts(".. code-block::\n\n    make demo\n").is_empty());
}

#[test]
fn a_directive_body_ends_when_the_indentation_does() {
    // The indent comparison had four surviving mutants. A line at or below the
    // directive's own indentation is outside its body.
    let found = texts(".. code-block:: bash\n\n    make inside\n\nmake outside\n");
    assert_eq!(found, vec!["make inside"], "body boundary is wrong");
}

#[test]
fn a_version_directive_body_is_not_read() {
    // flask's config page carries a dozen of these naming keys since removed.
    assert!(texts(".. versionadded:: 0.4\n   ``LOGGER_NAME``\n").is_empty());
    assert!(texts(".. deprecated:: 2.0\n   ``OLD_KEY``\n").is_empty());
    // The near-miss: an ordinary directive's body is still prose worth reading.
    assert_eq!(
        texts(".. note::\n   See ``docs/a.md`` for detail.\n"),
        vec!["docs/a.md"]
    );
}

#[test]
fn a_whole_file_suppression_silences_everything() {
    assert!(claims(".. stilltrue:ignore-file\n\nSee ``docs/a.md``.\n").is_empty());
}

#[test]
fn a_suppression_covers_the_next_block_only() {
    let found = texts(".. stilltrue:ignore\n\nSee ``docs/a.md``.\n\nSee ``docs/b.md``.\n");
    assert_eq!(found, vec!["docs/b.md"]);
}

#[test]
fn an_unused_suppression_is_reported() {
    let extraction = rst::extract(
        ".. stilltrue:ignore\n\nNothing claimed here.\n",
        Path::new("docs/guide.rst"),
    );
    assert_eq!(extraction.unused_suppressions.len(), 1);
    assert_eq!(extraction.unused_suppressions[0].line, 1);
}

#[test]
fn a_version_claim_is_found_in_prose() {
    assert_eq!(texts("We target Rust 1.98 here.\n"), vec!["Rust 1.98"]);
}

#[test]
fn a_version_directive_body_ends_when_the_indentation_does() {
    // A line back at the directive's own indentation is outside its body, so the claim
    // on it is read. `>` rather than `>=`.
    let found = texts(".. versionadded:: 0.4\n   ``IGNORED_KEY``\n\nSee ``docs/a.md``.\n");
    assert_eq!(found, vec!["docs/a.md"]);
}

#[test]
fn a_suppression_that_covered_something_is_not_reported_unused() {
    // The near-miss for the unused-marker report: dropping the "did it cover anything"
    // check would flag every suppression in the file.
    let extraction = rst::extract(
        ".. stilltrue:ignore\n\nSee ``docs/a.md``.\n\nSee ``docs/b.md``.\n",
        Path::new("docs/guide.rst"),
    );
    assert!(
        extraction.unused_suppressions.is_empty(),
        "a used marker was reported: {:?}",
        extraction.unused_suppressions
    );
}

#[test]
fn a_literal_padded_inside_its_backticks_is_still_located_exactly() {
    // reStructuredText allows `` x ``; the span must point at the name, not the space.
    let source = "See ``   docs/a.md   `` here.\n";
    let found = claims(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].text, "docs/a.md");
    assert_eq!(
        &source[found[0].span.clone()],
        "docs/a.md",
        "span drifted off the name"
    );
}

#[test]
fn suppression_markers_are_counted_for_the_run_report() {
    let blocks = ".. stilltrue:ignore\n\nSee ``a/b.rst``.\n\n.. stilltrue:ignore\n\nPlain text.\n\n.. stilltrue:ignore\n";
    let extraction = stilltrue::rst::extract(blocks, std::path::Path::new("R.rst"));
    assert_eq!(extraction.markers, 3);
    assert!(!extraction.whole_file);
    // A marker before a whole-file one is still counted, and so is the whole-file one.
    let whole = ".. stilltrue:ignore\n\n.. stilltrue:ignore-file\n\nSee ``a/b.rst``.\n";
    let extraction = stilltrue::rst::extract(whole, std::path::Path::new("R.rst"));
    assert_eq!(extraction.markers, 2);
    assert!(extraction.whole_file);
    assert!(extraction.claims.is_empty());
}
