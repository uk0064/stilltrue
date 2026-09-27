//! What `resolve` hands to `gate`.
//!
//! A broken claim carries its own evidence, because only the resolver that just failed
//! knows what it looked for and where (ADR-0001).

use std::path::PathBuf;

/// Escape the POSIX ERE metacharacters that can appear in a target name.
pub fn escape_ere(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        // POSIX ERE metacharacters, and only those. `/` is not one of them, and `\/`
        // is undefined in POSIX — GNU tolerates it, which is exactly how an escape
        // that does nothing survives unnoticed.
        if "\\.[]{}()*+?^$|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The pattern history is searched for, to establish whether a claim was ever true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Needle {
    /// `git log --pickaxe-regex -S<pattern> -- <scope>`. Definition-shaped and scoped
    /// (ADR-0005).
    ///
    /// Patterns are POSIX extended regular expressions, which is the dialect git's
    /// pickaxe compiles. `\b` matches nothing there and `\s` is silently read as a
    /// literal `s`, so use `[[:space:]]` and an explicit non-identifier class.
    Regex { pattern: String, scope: Vec<String> },
    /// `git log -S<text> -- <scope>`, no regex. For needles whose exact text is the
    /// evidence and where a word boundary has no portable spelling.
    Literal { text: String, scope: Vec<String> },
    /// `git log --full-history -- :(literal)<path>`, over HEAD's history.
    Path { path: PathBuf },
    /// A Markdown heading whose GitHub slug is `slug`, inside `path`. Searched
    /// case-insensitively, because a slug is lowercased and the heading it came from
    /// was not.
    Heading { slug: String, path: PathBuf },
}

/// What a resolver attaches to a broken claim.
#[derive(Debug, Clone)]
pub struct Evidence {
    /// `None` means no definition-shaped needle can be expressed, which makes the
    /// claim Tier C rather than Tier A (ADR-0005).
    pub needle: Option<Needle>,
    /// Real targets the claim could plausibly have meant.
    pub candidates: Vec<String>,
    /// Human-readable statement of what is broken.
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum Resolution {
    /// The claim still describes the repository.
    True,
    /// The claim provably does not describe the repository.
    Broken(Evidence),
    /// We cannot speak about this claim at all — no manifest, no supported resolver,
    /// nothing to compare against. Never a finding. The reason is what the run report
    /// counts, so a quiet run can say what it did not look at.
    Skip(SkipReason),
    /// The claim is real and does not describe this repository, but we cannot honestly
    /// say it was meant to. Tier C: never a finding, at any flag (ADR-0011).
    Ambiguous(AmbiguityReason),
}

/// Why a claim was not checked. The codes are a public interface: the run report
/// aggregates on them, and an agent reads them rather than prose (ADR-0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipReason {
    /// The runner's manifest is absent — a coverage limitation, not a fault.
    NoManifest,
    /// The manifest exists and could not be read.
    ManifestUnreadable,
    /// The manifest was read and did not parse. Its target list is unknown rather than
    /// empty: reading it as empty made every target it names look missing, and history
    /// then promoted each one to rot.
    ManifestUnparseable,
    /// A runner with no manifest format this tool reads.
    UnsupportedRunner,
    /// No file pins exactly one version of this tool (ADR-0003).
    NoPin,
    /// A pin file exists and could not be read.
    PinUnreadable,
    /// The link leaves the repository.
    OutsideRepository,
    /// The link's target exists and could not be read to look for the anchor.
    TargetUnreadable,
    /// Not a claim about the repository at all.
    NotAClaim,
}

impl SkipReason {
    pub fn code(self) -> &'static str {
        match self {
            SkipReason::NoManifest => "no-manifest",
            SkipReason::ManifestUnreadable => "manifest-unreadable",
            SkipReason::ManifestUnparseable => "manifest-unparseable",
            SkipReason::UnsupportedRunner => "unsupported-runner",
            SkipReason::NoPin => "no-pin",
            SkipReason::PinUnreadable => "pin-unreadable",
            SkipReason::OutsideRepository => "outside-repository",
            SkipReason::TargetUnreadable => "target-unreadable",
            SkipReason::NotAClaim => "not-a-claim",
        }
    }

    /// Whether this skip is an operational failure — something that should have been
    /// read and was not — rather than an expected abstention. A failure makes the run
    /// incomplete (ADR-0019); an abstention is only a coverage limitation.
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            SkipReason::ManifestUnreadable
                | SkipReason::ManifestUnparseable
                | SkipReason::PinUnreadable
                | SkipReason::TargetUnreadable
        )
    }
}

/// Why a broken claim is Tier C. Every variant is a classification rule with its own
/// ADR; the code names it in the run report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AmbiguityReason {
    /// A code language is present that no resolver reads (ADR-0004).
    UnsupportedLanguage,
    /// The namespace computes its attributes with a module-level `__getattr__`.
    DynamicModule,
    /// The namespace is neither a module of this repository nor a class (ADR-0013).
    UnreadableNamespace,
    /// A bare manifest or lockfile name (ADR-0011).
    EcosystemFilename,
    /// The repository assembles names in this family (ADR-0018).
    AssembledName,
    /// An extensionless link a documentation site may resolve in URL space.
    SiteRelativeLink,
    /// The resolver has no definition-shaped needle to search history with (ADR-0005).
    NoNeedle,
}

impl AmbiguityReason {
    pub fn code(self) -> &'static str {
        match self {
            AmbiguityReason::UnsupportedLanguage => "unsupported-language",
            AmbiguityReason::DynamicModule => "dynamic-module",
            AmbiguityReason::UnreadableNamespace => "unreadable-namespace",
            AmbiguityReason::EcosystemFilename => "ecosystem-filename",
            AmbiguityReason::AssembledName => "assembled-name",
            AmbiguityReason::SiteRelativeLink => "site-relative-link",
            AmbiguityReason::NoNeedle => "no-history-needle",
        }
    }
}

impl Needle {
    /// The definition-shaped pattern for a heading with this slug.
    ///
    /// A slug drops case and punctuation, so the pattern puts the parts back in order
    /// separated by "any run of non-identifier characters" — `union-modes` becomes a
    /// heading line reading Union Modes, Union/Modes, or union modes.
    ///
    /// The end of the heading is part of the pattern, and has to be: without it
    /// `#install` matches `## Installation` and the anchor is blamed on the commit
    /// that *added* the heading it does not name. After the last part only trailing
    /// space, an attribute block, and closing `#`s may follow.
    pub fn heading_pattern(slug: &str) -> String {
        let parts: Vec<String> = slug
            .split('-')
            .filter(|part| !part.is_empty())
            .map(escape_ere)
            .collect();
        format!(
            "^#+[[:space:]]*{}[[:space:]]*(\\{{[^}}]*\\}})?[[:space:]]*#*$",
            parts.join("[^A-Za-z0-9]+")
        )
    }
}
