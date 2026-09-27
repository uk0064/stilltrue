//! Setup documentation is a product surface.
//!
//! An example someone copies into their CI is an instruction a machine will execute, so
//! it is held to what this repository can prove: a pinned release that was verified,
//! and a trigger that runs on code changes as well as on documents.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every document a user is sent to in order to set the tool up.
fn setup_documents() -> Vec<PathBuf> {
    let mut out = vec![root().join("README.md")];
    for dir in ["docs", "integrations"] {
        collect(&root().join(dir), &mut out);
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// Versions a maintainer recorded as verified from the published archives.
fn verified() -> Vec<String> {
    std::fs::read_to_string(root().join("tests/release/verified.txt"))
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Every `uk0064/stilltrue@<ref>` in a text.
fn action_refs(text: &str) -> Vec<String> {
    text.match_indices("uk0064/stilltrue@")
        .map(|(at, needle)| {
            text[at + needle.len()..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '`' && *c != ')')
                .collect()
        })
        .collect()
}

#[test]
fn setup_examples_pin_a_verified_release_or_say_that_none_is() {
    let verified = verified();
    let mut seen = 0;
    for document in setup_documents() {
        let text = std::fs::read_to_string(&document).unwrap();
        for reference in action_refs(&text) {
            seen += 1;
            let allowed = reference == "vX.Y.Z"
                || reference
                    .strip_prefix('v')
                    .is_some_and(|v| verified.iter().any(|known| known == v));
            assert!(
                allowed,
                "{}: `@{reference}` is not a verified release. Pin one listed in \
                 tests/release/verified.txt, or show the vX.Y.Z placeholder",
                document.display()
            );
        }
    }
    assert!(seen >= 2, "no Action example was checked at all");
}

#[test]
fn a_mutable_branch_is_never_the_pin() {
    // The rule above already refuses it; this names the specific trap, because `@main`
    // is the one that looks like it works and then changes underneath every consumer.
    for reference in ["@main", "@master", "@HEAD"] {
        for document in setup_documents() {
            let text = std::fs::read_to_string(&document).unwrap();
            assert!(
                !text.contains(&format!("uk0064/stilltrue{reference}")),
                "{} pins {reference}",
                document.display()
            );
        }
    }
}

/// The path patterns under a `paths:` key in a YAML-ish block.
fn path_filters(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("paths:") || trimmed.starts_with("paths-ignore:")) {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let mut patterns = Vec::new();
        if let Some(inline) = trimmed.split_once('[').map(|(_, rest)| rest) {
            patterns.extend(
                inline
                    .trim_end_matches(']')
                    .split(',')
                    .map(|p| p.trim().trim_matches(['"', '\'']).to_string()),
            );
        }
        for next in &lines[i + 1..] {
            let next_trimmed = next.trim_start();
            let next_indent = next.len() - next_trimmed.len();
            if next_indent <= indent || !next_trimmed.starts_with("- ") {
                break;
            }
            patterns.push(
                next_trimmed[2..]
                    .trim()
                    .trim_matches(['"', '\''])
                    .to_string(),
            );
        }
        out.push(patterns);
    }
    out
}

fn documents_only(patterns: &[String]) -> bool {
    !patterns.is_empty()
        && patterns.iter().all(|p| {
            [".md", ".mdx", ".rst", ".markdown"]
                .iter()
                .any(|e| p.ends_with(e))
        })
}

#[test]
fn no_setup_example_runs_only_when_a_document_changed() {
    // A code change is what breaks an untouched document. A filter that runs the check
    // only when Markdown changed skips exactly the pull requests it exists for.
    for document in setup_documents() {
        let text = std::fs::read_to_string(&document).unwrap();
        for patterns in path_filters(&text) {
            assert!(
                !documents_only(&patterns),
                "{} filters to documents only: {patterns:?}",
                document.display()
            );
        }
    }
}

#[test]
fn ci_runs_on_every_change() {
    let workflows = root().join(".github/workflows");
    for name in ["ci.yml", "release.yml", "published-smoke.yml"] {
        let text = std::fs::read_to_string(workflows.join(name)).unwrap();
        assert!(
            path_filters(&text).is_empty(),
            "{name} restricts when it runs by path"
        );
    }
}

#[test]
fn the_filter_detector_can_see_a_documents_only_filter() {
    // The two tests above pass on silence, so show that the detector is not blind.
    let inline = "on:\n  pull_request:\n    paths: ['**.md', 'docs/**.md']\n";
    let listed =
        "on:\n  push:\n    paths:\n      - '**/*.md'\n      - \"*.rst\"\n  pull_request:\n";
    let mixed = "on:\n  push:\n    paths:\n      - '**/*.md'\n      - 'src/**'\n";
    assert!(documents_only(&path_filters(inline)[0]));
    assert!(documents_only(&path_filters(listed)[0]));
    assert!(!documents_only(&path_filters(mixed)[0]));
    assert_eq!(action_refs("uses: uk0064/stilltrue@main\n"), ["main"]);
}

/// Every `rev:` pinned under a `repo:` that is this repository.
fn precommit_revs(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("- repo:")
            && line.contains("github.com/uk0064/stilltrue")
            && let Some(rev) = lines[i + 1..]
                .iter()
                .take(3)
                .find_map(|l| l.trim_start().strip_prefix("rev:"))
        {
            out.push(rev.trim().to_string());
        }
    }
    out
}

#[test]
fn a_pre_commit_example_pins_a_verified_release_or_the_placeholder() {
    let verified = verified();
    let mut seen = 0;
    for document in setup_documents() {
        let text = std::fs::read_to_string(&document).unwrap();
        for rev in precommit_revs(&text) {
            seen += 1;
            let allowed = rev == "vX.Y.Z"
                || rev
                    .strip_prefix('v')
                    .is_some_and(|v| verified.iter().any(|known| known == v));
            assert!(
                allowed,
                "{}: pre-commit rev `{rev}` is not verified",
                document.display()
            );
        }
    }
    assert!(seen >= 1, "no pre-commit example was checked");
    assert_eq!(
        precommit_revs("repos:\n  - repo: https://github.com/uk0064/stilltrue\n    rev: main\n"),
        ["main"]
    );
}

#[test]
fn every_published_hook_checks_the_whole_repository_on_every_commit() {
    let manifest = std::fs::read_to_string(root().join(".pre-commit-hooks.yaml")).unwrap();
    let hooks: Vec<&str> = manifest.split("\n- id: ").skip(1).collect();
    assert_eq!(hooks.len(), 2, "{manifest}");
    for hook in hooks {
        let id = hook.lines().next().unwrap();
        // A file list would limit the check to what changed, and a code change is what
        // breaks an untouched document.
        assert!(hook.contains("\n  pass_filenames: false\n"), "{id}");
        // Without this, a commit that stages no document skips the hook entirely.
        assert!(hook.contains("\n  always_run: true\n"), "{id}");
    }
}
