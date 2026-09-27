//! `stilltrue.toml`, read from the repository root only.

use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

use crate::coverage::Diagnostic;

/// The documents linted when nothing is configured.
pub const DEFAULT_INCLUDE: &[&str] = &[
    "README*",
    "docs/**",
    "CLAUDE.md",
    "AGENTS.md",
    ".cursor/rules/**",
    "**/SKILL.md",
    ".github/copilot-instructions.md",
    // At any depth, too. A monorepo documents `pnpm run build` in the package's own
    // README, which is the case the nearest-ancestor-manifest walk is built for —
    // and that walk never ran, because the file was not a document.
    "**/README*",
    "**/CLAUDE.md",
    "**/AGENTS.md",
];

/// Historical records, excluded by default at any depth (ADR-0012). A changelog
/// describes what was true at a release, so an entry naming something that has since
/// moved is the genre working, not a defect. Matched case-insensitively, because the
/// corpus spelled them `docs/changelog.md` and `docs/release-notes.md`.
pub const DEFAULT_EXCLUDE: &[&str] = &[
    "CHANGELOG*",
    "CHANGES*",
    "HISTORY*",
    "NEWS*",
    "RELEASE-NOTES*",
    "RELEASE_NOTES*",
    "RELEASES*",
    "**/CHANGELOG*",
    "**/CHANGES*",
    "**/HISTORY*",
    "**/NEWS*",
    "**/RELEASE-NOTES*",
    "**/RELEASE_NOTES*",
    "**/RELEASES*",
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Replaces the defaults rather than extending them.
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub ignore: Ignore,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ignore {
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
}

impl Config {
    /// Read `stilltrue.toml` from the repository root, and say what went wrong doing it.
    /// Its absence is the common case and is not an error; an unreadable or invalid one
    /// warns and is skipped — and because a configuration that could not be
    /// used changes which documents were selected, the run cannot call itself complete.
    pub fn load_reporting(root: &Path) -> (Self, Vec<Diagnostic>) {
        let path = root.join("stilltrue.toml");
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), Vec::new());
            }
            Err(error) => {
                eprintln!("stilltrue: warning: ignoring {}: {error}", path.display());
                let diagnostic = Diagnostic::failure(
                    "config-unreadable",
                    format!("could not read stilltrue.toml: {error}"),
                    Some("stilltrue.toml".into()),
                );
                return (Self::default(), vec![diagnostic]);
            }
        };
        let config: Self = match toml::from_str(&contents) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("stilltrue: warning: ignoring {}: {error}", path.display());
                let diagnostic = Diagnostic::failure(
                    "config-invalid",
                    format!(
                        "stilltrue.toml was ignored: {}",
                        one_line(&error.to_string())
                    ),
                    Some("stilltrue.toml".into()),
                );
                return (Self::default(), vec![diagnostic]);
            }
        };
        let diagnostics = config
            .include
            .iter()
            .flatten()
            .chain(&config.exclude)
            .filter(|pattern| GlobBuilder::new(pattern).build().is_err())
            .map(|pattern| {
                Diagnostic::failure(
                    "glob-invalid",
                    format!("the glob `{pattern}` in stilltrue.toml does not parse"),
                    Some("stilltrue.toml".into()),
                )
            })
            .collect();
        (config, diagnostics)
    }

    pub fn include_set(&self) -> GlobSet {
        let patterns = self
            .include
            .clone()
            .unwrap_or_else(|| DEFAULT_INCLUDE.iter().map(|s| (*s).to_string()).collect());
        build(&patterns)
    }

    /// `exclude` **extends** the defaults rather than replacing them, unlike `include`.
    /// Replacing them would mean that configuring one exclusion silently re-admitted
    /// every changelog in the repository.
    pub fn exclude_set(&self) -> GlobSet {
        let mut builder = GlobSetBuilder::new();
        for pattern in DEFAULT_EXCLUDE {
            add(&mut builder, pattern, true);
        }
        for pattern in &self.exclude {
            add(&mut builder, pattern, false);
        }
        builder.build().unwrap_or_else(|_| GlobSet::empty())
    }
}

fn add(builder: &mut GlobSetBuilder, pattern: &str, case_insensitive: bool) {
    match GlobBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
    {
        Ok(glob) => {
            builder.add(glob);
        }
        Err(error) => eprintln!("stilltrue: warning: bad glob `{pattern}`: {error}"),
    }
}

fn build(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        add(&mut builder, pattern, false);
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// A multi-line parser error, on one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
