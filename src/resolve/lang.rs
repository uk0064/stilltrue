//! The polyglot seam. One resolver ships in v1; the trait is what proves the seam.

use std::collections::{BTreeMap, BTreeSet};

use crate::repo::Repo;

/// A language whose definitions we can harvest.
pub trait LanguageResolver: Sync {
    /// The file extensions this resolver claims. A language is rarely one extension:
    /// TypeScript is `.ts`, `.tsx`, `.mts` and the JavaScript it is a superset of.
    fn extensions(&self) -> &'static [&'static str];

    /// The grammar to parse a file with. TSX and TypeScript are different grammars for
    /// the same language, so this takes the extension rather than being a constant.
    fn language(&self, extension: &str) -> tree_sitter::Language;

    /// A tree-sitter query whose every capture is a defined name.
    ///
    /// This has to describe the same set as `needle` below. When the two disagree, a
    /// name in the gap resolves Broken and is then promoted to Tier A by its own
    /// history — see the commit that closed exactly that hole for Python.
    fn query(&self) -> &'static str;

    /// The definition-shaped history needle for a name (ADR-0005).
    fn needle(&self, name: &str) -> String;

    /// Names that are defined everywhere and meant nowhere: what the language itself
    /// provides, and what it reserves by convention. `print` is not defined in any
    /// repository and is not missing from one either, and nobody writing
    /// `Widget.render()` meant `self`. They belong in the index — leaving them out
    /// would break the symmetry with `needle` — but never in "did you mean".
    ///
    /// Required rather than defaulted to `&[]`. Both resolvers already answer it, so
    /// the default was unreachable, and a mutation sweep can say nothing about code
    /// nothing calls. It is the wrong default anyway: a new language that said nothing
    /// here would get an index missing every builtin it has, which is the asymmetry the
    /// paragraph above warns about, arrived at silently. Adding a language should have
    /// to answer this.
    fn builtins(&self) -> &'static [&'static str];

    /// The pathspec history searches within.
    fn scope(&self) -> Vec<String> {
        self.extensions()
            .iter()
            .map(|ext| format!("*.{ext}"))
            .collect()
    }

    /// Whether this repository contains any file this resolver can read.
    fn present(&self, repo: &Repo) -> bool {
        repo.files().iter().any(|file| {
            file.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| self.extensions().contains(&ext))
        })
    }

    /// A query whose every capture is a name that can *hold* other names: a class, an
    /// interface, an enum. Judging `Foo.bar` means being able to read `Foo`, and this
    /// is the question "can we?" — not a guess from its capitalisation (ADR-0013).
    ///
    /// Unlike `query`, this one has no needle to match: it is never resolved against,
    /// only consulted, so it is exempt from the symmetry rule above.
    fn namespace_query(&self) -> &'static str;

    /// Every namespace this resolver can read in the repository.
    fn namespaces(&self, repo: &Repo) -> BTreeSet<String> {
        self.captures(repo, self.namespace_query())
    }

    /// Every definition name this resolver can find in the repository.
    fn definitions(&self, repo: &Repo) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self.builtins().iter().map(|b| (*b).to_string()).collect();
        out.extend(self.captures(repo, self.query()));
        out
    }

    /// Every name captured by one query across every file this resolver reads.
    fn captures(&self, repo: &Repo, query_source: &'static str) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        // One compiled query per grammar, not per file. Compiling a query is the
        // expensive part of using one — tree-sitter analyses every pattern against the
        // grammar — and compiling it inside this loop made the 500-document,
        // 5,000-file benchmark take eight seconds warm, against a budget of one.
        let mut queries: BTreeMap<&str, tree_sitter::Query> = BTreeMap::new();
        let mut parsers: BTreeMap<&str, tree_sitter::Parser> = BTreeMap::new();
        let mut cursor = tree_sitter::QueryCursor::new();
        for file in repo.files() {
            let Some(ext) = file.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let Some(ext) = self
                .extensions()
                .iter()
                .copied()
                .find(|known| *known == ext)
            else {
                continue;
            };
            let Ok(source) = std::fs::read_to_string(repo.root().join(file)) else {
                continue;
            };
            if !queries.contains_key(ext) {
                let language = self.language(ext);
                let Ok(query) = tree_sitter::Query::new(&language, query_source) else {
                    // A query that will not compile is a programming error, and a silent
                    // empty index would make every symbol in the repository broken.
                    eprintln!("stilltrue: warning: could not compile the query for {ext}");
                    return BTreeSet::new();
                };
                let mut parser = tree_sitter::Parser::new();
                if parser.set_language(&language).is_err() {
                    continue;
                }
                queries.insert(ext, query);
                parsers.insert(ext, parser);
            }
            let (Some(query), Some(parser)) = (queries.get(ext), parsers.get_mut(ext)) else {
                continue;
            };
            let Some(tree) = parser.parse(source.as_bytes(), None) else {
                continue;
            };
            let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
            while let Some(m) = tree_sitter::StreamingIterator::next(&mut matches) {
                for capture in m.captures {
                    if let Ok(text) = capture.node.utf8_text(source.as_bytes()) {
                        out.insert(text.to_string());
                    }
                }
            }
        }
        out
    }
}

/// Every shipped resolver. Adding one here is the whole of "support a new language".
pub fn resolvers() -> &'static [&'static (dyn LanguageResolver + Sync)] {
    &[&Python, &TypeScript]
}

/// The resolvers that have something to read in this repository.
pub fn active(repo: &Repo) -> Vec<&'static (dyn LanguageResolver + Sync)> {
    resolvers()
        .iter()
        .copied()
        .filter(|resolver| resolver.present(repo))
        .collect()
}

/// Extensions that mean "there is code here" for the purposes of ADR-0004.
const CODE_EXTENSIONS: &[&str] = &[
    "py", "rs", "go", "ts", "tsx", "js", "jsx", "java", "rb", "c", "cc", "cpp", "h", "hpp", "cs",
    "kt", "swift", "php", "scala", "ex", "exs", "clj", "hs", "ml", "lua", "dart",
];

/// Below this many files, a language is a stray script rather than a language the
/// repository is written in.
const MIN_LANGUAGE_FILES: usize = 2;

pub fn python() -> Python {
    Python
}

/// Whether every code language present in the repository has a shipped resolver.
///
/// A document never says what language a symbol is in, so if anything unsupported is
/// present we say nothing at all (ADR-0004).
pub fn fully_supported(repo: &Repo) -> bool {
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for file in repo.files() {
        let Some(ext) = file.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if let Some(known) = CODE_EXTENSIONS.iter().find(|e| **e == ext) {
            *counts.entry(known).or_default() += 1;
        }
    }
    let supported = |ext: &str| resolvers().iter().any(|r| r.extensions().contains(&ext));
    // At least one file we can actually read: "every language present is supported" is
    // vacuously true of a repository with no code, and an empty index would make every
    // symbol claim broken.
    let has_supported = counts.keys().any(|ext| supported(ext));
    let unsupported = counts
        .iter()
        .any(|(ext, count)| !supported(ext) && *count >= MIN_LANGUAGE_FILES);
    has_supported && !unsupported
}

pub struct Python;

impl LanguageResolver for Python {
    fn extensions(&self) -> &'static [&'static str] {
        &["py", "pyi"]
    }

    fn language(&self, _extension: &str) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn query(&self) -> &'static str {
        // The other half of `needle` below, and the two must describe the same set.
        // The needle matches an assignment at any indentation, so the index must too:
        // when the index is the narrower of the pair, every name in the gap resolves
        // Broken and is then promoted to Tier A by its own history. Parameters are
        // included because a black-formatted signature puts each one on its own line,
        // where the needle reads it as an assignment.
        r#"
            (function_definition name: (identifier) @name)
            (class_definition name: (identifier) @name)
            (assignment left: (identifier) @name)
            (assignment left: (attribute attribute: (identifier) @name))
            (parameters (identifier) @name)
            (default_parameter name: (identifier) @name)
            (typed_parameter (identifier) @name)
            (typed_default_parameter name: (identifier) @name)
            (list_splat_pattern (identifier) @name)
            (dictionary_splat_pattern (identifier) @name)
            (import_from_statement name: (dotted_name (identifier) @name))
            (import_from_statement name: (aliased_import alias: (identifier) @name))
            (import_statement name: (dotted_name (identifier) @name))
            (import_statement name: (aliased_import alias: (identifier) @name))
        "#
    }

    fn builtins(&self) -> &'static [&'static str] {
        &[
            // Not builtins, but reserved by convention and captured as parameters on
            // every method in the repository, which made them outrank real candidates.
            "self",
            "cls",
            "abs",
            "all",
            "any",
            "bool",
            "bytes",
            "callable",
            "chr",
            "classmethod",
            "compile",
            "complex",
            "dict",
            "dir",
            "divmod",
            "enumerate",
            "eval",
            "exec",
            "filter",
            "float",
            "format",
            "frozenset",
            "getattr",
            "globals",
            "hasattr",
            "hash",
            "hex",
            "id",
            "input",
            "int",
            "isinstance",
            "issubclass",
            "iter",
            "len",
            "list",
            "locals",
            "map",
            "max",
            "memoryview",
            "min",
            "next",
            "object",
            "oct",
            "open",
            "ord",
            "pow",
            "print",
            "property",
            "range",
            "repr",
            "reversed",
            "round",
            "set",
            "setattr",
            "slice",
            "sorted",
            "staticmethod",
            "str",
            "sum",
            "super",
            "tuple",
            "type",
            "vars",
            "zip",
            "AttributeError",
            "Exception",
            "ImportError",
            "IndexError",
            "KeyError",
            "NotImplementedError",
            "OSError",
            "RuntimeError",
            "StopIteration",
            "TypeError",
            "ValueError",
        ]
    }

    fn namespace_query(&self) -> &'static str {
        "(class_definition name: (identifier) @name)"
    }

    fn needle(&self, name: &str) -> String {
        // POSIX ERE: no `\s`, no `\b`. A definition line always has a delimiter after
        // the name (`(` for a function, `(` or `:` for a class), so requiring one
        // non-identifier character stands in for the word boundary.
        let name = crate::resolution::escape_ere(name);
        format!(
            "^[[:space:]]*(def|class)[[:space:]]+{name}[^A-Za-z0-9_]|^[[:space:]]*{name}[[:space:]]*="
        )
    }
}

pub struct TypeScript;

impl LanguageResolver for TypeScript {
    fn extensions(&self) -> &'static [&'static str] {
        &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"]
    }

    fn language(&self, extension: &str) -> tree_sitter::Language {
        // TSX and TypeScript are separate grammars: the TSX one reads `<div/>`, the
        // TypeScript one reads `<T>` as a type argument, and neither reads both.
        match extension {
            "tsx" | "jsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        }
    }

    fn query(&self) -> &'static str {
        // Symmetric with `needle`, for the same reason Python's is. `const x =` and a
        // class member are both definitions here, and parameters count because a
        // prettier-formatted signature puts each on its own line.
        //
        // An enum's members are definitions too. Leaving them out made `Mode.On` —
        // defined in the very file being scanned — a Tier A finding blamed on the
        // commit that introduced it. Both spellings are needed: `On = 1` is an
        // `enum_assignment`, a bare `Off` is a plain `property_identifier`.
        //
        // Comments inside this string must use tree-sitter's `;`, not `//`: a `//`
        // line is query syntax, fails to compile, and the resolver then warns and
        // switches TypeScript off entirely.
        r#"
            (function_declaration name: (identifier) @name)
            (generator_function_declaration name: (identifier) @name)
            (class_declaration name: (type_identifier) @name)
            (interface_declaration name: (type_identifier) @name)
            (type_alias_declaration name: (type_identifier) @name)
            (enum_declaration name: (identifier) @name)
            (enum_assignment name: (property_identifier) @name)
            (enum_body (property_identifier) @name)
            (variable_declarator name: (identifier) @name)
            (method_definition name: (property_identifier) @name)
            (public_field_definition name: (property_identifier) @name)
            (property_signature name: (property_identifier) @name)
            (pair key: (property_identifier) @name)
            (required_parameter pattern: (identifier) @name)
            (optional_parameter pattern: (identifier) @name)
            (import_specifier name: (identifier) @name)
            (import_specifier alias: (identifier) @name)
            (namespace_import (identifier) @name)
            (import_clause (identifier) @name)
        "#
    }

    fn builtins(&self) -> &'static [&'static str] {
        &[
            "Array",
            "BigInt",
            "Boolean",
            "Date",
            "Error",
            "Function",
            "Infinity",
            "JSON",
            "Map",
            "Math",
            "NaN",
            "Number",
            "Object",
            "Promise",
            "Proxy",
            "Reflect",
            "RegExp",
            "Set",
            "String",
            "Symbol",
            "WeakMap",
            "WeakSet",
            "clearInterval",
            "clearTimeout",
            "console",
            "decodeURIComponent",
            "document",
            "encodeURIComponent",
            "exports",
            "fetch",
            "globalThis",
            "isNaN",
            "module",
            "parseFloat",
            "parseInt",
            "process",
            "require",
            "setInterval",
            "setTimeout",
            "structuredClone",
            "undefined",
            "window",
        ]
    }

    fn namespace_query(&self) -> &'static str {
        r#"
            (class_declaration name: (type_identifier) @name)
            (abstract_class_declaration name: (type_identifier) @name)
            (interface_declaration name: (type_identifier) @name)
            (enum_declaration name: (identifier) @name)
            (type_alias_declaration name: (type_identifier) @name)
            (internal_module name: (identifier) @name)
        "#
    }

    fn needle(&self, name: &str) -> String {
        let name = crate::resolution::escape_ere(name);
        format!(
            "^[[:space:]]*(export[[:space:]]+)?(default[[:space:]]+)?(async[[:space:]]+)?\
             (function|class|interface|type|enum|const|let|var)[[:space:]]+{name}[^A-Za-z0-9_$]\
             |^[[:space:]]*{name}[[:space:]]*[:(=]"
        )
    }
}

/// Strip a trailing `()`, then take the final segment of a dotted or `::`-separated
/// name.
pub fn normalise_symbol(text: &str) -> &str {
    let text = text.strip_suffix("()").unwrap_or(text);
    // Sphinx writes `:attr:`~Context.params`` to render the last segment only.
    let text = text.trim_start_matches('~');
    let text = text.rsplit("::").next().unwrap_or(text);
    text.rsplit('.').next().unwrap_or(text)
}
