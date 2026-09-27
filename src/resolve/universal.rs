//! Resolvers that need no language support. These carry the agent-config wedge,
//! because instruction files overwhelmingly make command and path claims.

use std::path::{Path, PathBuf};

use crate::claim::Claim;
use crate::repo::{Repo, normalise};
use crate::resolution::{AmbiguityReason, Evidence, Needle, Resolution, SkipReason};

/// A `Path` claim is true if the target exists relative to the document's directory or
/// to the repository root.
pub fn path(claim: &Claim, repo: &Repo) -> Resolution {
    // A leading `/` spells "from the repository root". Stripping it is not
    // cosmetic: an absolute path normalises to nothing, so the claim could never be
    // true, and the raw text then reached git as `:(literal)/x` — an invalid pathspec
    // that makes git refuse the whole search.
    let target = Path::new(claim.text.trim_start_matches('/'));
    for candidate in anchored(&claim.file, target) {
        if repo.exists(&candidate) {
            return Resolution::True;
        }
    }

    if is_bare_ecosystem_filename(&claim.text) {
        return Resolution::Ambiguous(AmbiguityReason::EcosystemFilename);
    }

    let normalised = normalise(target).unwrap_or_else(|| target.to_path_buf());
    Resolution::Broken(Evidence {
        needle: Some(Needle::Path {
            path: normalised.clone(),
        }),
        candidates: same_basename(repo, &normalised),
        message: format!("path `{}` does not exist", claim.text),
    })
}

/// Manifests, lockfiles, and the tool configuration a document tells its reader to
/// create. Documentation names these in the abstract — "add it to your
/// `requirements.txt`", "set this in `poetry.toml`" — as readily as it names its own,
/// and the token is identical either way (ADR-0011). `Makefile` and `Dockerfile` are
/// deliberately absent: documents name those far more often as the repository's own.
const ECOSYSTEM_FILENAMES: &[&str] = &[
    ".env",
    ".npmrc",
    ".pre-commit-config.yaml",
    "mypy.ini",
    "poetry.toml",
    "ruff.toml",
    "tox.ini",
    "uv.lock",
    "Cargo.lock",
    "Cargo.toml",
    "Gemfile",
    "Gemfile.lock",
    "Pipfile",
    "Pipfile.lock",
    "build.gradle",
    "composer.json",
    "composer.lock",
    "go.mod",
    "go.sum",
    "package-lock.json",
    "package.json",
    "pnpm-lock.yaml",
    "poetry.lock",
    "pom.xml",
    "pyproject.toml",
    "requirements.txt",
    "setup.cfg",
    "setup.py",
    // Our own config file is the same class as everyone else's: a document naming it is
    // telling its reader to create one.
    "stilltrue.toml",
    "tsconfig.json",
    "yarn.lock",
];

/// A claim naming one of those with no directory component at all. Saying you mean a
/// path — `./requirements.txt`, `config/requirements.txt` — opts back into gating.
fn is_bare_ecosystem_filename(text: &str) -> bool {
    !text.contains('/') && ECOSYSTEM_FILENAMES.contains(&text)
}

/// The document-relative and root-relative readings of a target, in that order.
pub fn anchored(document: &Path, target: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = document.parent()
        && let Some(joined) = normalise(&dir.join(target))
    {
        out.push(joined);
    }
    if let Some(rooted) = normalise(target)
        && !out.contains(&rooted)
    {
        out.push(rooted);
    }
    out
}

/// Files elsewhere in the repository sharing the target's file name — the shape a
/// moved file takes.
fn same_basename(repo: &Repo, target: &Path) -> Vec<String> {
    let Some(name) = target.file_name() else {
        return Vec::new();
    };
    repo.files()
        .iter()
        .filter(|p| p.file_name() == Some(name))
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// The same, spelled the way a link in `document` would have to spell it.
///
/// A `Path` claim resolves against the document *or* the repository root, so a
/// repository-relative candidate is a true answer for one. A `Link` does not: it is
/// file-relative and strict, matching GitHub's rendering. Offering
/// `docs/new/x.md` to a document already inside `docs/` names a file that is not
/// there, so the suggestion was wrong on its own terms — and `--fix` wrote it in,
/// leaving a link still broken but no longer carrying any history, which turned a
/// reported finding into a silent one.
fn same_basename_from(repo: &Repo, document: &Path, target: &Path) -> Vec<String> {
    let dir = document.parent().unwrap_or(Path::new(""));
    same_basename(repo, target)
        .iter()
        .map(|candidate| relative_to(dir, Path::new(candidate)))
        .collect()
}

/// Spell `target` as a path relative to the directory `from`, the way a link has to.
fn relative_to(from: &Path, target: &Path) -> String {
    let mut from = from.components().peekable();
    let mut target = target.components().peekable();
    while let (Some(a), Some(b)) = (from.peek(), target.peek()) {
        if a != b {
            break;
        }
        from.next();
        target.next();
    }
    // Every directory left unmatched is one the link has to climb out of.
    let up = from.count();
    // Joined with `/` rather than collected into a `PathBuf`, which would use the
    // platform separator: a link is a URL reference and reads `/` on every platform,
    // so a Windows run must not offer `new\x.md`.
    let down: Vec<String> = target
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut out = String::new();
    for _ in 0..up {
        out.push_str("../");
    }
    out.push_str(&down.join("/"));
    out
}

/// Extensions that never count as evidence for an `EnvVar` claim: a document is a
/// tracked file, so without this exclusion every claim satisfies itself.
const DOCUMENT_EXTENSIONS: &[&str] = &["md", "markdown", "mdx", "mdc", "rst", "txt"];

const DOTENV_EXAMPLES: &[&str] = &[".env.example", ".env.sample", ".env.template"];

/// This tool's own configuration, which is never evidence for anything in it.
const CONFIG_FILE: &str = "stilltrue.toml";

/// An `EnvVar` claim is true if the literal name appears in code or config — never in
/// prose.
pub fn env_var(claim: &Claim, repo: &Repo) -> Resolution {
    let name = claim.text.as_str();
    if repo.env_names().contains(name) {
        return Resolution::True;
    }
    if assembled(name, repo) {
        return Resolution::Ambiguous(AmbiguityReason::AssembledName);
    }

    Resolution::Broken(Evidence {
        // A literal: POSIX ERE, which git's pickaxe uses, has no portable word
        // boundary, and an all-caps underscored name is distinctive enough without one.
        needle: Some(Needle::Literal {
            text: name.to_string(),
            scope: document_exclusions(),
        }),
        candidates: Vec::new(),
        message: format!("env var `{name}` is not read anywhere in the repository"),
    })
}

/// Whether this repository builds variable names in the family this one belongs to.
///
/// A name that ends in an underscore is not a variable — nothing reads `POETRY_`. It is
/// what the code wrote down while constructing a longer name, whether by concatenation
/// (`"POETRY_" + "_".join(...)`) or by matching a family with a pattern
/// (`POETRY_REPOSITORIES_(?P<name>[A-Z_]+)_URL`). Either way the repository is saying
/// that names in that family are assembled, and an assembled name cannot be looked up.
///
/// Only reached once a claim has already failed, so this turns a finding into silence
/// and never the other way round — the direction this tool errs in. The cost is that
/// genuine rot in an assembled family goes unreported, which is the same trade as
/// ADR-0004 and ADR-0013: when the question needs something this tool cannot see,
/// say nothing. See ADR-0018.
fn assembled(name: &str, repo: &Repo) -> bool {
    name.char_indices()
        .filter(|(_, c)| *c == '_')
        .any(|(i, _)| repo.env_names().contains(&name[..=i]))
}

/// Git pathspecs excluding every extension the index refuses as evidence.
///
/// The needle and the index have to describe the same set of files. When the needle is
/// the broader of the pair, a name that only ever appeared in prose is "proved" to have
/// once existed and an example variable is promoted to Tier A — click documents
/// `WEB_RUN_RELOAD` as a worked example, and its only history is a docs reorganisation.
fn document_exclusions() -> Vec<String> {
    // The needle and the index have to describe one set of files. When the needle is
    // the broader of the pair, a name that only ever appeared in an excluded file is
    // proved by that file's own history.
    let mut out: Vec<String> = DOCUMENT_EXTENSIONS
        .iter()
        .map(|ext| format!(":(exclude)*.{ext}"))
        .collect();
    out.push(format!(":(exclude){CONFIG_FILE}"));
    out
}

pub fn is_evidence(file: &Path) -> bool {
    // The tool's own configuration is not code. A variable named there is being named
    // so it can be *ignored*, and counting that as "read somewhere" would make the
    // config satisfy its own claims — the same hole the prose exclusion below closes,
    // and one that silences a finding by the wrong mechanism entirely.
    if file == Path::new(CONFIG_FILE) {
        return false;
    }
    if DOTENV_EXAMPLES
        .iter()
        .any(|n| file.file_name().is_some_and(|f| f == *n))
    {
        return true;
    }
    match file.extension().and_then(|e| e.to_str()) {
        Some(ext) => !DOCUMENT_EXTENSIONS.contains(&ext),
        None => true,
    }
}

/// Files that pin exactly one version. Range constraints are ignored entirely
/// (ADR-0003).
fn pin_file(tool: &str) -> &'static [&'static str] {
    match tool {
        "Node" | "Node.js" => &[".nvmrc", ".node-version"],
        "Python" => &[".python-version"],
        "Rust" => &["rust-toolchain.toml", "rust-toolchain"],
        "Go" => &["go.mod"],
        _ => &[],
    }
}

/// A `Version` claim is compared only against an exact pin (ADR-0003).
pub fn version(repo: &Repo, tool: &str, stated: &str) -> Resolution {
    // A pin that exists and cannot be read is a failed read, not an absent pin: the
    // run report has to be able to say it looked and could not see.
    let mut unreadable = false;
    for name in pin_file(tool) {
        let path = Path::new(name);
        if !repo.exists(path) {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(repo.root().join(path)) else {
            unreadable = true;
            continue;
        };
        let Some(pinned) = pinned_version(name, &contents) else {
            continue;
        };
        return if compatible(stated, &pinned) {
            Resolution::True
        } else {
            Resolution::Broken(Evidence {
                needle: Some(Needle::Path {
                    path: path.to_path_buf(),
                }),
                candidates: vec![pinned.clone()],
                message: format!("{tool} {stated} contradicts `{name}`, which pins {pinned}"),
            })
        };
    }
    Resolution::Skip(if unreadable {
        SkipReason::PinUnreadable
    } else {
        SkipReason::NoPin
    })
}

/// Whether a pin file's contents name one version rather than a moving target.
///
/// ADR-0003 consults only files that pin exactly one version, and a channel
/// (`stable`, `nightly-2026-01-01`), an alias (`lts/iron`, `system`) and an
/// implementation (`pypy3.10-7.3.15`) are none of them: they name whatever that
/// stream points at today, so prose stating a number cannot contradict one. Read as a
/// pin, `channel = "stable"` — the commonest rust-toolchain.toml there is — made every
/// Rust version stated in a README rot, blamed on the commit that created the file.
fn is_version_number(pinned: &str) -> bool {
    let pinned = pinned.trim_start_matches('v');
    !pinned.is_empty()
        && pinned.starts_with(|c: char| c.is_ascii_digit())
        && pinned.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn pinned_version(name: &str, contents: &str) -> Option<String> {
    let pinned = read_pin(name, contents)?;
    is_version_number(&pinned).then_some(pinned)
}

fn read_pin(name: &str, contents: &str) -> Option<String> {
    match name {
        // See the note in `cargo_aliases`: `str::parse` rejects what `from_str` accepts,
        // so this silently never read a Rust toolchain pin at all.
        "rust-toolchain.toml" => toml::from_str::<toml::Value>(contents)
            .ok()?
            .get("toolchain")?
            .get("channel")?
            .as_str()
            .map(str::to_string),
        "go.mod" => contents
            .lines()
            .find_map(|l| l.trim().strip_prefix("go "))
            .map(|v| v.trim().to_string()),
        _ => {
            let first = contents.lines().find(|l| !l.trim().is_empty())?;
            Some(first.trim().trim_start_matches('v').to_string())
        }
    }
}

/// The stated version is compatible when it is a prefix of the pin at a component
/// boundary: "Node 24" is satisfied by a pin of `24.3.1`, "Node 18" is not.
fn compatible(stated: &str, pinned: &str) -> bool {
    let pinned = pinned.trim_start_matches('v');
    let stated: Vec<&str> = stated.split('.').collect();
    let pinned: Vec<&str> = pinned.split('.').collect();
    stated.len() <= pinned.len() && stated.iter().zip(&pinned).all(|(a, b)| a == b)
}

/// A `Link` is file-relative only, strictly, matching GitHub's rendering.
/// The file a documentation site would serve for a link target.
///
/// mkdocs and Docusaurus both drop the extension from their URLs, so httpx's
/// `../advanced/transports` is served from `advanced/transports.md`. Checking only for
/// a literal file calls every extensionless link in such a site broken.
fn served(repo: &Repo, resolved: PathBuf) -> Option<PathBuf> {
    if repo.exists(&resolved) {
        return Some(resolved);
    }
    if resolved.extension().is_some() {
        return None;
    }
    [
        resolved.with_extension("md"),
        resolved.join("index.md"),
        resolved.join("README.md"),
    ]
    .into_iter()
    .find(|candidate| repo.exists(candidate))
}

pub fn link(claim: &Claim, repo: &Repo, target: &str, anchor: Option<&str>) -> Resolution {
    let dir = claim
        .file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let Some(resolved) = normalise(&dir.join(target)) else {
        return Resolution::Skip(SkipReason::OutsideRepository);
    };

    let Some(resolved) = served(repo, resolved) else {
        // An extensionless target we cannot locate is Tier C. A documentation site
        // resolves links in URL space, where a page served at `/async/` puts `..` at
        // the site root; file space says something different, and which is meant
        // depends on configuration we do not read.
        if Path::new(target).extension().is_none() {
            return Resolution::Ambiguous(AmbiguityReason::SiteRelativeLink);
        }
        let resolved = normalise(&dir.join(target)).unwrap_or_default();
        return Resolution::Broken(Evidence {
            needle: Some(Needle::Path {
                path: resolved.clone(),
            }),
            candidates: same_basename_from(repo, &claim.file, &resolved),
            message: format!("link target `{target}` does not exist"),
        });
    };

    let Some(anchor) = anchor else {
        return Resolution::True;
    };
    let Ok(contents) = std::fs::read_to_string(repo.root().join(&resolved)) else {
        return Resolution::Skip(SkipReason::TargetUnreadable);
    };
    let headings = headings(&contents);
    if headings.iter().any(|h| h == anchor) {
        return Resolution::True;
    }
    Resolution::Broken(Evidence {
        // A heading *is* definition-shaped — `^#+ Title` — so an anchor can reach Tier
        // A after all. Retitling a heading is the commonest documentation refactor
        // there is, and this was the one rot the tool could detect and never report.
        needle: Some(Needle::Heading {
            slug: anchor.to_string(),
            path: resolved.clone(),
        }),
        candidates: headings,
        message: format!("link `{target}` has no heading `#{anchor}`"),
    })
}

/// Heading slugs under GitHub's rules: lowercase, spaces to hyphens, drop anything
/// that is not alphanumeric, hyphen, or underscore.
fn headings(contents: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in contents.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // `{#id}` names an anchor wherever it appears, not only on a heading: mkdocs'
        // `[](){#id}` idiom defines a standalone one, and pydantic uses it throughout
        // its API pages.
        for id in explicit_ids(line) {
            out.push(id);
        }
        let Some(rest) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let title = rest.trim_start_matches('#').trim();
        if title.is_empty() {
            continue;
        }
        // `## Title {#explicit-id}` — mkdocs' attr_list and pandoc both let a heading
        // name its own anchor, and that id is what the rendered page uses. GitHub
        // ignores the attribute and slugs the whole line, so both spellings count:
        // the document is correct under either renderer.
        let title = match title.strip_suffix('}').and_then(|t| t.rsplit_once("{#")) {
            Some((before, _)) => before.trim(),
            None => title,
        };
        if title.is_empty() {
            continue;
        }
        let slug: String = title
            .to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect();
        if !slug.is_empty() {
            out.push(slug);
        }
        // GitHub slugs the *rendered* heading, so `_Black_` contributes `black`. But an
        // underscore inside a name is not emphasis and does survive. Telling the two
        // apart needs an inline parse; accepting both spellings does not, and erring
        // towards more anchors errs towards silence.
        if title.contains(['_', '*', '`']) {
            let stripped: String = title.chars().filter(|c| !"_*`".contains(*c)).collect();
            let slug: String = stripped
                .to_lowercase()
                .chars()
                .filter_map(|c| match c {
                    ' ' => Some('-'),
                    c if c.is_alphanumeric() || c == '-' => Some(c),
                    _ => None,
                })
                .collect();
            if !slug.is_empty() {
                out.push(slug);
            }
        }
    }
    out
}

/// Every `{#id}` attribute-list anchor on a line.
fn explicit_ids(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some((_, after)) = rest.split_once("{#") {
        match after.split_once('}') {
            Some((id, tail)) => {
                let id = id.trim();
                if !id.is_empty() && id.chars().all(|c| !c.is_whitespace()) {
                    out.push(id.to_lowercase());
                }
                rest = tail;
            }
            None => break,
        }
    }
    out
}
