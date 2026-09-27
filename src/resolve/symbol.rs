//! The `Symbol` resolver, gated on the repository being fully supported (ADR-0004).

use crate::claim::Claim;
use crate::repo::Repo;
use crate::resolution::{AmbiguityReason, Evidence, Needle, Resolution};
use crate::resolve::lang;

pub fn resolve(claim: &Claim, repo: &Repo) -> Resolution {
    // Tier C, in the vocabulary CONTEXT.md uses: the claim is real, and a document
    // never says what language a symbol is in (ADR-0004).
    if !repo.symbols_supported() {
        return Resolution::Ambiguous(AmbiguityReason::UnsupportedLanguage);
    }

    let name = lang::normalise_symbol(&claim.text);
    let definitions = repo.definitions();
    let active = lang::active(repo);

    if definitions.contains(name) {
        return Resolution::True;
    }

    // A builtin of *any* language this tool reads, not only the ones with files here.
    // ADR-0004's premise is that a document never says what language a symbol is in,
    // and a name the language itself provides is not defined in any repository and not
    // missing from one either. flask is wholly Python and its docs name `fetch()`; the
    // resolver that knows `fetch` is free had nothing to read, so the claim was
    // reported as rot in a repository that could never have defined it.
    if lang::resolvers()
        .iter()
        .any(|resolver| resolver.builtins().contains(&name))
    {
        return Resolution::True;
    }

    // A dotted name lives inside a namespace, and judging it means being able to read
    // that namespace. A module of this repository we can read, and so is a class whose
    // name the index already holds. An instance we cannot, because that needs type
    // inference and this tool does none — and neither can we read a namespace that
    // belongs to something else entirely.
    //
    // CamelCase used to stand in for "a class", by the same Python convention the bare
    // `snake_case` rule rested on before ADR-0017 removed it, and it failed the same
    // way: `Object`, `JSON`, `Array` and `Math` are CamelCase namespaces that no Python
    // repository defines, and a project's documentation names them the moment it
    // mentions the browser it ships alongside. Asking the index is not a convention at
    // all — it is the question the rule was always about.
    if let Some(namespace) = namespace_of(&claim.text) {
        // A module that computes its own attributes is no more readable than an
        // instance: the name a document writes appears nowhere the index can reach.
        if repo.dynamic_modules().contains(namespace) {
            return Resolution::Ambiguous(AmbiguityReason::DynamicModule);
        }
        if !repo.modules().contains(namespace) && !repo.namespaces().contains(namespace) {
            return Resolution::Ambiguous(AmbiguityReason::UnreadableNamespace);
        }
    }

    Resolution::Broken(Evidence {
        needle: Some(Needle::Regex {
            // One needle per active language, joined: a document never says which one
            // the symbol belongs to, so history is searched for all of them.
            pattern: active
                .iter()
                .map(|resolver| resolver.needle(name))
                .collect::<Vec<_>>()
                .join("|"),
            scope: active.iter().flat_map(|r| r.scope()).collect(),
        }),
        // Builtins belong in the index — `print` is not missing from anything — but
        // not in "did you mean". Seventy of them swamp the candidate set, which stops
        // it being small enough to enumerate and leaves nothing near enough to guess.
        candidates: definitions
            .iter()
            .filter(|name| {
                !active
                    .iter()
                    .any(|resolver| resolver.builtins().contains(&name.as_str()))
            })
            .cloned()
            .collect(),
        message: format!("symbol `{}` is not defined anywhere", claim.text),
    })
}

/// The namespace a dotted symbol is written against, if it has one.
fn namespace_of(text: &str) -> Option<&str> {
    let text = text
        .strip_suffix("()")
        .unwrap_or(text)
        .trim_start_matches('~');
    if let Some((first, _)) = text.split_once("::") {
        return Some(first);
    }
    text.split_once('.').map(|(first, _)| first)
}
