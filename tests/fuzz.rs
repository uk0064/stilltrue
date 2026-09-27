//! Pathological input, deterministically generated.
//!
//! The extractor reads whatever Markdown or reStructuredText the internet produced, and
//! a linter must never fail a build because it choked. Not `cargo-fuzz`: this is a
//! seeded generator so it runs in ordinary CI and any failure reproduces from its seed.

use std::path::Path;

use stilltrue::extract;
use stilltrue::rst;

/// xorshift64*, so a failing case is reproducible from its seed alone.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn pick<'a>(&mut self, options: &[&'a str]) -> &'a str {
        options[(self.next() % options.len() as u64) as usize]
    }
}

/// Fragments chosen to land on the edges: unbalanced delimiters, nested fences, exotic
/// whitespace, astral characters, and the byte sequences that have broken this before.
#[rustfmt::skip]
const FRAGMENTS: &[&str] = &[
    "`", "``", "```", "```bash", "~~~", "<!-- stilltrue:ignore -->", "<!-- stilltrue:ignore-file -->",
    "# ", "## Heading {#id}", "[a](b.md#c)", "[](){#x}", "![img](x.png)", "- item", "> quote",
    "|a|b|", "\t", "   ", "\r\n", "\n", "\u{2028}", "\u{feff}", "é", "🌍", "\u{0}", "\\",
    "make demo", "$ npm run x", "`cargo\nbinstall`", "docs/a.md", "A_B_C", "x.y()", "Rust 1.98",
    "`` ``", ".. code-block:: bash", ".. versionadded:: 1.0", "``lit``", "`t <u.rst>`_",
    "*", "**", "_", "[", "]", "(", ")", "{", "}", "<", ">", "&", "\"", "'", ":", "::",
];

fn document(seed: u64, parts: usize) -> String {
    let mut rng = Rng(seed | 1);
    let mut out = String::new();
    for _ in 0..parts {
        out.push_str(rng.pick(FRAGMENTS));
        if rng.next().is_multiple_of(3) {
            out.push('\n');
        }
    }
    out
}

/// Every invariant a claim must satisfy, whatever produced it.
fn check(source: &str, claims: &[stilltrue::claim::Claim], what: &str) {
    for claim in claims {
        assert!(claim.line >= 1, "line 0 in {what}");
        assert!(claim.column >= 1, "column 0 in {what}");
        assert!(
            claim.end_line >= claim.line,
            "end before start in {what}: {claim:?}"
        );
        assert!(
            claim.span.start <= claim.span.end,
            "inverted span in {what}: {claim:?}"
        );
        assert!(
            claim.span.end <= source.len(),
            "span past end in {what}: {claim:?}"
        );
        assert!(
            source.is_char_boundary(claim.span.start) && source.is_char_boundary(claim.span.end),
            "span splits a character in {what}: {claim:?}"
        );
        assert!(
            !claim.text.contains('\n'),
            "newline in claim text in {what}: {claim:?}"
        );
        assert!(!claim.text.is_empty(), "empty claim text in {what}");
    }
}

#[test]
fn the_markdown_extractor_survives_pathological_input() {
    for seed in 1..400u64 {
        let source = document(seed, 40);
        let claims = extract::claims(&source, Path::new("R.md"));
        check(&source, &claims, &format!("markdown seed {seed}"));
    }
}

#[test]
fn the_restructuredtext_extractor_survives_pathological_input() {
    for seed in 1..400u64 {
        let source = document(seed, 40);
        let extraction = rst::extract(&source, Path::new("R.rst"));
        check(&source, &extraction.claims, &format!("rst seed {seed}"));
    }
}

#[test]
fn extraction_is_deterministic() {
    // Snapshot tests are worthless if the same bytes can yield two answers.
    for seed in 1..80u64 {
        let source = document(seed, 30);
        let once = extract::claims(&source, Path::new("R.md"));
        let twice = extract::claims(&source, Path::new("R.md"));
        assert_eq!(once.len(), twice.len(), "markdown seed {seed}");
        for (a, b) in once.iter().zip(&twice) {
            assert_eq!(a.text, b.text);
            assert_eq!(a.span, b.span);
        }
    }
}

#[test]
fn a_document_of_nothing_but_delimiters_yields_nothing_absurd() {
    for source in [
        "",
        "`",
        "``",
        "```",
        "```bash",
        "```bash\n",
        "[](",
        "[](#",
        "<!-- stilltrue:ignore -->",
        "\u{feff}",
        "\u{0}\u{0}\u{0}",
    ] {
        let claims = extract::claims(source, Path::new("R.md"));
        check(source, &claims, &format!("{source:?}"));
        let extraction = rst::extract(source, Path::new("R.rst"));
        check(source, &extraction.claims, &format!("rst {source:?}"));
    }
}
