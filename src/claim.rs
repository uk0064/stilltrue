//! The typed claims a document makes about the repository.
//!
//! See `CONTEXT.md` for the vocabulary: a claim is later found *true* or *broken*,
//! never "resolved".

use std::ops::Range;
use std::path::PathBuf;

/// What kind of statement a claim makes. Decided once, in `extract`, by first-match
/// precedence, and never revisited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimKind {
    Path,
    /// A runner and its raw arguments. Per-runner meaning (which argument names the
    /// target, whether `run` is a prefix) belongs to the resolver, which is the only
    /// thing that knows how to look the target up and how to search history for it.
    Command {
        runner: String,
        args: Vec<String>,
    },
    EnvVar,
    Symbol,
    Version {
        tool: String,
        version: String,
    },
    Link {
        target: String,
        anchor: Option<String>,
    },
    /// Not a claim about the repository but a finding about the document: a
    /// suppression marker that turned out to cover nothing (ADR-0016).
    Suppression,
}

impl ClaimKind {
    /// The segment used in rule ids: `stilltrue/<claim-type>/<tier>`.
    pub fn slug(&self) -> &'static str {
        match self {
            ClaimKind::Path => "path",
            ClaimKind::Command { .. } => "command",
            ClaimKind::EnvVar => "env",
            ClaimKind::Symbol => "symbol",
            ClaimKind::Version { .. } => "version",
            ClaimKind::Link { .. } => "link",
            ClaimKind::Suppression => "suppression",
        }
    }
}

/// A statement a document makes about the repository, with an exact span.
#[derive(Debug, Clone)]
pub struct Claim {
    pub kind: ClaimKind,
    /// The raw text of the claim, delimiters stripped.
    pub text: String,
    /// The document the claim was made in, as given to `extract`.
    pub file: PathBuf,
    /// 1-based.
    pub line: usize,
    /// 1-based, counted in Unicode code points, and pointing at the claim text rather
    /// than at any delimiter.
    pub column: usize,
    /// One past the last character of the claim, in the same units as `line`/`column`.
    /// A code span the author wrapped ends on a later line than it started.
    pub end_line: usize,
    pub end_column: usize,
    /// Byte span of the claim text within the document.
    pub span: Range<usize>,
}
