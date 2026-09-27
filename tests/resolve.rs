//! Seam: `resolve` decides whether a claim is true, broken, or unspeakable — and a
//! broken claim carries the evidence `gate` needs (ADR-0001).

use std::path::Path;

use stilltrue::claim::{Claim, ClaimKind};
use stilltrue::repo::Repo;
use stilltrue::resolution::{AmbiguityReason, Needle, Resolution, SkipReason};
use stilltrue::resolve;

/// Build a throwaway repository from `(path, contents)` pairs.
fn repo_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, contents) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    let repo = Repo::new(dir.path());
    (dir, repo)
}

/// A claim of the given kind, made in `document`.
fn claim(kind: ClaimKind, text: &str, document: &str) -> Claim {
    Claim {
        kind,
        text: text.to_string(),
        file: Path::new(document).to_path_buf(),
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 1 + text.chars().count(),
        span: 0..text.len(),
    }
}

fn broken(resolution: &Resolution) -> &stilltrue::resolution::Evidence {
    match resolution {
        Resolution::Broken(evidence) => evidence,
        other => panic!("expected Broken, got {other:?}"),
    }
}

#[test]
fn a_path_that_exists_relative_to_the_repository_root_is_true() {
    let (_dir, repo) = repo_with(&[("docs/setup.md", "")]);
    let c = claim(ClaimKind::Path, "docs/setup.md", "README.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::True));
}

#[test]
fn a_path_that_exists_relative_to_the_document_is_true() {
    // Either hit resolves it; the ambiguity is settled in favour of silence.
    let (_dir, repo) = repo_with(&[("docs/guide/setup.md", "")]);
    let c = claim(ClaimKind::Path, "setup.md", "docs/guide/README.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::True));
}

#[test]
fn a_missing_path_is_broken_and_carries_a_path_history_needle() {
    let (_dir, repo) = repo_with(&[("README.md", "")]);
    let c = claim(ClaimKind::Path, "docs/gone.md", "README.md");
    let evidence = broken(&resolve::resolve(&c, &repo)).clone();
    assert_eq!(
        evidence.needle,
        Some(Needle::Path {
            path: Path::new("docs/gone.md").to_path_buf()
        })
    );
}

#[test]
fn a_moved_file_is_offered_as_a_candidate() {
    let (_dir, repo) = repo_with(&[("docs/guide/setup.md", ""), ("README.md", "")]);
    let c = claim(ClaimKind::Path, "setup.md", "README.md");
    let evidence = broken(&resolve::resolve(&c, &repo)).clone();
    assert!(
        evidence
            .candidates
            .iter()
            .any(|p| p == "docs/guide/setup.md"),
        "got {:?}",
        evidence.candidates
    );
}

fn command(text: &str, document: &str) -> Claim {
    let mut tokens = text.split_whitespace();
    let runner = tokens.next().unwrap().to_string();
    let args = tokens.map(str::to_string).collect();
    claim(ClaimKind::Command { runner, args }, text, document)
}

#[test]
fn a_make_target_that_exists_is_true() {
    let (_dir, repo) = repo_with(&[("Makefile", "demo:\n\techo hi\n")]);
    assert!(matches!(
        resolve::resolve(&command("make demo", "README.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_missing_make_target_is_broken_with_a_definition_shaped_needle() {
    let (_dir, repo) = repo_with(&[("Makefile", "seed:\n\techo\n\nserve:\n\techo\n")]);
    let evidence = broken(&resolve::resolve(&command("make demo", "README.md"), &repo)).clone();
    assert_eq!(
        evidence.needle,
        Some(Needle::Regex {
            // POSIX ERE: git's pickaxe reads `\s` as a literal `s`.
            pattern: "^demo[[:space:]]*:".to_string(),
            scope: vec!["Makefile".to_string()],
        })
    );
    assert_eq!(
        evidence.candidates,
        vec!["seed".to_string(), "serve".to_string()]
    );
}

#[test]
fn a_command_with_no_manifest_anywhere_is_unspeakable() {
    // Missing manifest means we cannot prove anything, so we say nothing.
    let (_dir, repo) = repo_with(&[("README.md", "")]);
    assert!(matches!(
        resolve::resolve(&command("make demo", "README.md"), &repo),
        Resolution::Skip(SkipReason::NoManifest)
    ));
}

#[test]
fn a_nested_manifest_anchors_a_nested_document() {
    // The monorepo case: the nested package.json is checked, not the root one.
    let (_dir, repo) = repo_with(&[
        ("package.json", r#"{"scripts":{"root-only":"echo"}}"#),
        ("packages/a/package.json", r#"{"scripts":{"build":"tsc"}}"#),
    ]);
    let c = command("pnpm run build", "packages/a/README.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::True));

    let root = command("pnpm run build", "README.md");
    assert!(matches!(
        resolve::resolve(&root, &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_builtin_subcommand_always_resolves() {
    let (_dir, repo) = repo_with(&[("Cargo.toml", "[package]\nname=\"x\"\n")]);
    assert!(matches!(
        resolve::resolve(&command("cargo build", "README.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn every_runner_family_knows_its_own_builtins() {
    // Only cargo's arm of this table was covered. The others matter for different
    // reasons, so both are asserted as `True` rather than merely "not reported".
    //
    // For npm the cost of losing the arm is immediate: `install` is not a script in
    // anyone's package.json, so the claim falls through to the script list and comes
    // back as a finding. `npm install` appears in most JavaScript READMEs ever written.
    //
    // For go and uv the arm is the difference between resolving a claim and abstaining
    // on it. Both are silent today, which is why nothing noticed, but an abstention is
    // a statement that the claim was not checked — and silence reached by not looking
    // is not the same answer as silence reached by looking.
    let (_dir, repo) = repo_with(&[
        ("package.json", "{\"scripts\":{\"build\":\"tsc\"}}"),
        ("go.mod", "module example.com/x\n"),
        ("pyproject.toml", "[project]\nname = \"x\"\n"),
    ]);

    for text in [
        "npm install",
        "npm ci",
        "pnpm add",
        "yarn publish",
        "go test",
        "go mod",
        "uv sync",
        "uvx run",
    ] {
        assert!(
            matches!(
                resolve::resolve(&command(text, "README.md"), &repo),
                Resolution::True
            ),
            "{text}: a builtin subcommand resolves on its own, with no manifest entry",
        );
    }

    // The near-miss. A runner that reads a manifest still judges what is not a builtin,
    // or the arm above would be indistinguishable from "npm is never checked".
    assert!(
        matches!(
            resolve::resolve(&command("npm run build", "README.md"), &repo),
            Resolution::True
        ),
        "a declared script resolves",
    );
    assert!(
        matches!(
            resolve::resolve(&command("npm run nope", "README.md"), &repo),
            Resolution::Broken { .. }
        ),
        "and one that is neither builtin nor declared is broken",
    );
}

#[test]
fn a_just_recipe_is_read_from_the_justfile() {
    let (_dir, repo) = repo_with(&[("justfile", "release:\n    echo\n")]);
    assert!(matches!(
        resolve::resolve(&command("just release", "README.md"), &repo),
        Resolution::True
    ));
    assert!(matches!(
        resolve::resolve(&command("just nope", "README.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn an_env_var_named_in_code_is_true() {
    let (_dir, repo) = repo_with(&[("app.py", "key = os.environ['GEMINI_API_KEY']\n")]);
    let c = claim(ClaimKind::EnvVar, "GEMINI_API_KEY", "CLAUDE.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::True));
}

#[test]
fn an_env_var_named_only_in_documents_is_broken() {
    // Without excluding documents, every EnvVar claim satisfies itself: the document
    // that makes the claim is a tracked file.
    let (_dir, repo) = repo_with(&[
        ("CLAUDE.md", "Set `GEMINI_API_KEY`.\n"),
        ("docs/setup.md", "Also GEMINI_API_KEY.\n"),
        ("app.py", "print('hi')\n"),
    ]);
    let c = claim(ClaimKind::EnvVar, "GEMINI_API_KEY", "CLAUDE.md");
    let evidence = broken(&resolve::resolve(&c, &repo)).clone();
    // A literal needle: POSIX ERE has no portable word boundary. Its scope excludes
    // documents for the same reason the index does — otherwise a name that only ever
    // appeared in prose is proved by its own document's history — and excludes this
    // tool's own configuration, where a name appears in order to be ignored.
    assert_eq!(
        evidence.needle,
        Some(Needle::Literal {
            text: "GEMINI_API_KEY".to_string(),
            scope: vec![
                ":(exclude)*.md".to_string(),
                ":(exclude)*.markdown".to_string(),
                ":(exclude)*.mdx".to_string(),
                ":(exclude)*.mdc".to_string(),
                ":(exclude)*.rst".to_string(),
                ":(exclude)*.txt".to_string(),
                ":(exclude)stilltrue.toml".to_string(),
            ]
        })
    );
}

#[test]
fn an_env_var_in_a_dotenv_example_is_true() {
    let (_dir, repo) = repo_with(&[(".env.example", "GEMINI_API_KEY=\n"), ("app.py", "")]);
    let c = claim(ClaimKind::EnvVar, "GEMINI_API_KEY", "CLAUDE.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::True));
}

fn version(tool: &str, v: &str) -> Claim {
    claim(
        ClaimKind::Version {
            tool: tool.to_string(),
            version: v.to_string(),
        },
        &format!("{tool} {v}"),
        "README.md",
    )
}

#[test]
fn a_version_matching_the_pin_is_true() {
    let (_dir, repo) = repo_with(&[(".nvmrc", "24\n")]);
    assert!(matches!(
        resolve::resolve(&version("Node", "24"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_version_contradicting_the_pin_is_broken() {
    let (_dir, repo) = repo_with(&[(".nvmrc", "24\n")]);
    let evidence = broken(&resolve::resolve(&version("Node", "18"), &repo)).clone();
    assert_eq!(
        evidence.needle,
        Some(Needle::Path {
            path: Path::new(".nvmrc").to_path_buf()
        })
    );
}

#[test]
fn a_version_with_only_a_range_constraint_is_unspeakable() {
    // ADR-0003: ranges are ignored entirely, so `engines` alone yields nothing.
    let (_dir, repo) = repo_with(&[("package.json", r#"{"engines":{"node":">=20"}}"#)]);
    assert!(matches!(
        resolve::resolve(&version("Node", "18"), &repo),
        Resolution::Skip(SkipReason::NoPin)
    ));
}

#[test]
fn a_pin_that_is_not_a_version_number_pins_nothing() {
    // ADR-0003: only a file that pins exactly one *version* is consulted. A channel,
    // an alias, and a family each name a moving target, and prose cannot contradict
    // one — `channel = "stable"` is the commonest rust-toolchain.toml there is, and
    // reading it as a pin made every stated Rust version in a README rot.
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "Rust",
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"stable\"\n",
            "1.70",
        ),
        (
            "Rust",
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"nightly-2026-01-01\"\n",
            "1.70",
        ),
        ("Rust", "rust-toolchain", "beta\n", "1.70"),
        ("Node", ".nvmrc", "lts/*\n", "18"),
        ("Node", ".nvmrc", "lts/iron\n", "18"),
        ("Node", ".node-version", "node\n", "18"),
        ("Python", ".python-version", "pypy3.10-7.3.15\n", "3.9"),
        ("Python", ".python-version", "system\n", "3.9"),
    ];
    for (tool, file, contents, stated) in cases {
        let (_dir, repo) = repo_with(&[(file, contents)]);
        assert!(
            matches!(
                resolve::resolve(&version(tool, stated), &repo),
                Resolution::Skip(SkipReason::NoPin)
            ),
            "{file} holding {contents:?} is not a pin, so {tool} {stated} cannot contradict it"
        );
    }
}

#[test]
fn a_numeric_pin_is_still_a_pin() {
    // The near-miss for the rule above: every spelling a real pin takes must survive
    // it, or the fix for the false positive buys silence everywhere instead.
    let cases: &[(&str, &str, &str, &str)] = &[
        ("Node", ".nvmrc", "v24.3.1\n", "18"),
        ("Node", ".nvmrc", "24\n", "18"),
        (
            "Rust",
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"1.98.0\"\n",
            "1.70",
        ),
        ("Go", "go.mod", "module x\n\ngo 1.23\n", "1.19"),
    ];
    for (tool, file, contents, stale) in cases {
        let (_dir, repo) = repo_with(&[(file, contents)]);
        assert!(
            matches!(
                resolve::resolve(&version(tool, stale), &repo),
                Resolution::Broken(_)
            ),
            "{file} holding {contents:?} is a pin, so {tool} {stale} contradicts it"
        );
    }
}

fn link(target: &str, anchor: Option<&str>, document: &str) -> Claim {
    claim(
        ClaimKind::Link {
            target: target.to_string(),
            anchor: anchor.map(str::to_string),
        },
        target,
        document,
    )
}

#[test]
fn a_file_relative_link_that_exists_is_true() {
    let (_dir, repo) = repo_with(&[("docs/guide/setup.md", "# Install\n")]);
    let c = link("./setup.md", None, "docs/guide/README.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::True));
}

#[test]
fn a_root_relative_link_written_in_a_nested_document_is_broken() {
    // Links are strictly file-relative, matching GitHub's rendering.
    let (_dir, repo) = repo_with(&[("docs/setup.md", "# Install\n")]);
    let c = link("docs/setup.md", None, "docs/guide/README.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::Broken(_)));
}

#[test]
fn a_link_suggestion_is_spelled_the_way_the_link_would_have_to_spell_it() {
    // A link is file-relative and strict, so a repository-relative candidate
    // names a file that is not where the link would look. `--fix` wrote one in and
    // left the link broken *and* historyless, which is a reported finding turned
    // silent — the worst direction this tool can move in.
    let (_dir, repo) = repo_with(&[
        ("docs/new/x.md", "# X\n"),
        ("other/y.md", "# Y\n"),
        ("top.md", "# T\n"),
    ]);

    let moved = broken(&resolve::resolve(
        &link("old/x.md", None, "docs/a.md"),
        &repo,
    ))
    .clone();
    assert_eq!(moved.candidates, vec!["new/x.md".to_string()]);

    // A candidate the link has to climb out of its own directory to reach.
    let sideways = broken(&resolve::resolve(&link("y.md", None, "docs/a.md"), &repo)).clone();
    assert_eq!(sideways.candidates, vec!["../other/y.md".to_string()]);

    // And from a document at the root there is nothing to climb.
    let rooted = broken(&resolve::resolve(&link("top.md", None, "docs/a.md"), &repo)).clone();
    assert_eq!(rooted.candidates, vec!["../top.md".to_string()]);
}

#[test]
fn a_path_suggestion_stays_repository_relative() {
    // The near-miss: a `Path` resolves against the document *or* the repository
    // root, so a repository-relative candidate is a true answer for one and must not
    // be rewritten into the link spelling.
    let (_dir, repo) = repo_with(&[("docs/new/x.md", "# X\n")]);
    let c = claim(ClaimKind::Path, "old/x.md", "docs/a.md");
    let evidence = broken(&resolve::resolve(&c, &repo)).clone();
    assert_eq!(evidence.candidates, vec!["docs/new/x.md".to_string()]);
}

#[test]
fn a_link_anchor_must_match_a_real_heading() {
    let (_dir, repo) = repo_with(&[("docs/setup.md", "# Getting Started\n\n## Install It\n")]);
    let good = link("./setup.md", Some("install-it"), "docs/README.md");
    assert!(matches!(resolve::resolve(&good, &repo), Resolution::True));

    let bad = link("./setup.md", Some("nope"), "docs/README.md");
    assert!(matches!(
        resolve::resolve(&bad, &repo),
        Resolution::Broken(_)
    ));
}

fn symbol(text: &str) -> Claim {
    claim(ClaimKind::Symbol, text, "CLAUDE.md")
}

fn env(text: &str) -> Claim {
    claim(ClaimKind::EnvVar, text, "CLAUDE.md")
}

#[test]
fn a_python_definition_that_exists_is_true() {
    let (_dir, repo) = repo_with(&[("app.py", "def fold_events(x):\n    return x\n")]);
    assert!(matches!(
        resolve::resolve(&symbol("fold_events()"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_dotted_symbol_resolves_on_its_final_segment() {
    let (_dir, repo) = repo_with(&[(
        "graph.py",
        "class Graph:\n    def query(self):\n        pass\n",
    )]);
    assert!(matches!(
        resolve::resolve(&symbol("graph.query"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_missing_python_symbol_is_broken_with_a_definition_shaped_needle() {
    let (_dir, repo) = repo_with(&[("app.py", "def other():\n    pass\n")]);
    let evidence = broken(&resolve::resolve(&symbol("fold_events()"), &repo)).clone();
    match evidence.needle {
        Some(Needle::Regex { pattern, scope }) => {
            assert!(pattern.contains("fold_events"), "got {pattern}");
            assert!(
                pattern.contains("def") && pattern.contains("class"),
                "got {pattern}"
            );
            assert_eq!(scope, vec!["*.py".to_string(), "*.pyi".to_string()]);
        }
        other => panic!("expected a scoped regex needle, got {other:?}"),
    }
}

// The corpus sweep found 244 Tier A symbol findings across ten real repositories,
// nearly all false. The cause is an asymmetry: the history needle in `lang.rs` matches
// an assignment at any indentation, while the definition index only knew module-level
// assignments. Every name in that gap resolves Broken, then the pickaxe proves it
// "once existed", and a false claim is promoted to the highest-confidence tier.
// These three shapes are the gap, taken from click and llm.

#[test]
fn a_class_attribute_is_a_definition() {
    let (_dir, repo) = repo_with(&[(
        "core.py",
        "class Context:\n    ignore_unknown_options: bool = False\n",
    )]);
    assert!(matches!(
        resolve::resolve(&symbol("ignore_unknown_options"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_keyword_parameter_is_a_definition() {
    let (_dir, repo) = repo_with(&[(
        "core.py",
        "def option(expose_value=True):\n    return expose_value\n",
    )]);
    assert!(matches!(
        resolve::resolve(&symbol("expose_value"), &repo),
        Resolution::True
    ));
}

#[test]
fn an_instance_attribute_is_a_definition() {
    let (_dir, repo) = repo_with(&[(
        "core.py",
        "class Context:\n    def __init__(self):\n        self.params = {}\n",
    )]);
    assert!(matches!(
        resolve::resolve(&symbol("~Context.params"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_namespace_is_readable_only_if_it_is_actually_in_the_index() {
    // ADR-0013 judges a dotted symbol inside a namespace it can read, and took
    // CamelCase as standing in for "a class". It does not: `Object`, `JSON`, `Array`
    // and `Math` are CamelCase namespaces from outside any Python repository, and a
    // Python project's documentation names them whenever it talks about the browser
    // it ships alongside. Every one was judged, and judged broken.
    let (_dir, repo) = repo_with(&[(
        "app.py",
        "class Widget:\n    def draw(self):\n        return 1\n",
    )]);
    for foreign in [
        "Object.keys()",
        "JSON.parse()",
        "Array.isArray()",
        "Math.floor()",
        "Decimal.quantize()",
    ] {
        assert!(
            matches!(
                resolve::resolve(&symbol(foreign), &repo),
                Resolution::Ambiguous(AmbiguityReason::UnreadableNamespace)
            ),
            "{foreign}: a namespace this repository has never defined is not readable"
        );
    }

    // The near-miss: a class this repository does define is readable, so a missing
    // attribute on it is still judged.
    assert!(
        matches!(
            resolve::resolve(&symbol("Widget.render()"), &repo),
            Resolution::Broken(_)
        ),
        "a class in the index stays judgeable"
    );
    assert!(matches!(
        resolve::resolve(&symbol("Widget.draw()"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_builtin_of_any_language_we_read_is_missing_from_nothing() {
    // flask's docs/patterns/javascript.rst names `fetch()`. flask is wholly Python, so
    // symbol claims are in scope and the TypeScript resolver — which knows `fetch` is a
    // builtin — is not active, having no files to read.
    //
    // ADR-0004's premise settles it: a document never says what language a symbol is
    // in. If a name is provided by any language this tool can read, then "it is not
    // defined anywhere" is a claim we are in no position to make.
    let (_dir, repo) = repo_with(&[("app.py", "def handler():\n    return 1\n")]);
    for builtin in ["fetch()", "JSON.parse()", "console.log()", "Promise.all()"] {
        assert!(
            !matches!(
                resolve::resolve(&symbol(builtin), &repo),
                Resolution::Broken(_)
            ),
            "{builtin} is a builtin somewhere, so it is missing from nothing"
        );
    }
    // Python's own, in the same repository, for the case that already worked.
    assert!(matches!(
        resolve::resolve(&symbol("print()"), &repo),
        Resolution::True
    ));

    // The near-miss: a name no language provides is still judged.
    assert!(matches!(
        resolve::resolve(&symbol("handler()"), &repo),
        Resolution::True
    ));
    assert!(
        matches!(
            resolve::resolve(&symbol("absent_thing()"), &repo),
            Resolution::Broken(_)
        ),
        "an ordinary missing name is still a finding"
    );
}

#[test]
fn the_linters_own_configuration_is_not_evidence() {
    // `stilltrue.toml` is a tracked file and not prose, so the environment scan counted
    // it as code. Naming a variable there — which is how you ask for it to be *ignored*
    // — therefore made it resolve as read, silencing it by the wrong mechanism and
    // hiding a genuine hole: a variable in the config satisfies its own claim, exactly
    // the way a document did before prose was excluded from the scan.
    let (_dir, repo) = repo_with(&[
        ("app.py", "def run():\n    pass\n"),
        (
            "stilltrue.toml",
            "# APP_SECRET is named here and read nowhere\n[ignore]\nenv = [\"OTHER_VAR\"]\n",
        ),
    ]);
    assert!(
        matches!(
            resolve::resolve(&env("APP_SECRET"), &repo),
            Resolution::Broken(_)
        ),
        "a name that appears only in the tool's own config is not read anywhere"
    );

    // And the needle carries the same exclusion, or the index and the history describe
    // different sets and every name in the gap comes back as rot.
    let evidence = broken(&resolve::resolve(&env("APP_SECRET"), &repo)).clone();
    let scope = match evidence.needle.as_ref().expect("a needle") {
        Needle::Literal { scope, .. } | Needle::Regex { scope, .. } => scope.clone(),
        other => panic!("unexpected needle: {other:?}"),
    };
    assert!(
        scope.iter().any(|s| s.contains("stilltrue.toml")),
        "the history scope must exclude the config too: {scope:?}"
    );
}

#[test]
fn a_variable_whose_prefix_the_code_builds_with_is_ambiguous() {
    // poetry documents POETRY_VIRTUALENVS_CREATE and never writes it down: every one
    // of its variables is assembled, `env = "POETRY_" + "_".join(...)`. The literal
    // the code does write is the prefix, and a prefix in the index is the repository
    // saying it constructs names in that family.
    let (_dir, repo) = repo_with(&[(
        "config.py",
        "def env_for(keys):\n    return \"POETRY_\" + \"_\".join(k.upper() for k in keys)\n",
    )]);
    assert!(
        matches!(
            resolve::resolve(&env("POETRY_VIRTUALENVS_CREATE"), &repo),
            Resolution::Ambiguous(AmbiguityReason::AssembledName)
        ),
        "a name in an assembled family cannot be judged"
    );

    // A regular expression that matches a family is the same statement.
    let (_dir2, re) = repo_with(&[(
        "config.py",
        "import re\n\npattern = re.compile(r\"POETRY_REPOSITORIES_(?P<name>[A-Z_]+)_URL\")\n",
    )]);
    assert!(matches!(
        resolve::resolve(&env("POETRY_REPOSITORIES_PYPI_URL"), &re),
        Resolution::Ambiguous(AmbiguityReason::AssembledName)
    ));

    // The near-misses. A repository that writes its variables out in full still has
    // every one of them judged, and an unrelated family is not covered by someone
    // else's prefix.
    let (_dir3, plain) = repo_with(&[(
        "app.py",
        "import os\n\nKEY = os.environ[\"ORCHARD_API_KEY\"]\n",
    )]);
    assert!(matches!(
        resolve::resolve(&env("ORCHARD_API_KEY"), &plain),
        Resolution::True
    ));
    assert!(
        matches!(
            resolve::resolve(&env("ORCHARD_SECRET"), &plain),
            Resolution::Broken(_)
        ),
        "a full name is not a prefix, so nothing here is assembled"
    );
    assert!(
        matches!(
            resolve::resolve(&env("UNRELATED_THING"), &plain),
            Resolution::Broken(_)
        ),
        "another family's prefix says nothing about this one"
    );
}

#[test]
fn a_module_that_computes_its_own_attributes_is_not_readable() {
    // click's `__init__.py` provides `get_text_stream` from a module-level
    // `__getattr__` that matches on the name as a string and imports a private
    // function to serve it. The name exists, and nothing static can see it — the index
    // holds `_get_text_stream`, which is not what anyone writes in a document.
    //
    // A module is a namespace this repository can read only while its attributes are
    // written down. Once it computes them, it is no more readable than an instance.
    let (_dir, repo) = repo_with(&[
        (
            "pkg/__init__.py",
            "from .utils import helper as helper\n\n\ndef __getattr__(name):\n    if name == \"lazy\":\n        from .utils import _lazy\n\n        return _lazy\n    raise AttributeError(name)\n",
        ),
        (
            "pkg/utils.py",
            "def helper():\n    pass\n\n\ndef _lazy():\n    pass\n",
        ),
    ]);
    assert!(
        matches!(
            resolve::resolve(&symbol("pkg.lazy"), &repo),
            Resolution::Ambiguous(AmbiguityReason::DynamicModule)
        ),
        "a lazily provided name cannot be judged"
    );
    assert!(
        matches!(
            resolve::resolve(&symbol("pkg.anything_at_all"), &repo),
            Resolution::Ambiguous(AmbiguityReason::DynamicModule)
        ),
        "nor can any other attribute of that module"
    );

    // The near-miss: a module that writes its names down stays readable, and a name
    // that is genuinely absent from it is still judged.
    let (_dir2, plain) = repo_with(&[
        ("mod/__init__.py", "from .inner import helper as helper\n"),
        ("mod/inner.py", "def helper():\n    pass\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("mod.helper"), &plain),
        Resolution::True
    ));
    assert!(
        matches!(
            resolve::resolve(&symbol("mod.missing"), &plain),
            Resolution::Broken(_)
        ),
        "a static module is still judged"
    );
}

#[test]
fn a_conventional_parameter_name_is_never_a_suggestion() {
    // `self` is defined in every Python repository that has a class, so the definition
    // index has to know it — that is what keeps the index and the needle describing one
    // set. It is never what a reader meant, though, and a wrong suggestion costs more
    // trust than no suggestion buys.
    let (_dir, repo) = repo_with(&[(
        "widget.py",
        "class Widget:\n    def draw(self, cls):\n        return 1\n",
    )]);
    let evidence = broken(&resolve::resolve(&symbol("Widget.render()"), &repo)).clone();
    assert!(
        evidence.candidates.contains(&"draw".to_string()),
        "the real method must still be offered: {:?}",
        evidence.candidates
    );
    for noise in ["self", "cls"] {
        assert!(
            !evidence.candidates.contains(&noise.to_string()),
            "`{noise}` must not be suggested: {:?}",
            evidence.candidates
        );
    }

    // But it stays in the index, so a claim on it resolves rather than going broken.
    assert!(matches!(
        resolve::resolve(&symbol("Widget.self"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_symbol_is_unspeakable_when_an_unsupported_language_is_present() {
    // ADR-0004: a document never says what language a symbol is in.
    let (_dir, repo) = repo_with(&[
        ("app.py", "def other():\n    pass\n"),
        ("main.go", "package main\n"),
        ("util.go", "package main\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("fold_events()"), &repo),
        Resolution::Ambiguous(AmbiguityReason::UnsupportedLanguage)
    ));
}

#[test]
fn a_single_stray_file_does_not_disqualify_a_repository() {
    // The near-miss for the rule above: one loose script is not a language.
    let (_dir, repo) = repo_with(&[
        ("app.py", "def other():\n    pass\n"),
        ("scripts/hook.go", "package main\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("fold_events()"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_symbol_is_unspeakable_when_the_repository_has_no_python_at_all() {
    let (_dir, repo) = repo_with(&[("README.md", "")]);
    assert!(matches!(
        resolve::resolve(&symbol("fold_events()"), &repo),
        Resolution::Ambiguous(AmbiguityReason::UnsupportedLanguage)
    ));
}

#[test]
fn the_shared_indexes_are_built_once_per_repository() {
    // Building these per claim, rather than once and sharing them, took a 500-document
    // repository from 0.2s to 36s. Pointer identity tests the invariant directly, with
    // none of a wall-clock assertion's flakiness.
    let (_dir, repo) = repo_with(&[("app.py", "def a():\n    pass\n"), ("b.py", "X = 1\n")]);
    assert!(std::ptr::eq(repo.env_names(), repo.env_names()));
    assert!(std::ptr::eq(repo.definitions(), repo.definitions()));
    assert!(std::ptr::eq(repo.files(), repo.files()));
}

#[test]
fn the_git_directory_is_never_evidence() {
    // `.git` is walked only because hidden files must be, and its contents must never
    // satisfy a claim about the repository's own code.
    let (_dir, repo) = repo_with(&[("app.py", "print(1)\n")]);
    std::fs::create_dir_all(repo.root().join(".git")).unwrap();
    std::fs::write(
        repo.root().join(".git/COMMIT_EDITMSG"),
        "add SECRET_TOKEN\n",
    )
    .unwrap();
    let c = claim(ClaimKind::EnvVar, "SECRET_TOKEN", "CLAUDE.md");
    assert!(matches!(resolve::resolve(&c, &repo), Resolution::Broken(_)));
}

// ADR-0011. `pip-tools` says `requirements.txt` twenty-five times in its README,
// meaning the reader's file. Neither it nor `setup.py` exists in that repository and
// both once did, so Tier A fired on a document that is not wrong.

fn bare_path(text: &str) -> Claim {
    claim(ClaimKind::Path, text, "README.md")
}

#[test]
fn a_broken_bare_ecosystem_filename_is_ambiguous() {
    let (_dir, repo) = repo_with(&[("app.py", "x = 1\n")]);
    assert!(matches!(
        resolve::resolve(&bare_path("requirements.txt"), &repo),
        Resolution::Ambiguous(AmbiguityReason::EcosystemFilename)
    ));
}

#[test]
fn a_qualified_ecosystem_filename_is_still_gated() {
    // The near-miss: the escape hatch is to say you mean a path, the same one bare
    // references already have.
    let (_dir, repo) = repo_with(&[("app.py", "x = 1\n")]);
    assert!(matches!(
        resolve::resolve(&bare_path("config/requirements.txt"), &repo),
        Resolution::Broken(_)
    ));
    assert!(matches!(
        resolve::resolve(&bare_path("./requirements.txt"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn an_ecosystem_filename_that_exists_is_still_true() {
    let (_dir, repo) = repo_with(&[("requirements.txt", "click\n")]);
    assert!(matches!(
        resolve::resolve(&bare_path("requirements.txt"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_bare_filename_outside_the_closed_list_is_still_gated() {
    // `Makefile` and `Dockerfile` are deliberately absent from the list: documentation
    // names those far more often as the repository's own than as the reader's.
    let (_dir, repo) = repo_with(&[("app.py", "x = 1\n")]);
    assert!(matches!(
        resolve::resolve(&bare_path("Makefile"), &repo),
        Resolution::Broken(_)
    ));
}

fn link_claim(target: &str, document: &str) -> Claim {
    claim(
        ClaimKind::Link {
            target: target.to_string(),
            anchor: None,
        },
        target,
        document,
    )
}

#[test]
fn an_extensionless_link_resolves_through_the_markdown_extension() {
    // mkdocs and Docusaurus drop the extension from their URLs, so a link written
    // `advanced/transports` is served from `advanced/transports.md`.
    let (_dir, repo) = repo_with(&[("docs/advanced/transports.md", "# Transports\n")]);
    assert!(matches!(
        resolve::resolve(&link_claim("advanced/transports", "docs/async.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn an_extensionless_link_also_resolves_a_directory_index() {
    let (_dir, repo) = repo_with(&[("docs/advanced/index.md", "# Advanced\n")]);
    assert!(matches!(
        resolve::resolve(&link_claim("advanced", "docs/async.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn an_extensionless_link_that_cannot_be_located_is_ambiguous() {
    // httpx writes `../advanced/transports` from `docs/async.md`. In file space that
    // is `advanced/transports`, which does not exist; in mkdocs URL space the page is
    // served at `/async/`, so `..` climbs to the site root and the link is correct.
    // Which one is meant depends on site configuration we do not read, so we abstain.
    let (_dir, repo) = repo_with(&[("docs/advanced/transports.md", "# Transports\n")]);
    assert!(matches!(
        resolve::resolve(
            &link_claim("../advanced/transports", "docs/async.md"),
            &repo
        ),
        Resolution::Ambiguous(AmbiguityReason::SiteRelativeLink)
    ));
}

#[test]
fn a_link_naming_a_file_outright_is_still_broken() {
    // The near-miss: abstention is for links whose target we cannot locate, not for
    // links that name a file and are wrong.
    let (_dir, repo) = repo_with(&[("docs/advanced/transports.md", "# Transports\n")]);
    assert!(matches!(
        resolve::resolve(&link_claim("advanced/missing.md", "docs/async.md"), &repo),
        Resolution::Broken(_)
    ));
}

// A dotted symbol names something inside a namespace, and judging it means knowing
// what that namespace is. A module we can read; a class we can read; an instance we
// cannot, because that needs type inference. httpx documents `response.next` and
// `netrc.netrc()`, click documents `project.scripts` — a pyproject table.

#[test]
fn an_attribute_on_an_instance_is_ambiguous() {
    // `payload`, not `next`: a builtin would resolve before the namespace rule is
    // reached, and this is a test of the namespace rule.
    let (_dir, repo) = repo_with(&[("app.py", "def handler(response):\n    return response\n")]);
    assert!(matches!(
        resolve::resolve(&symbol("response.payload"), &repo),
        Resolution::Ambiguous(AmbiguityReason::UnreadableNamespace)
    ));
}

#[test]
fn a_namespace_from_outside_the_repository_is_ambiguous() {
    // Nothing here imports `netrc`, so it is neither a module of this repository nor a
    // name in its namespace — we cannot read it, so we do not judge what is inside it.
    let (_dir, repo) = repo_with(&[("app.py", "x = 1\n")]);
    assert!(matches!(
        resolve::resolve(&symbol("netrc.retrieve()"), &repo),
        Resolution::Ambiguous(AmbiguityReason::UnreadableNamespace)
    ));
}

#[test]
fn an_attribute_on_a_class_is_still_judged() {
    // The near-miss: a CamelCase namespace is a class by Python convention, and a
    // class's attributes are in the index.
    let (_dir, repo) = repo_with(&[(
        "core.py",
        "class Context:\n    def __init__(self):\n        self.params = {}\n",
    )]);
    assert!(matches!(
        resolve::resolve(&symbol("Context.missing"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_name_inside_a_module_of_this_repository_is_still_judged() {
    // The other near-miss: `graph` is a module here, so we can read it and say so.
    let (_dir, repo) = repo_with(&[("graph.py", "def query():\n    pass\n")]);
    assert!(matches!(
        resolve::resolve(&symbol("graph.missing"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_package_directory_counts_as_a_module() {
    let (_dir, repo) = repo_with(&[
        ("click/__init__.py", "from .core import Command\n"),
        ("click/core.py", "class Command:\n    pass\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("click.missing"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_nested_directory_is_not_an_importable_namespace() {
    // poetry has `tests/fixtures/project_with_multi_constraints_dependency/project`,
    // which made `project.maintainers` — a pyproject table — look judgeable.
    // `import project` would not find it; only top-level modules are namespaces.
    let (_dir, repo) = repo_with(&[
        ("app.py", "x = 1\n"),
        ("tests/fixtures/sample/project/__init__.py", "\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("project.maintainers"), &repo),
        Resolution::Ambiguous(AmbiguityReason::UnreadableNamespace)
    ));
}

#[test]
fn a_package_under_a_source_root_is_importable() {
    // The near-miss: click lives at `src/click/`, which is a normal layout.
    let (_dir, repo) = repo_with(&[
        ("src/click/__init__.py", "\n"),
        ("src/click/core.py", "class Command:\n    pass\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("click.missing"), &repo),
        Resolution::Broken(_)
    ));
}

// llm's justfile writes its recipes as `@cog:` and `@test *options:`. In just, a
// leading `@` makes the recipe quiet and a trailing word list is its parameters —
// both are recipes, and both were being read as something to skip.

#[test]
fn a_quiet_just_recipe_is_a_recipe() {
    let (_dir, repo) = repo_with(&[("justfile", "# Rebuild docs\n@cog:\n  uv run cog -r\n")]);
    assert!(matches!(
        resolve::resolve(&command("just cog", "docs/contributing.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_just_recipe_with_parameters_is_a_recipe() {
    let (_dir, repo) = repo_with(&[("justfile", "@test *options:\n  uv run pytest\n")]);
    assert!(matches!(
        resolve::resolve(&command("just test", "docs/contributing.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_just_attribute_line_is_not_a_recipe() {
    // The near-miss: `[group: 'dev']` is an attribute, and it contains a colon.
    let (_dir, repo) = repo_with(&[("justfile", "[group: 'dev']\nbuild:\n  cargo build\n")]);
    assert!(matches!(
        resolve::resolve(&command("just group", "docs/contributing.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_just_assignment_is_not_a_recipe() {
    let (_dir, repo) = repo_with(&[(
        "justfile",
        "export RUST_LOG := \"debug\"\nbuild:\n  cargo build\n",
    )]);
    assert!(matches!(
        resolve::resolve(&command("just export", "docs/contributing.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn our_own_config_file_is_the_same_class_as_everyone_elses() {
    // The README says "`stilltrue.toml` in the repository root, if you want one" —
    // which is an instruction to the reader, not a claim about this repository.
    let (_dir, repo) = repo_with(&[("app.py", "x = 1\n")]);
    assert!(matches!(
        resolve::resolve(&bare_path("stilltrue.toml"), &repo),
        Resolution::Ambiguous(AmbiguityReason::EcosystemFilename)
    ));
}

// A Path claim is true relative to the document *or* the repository root. A
// leading `/` is the natural spelling of the second, and it must never reach git —
// `:(literal)/x` is an invalid pathspec, and git refuses the entire search.

#[test]
fn a_root_relative_path_is_read_from_the_repository_root() {
    let (_dir, repo) = repo_with(&[("docs/index.md", "# x\n")]);
    assert!(matches!(
        resolve::resolve(&bare_path("/docs/index.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_root_relative_path_never_becomes_an_absolute_pathspec() {
    let (_dir, repo) = repo_with(&[("app.py", "x = 1\n")]);
    let evidence = broken(&resolve::resolve(&bare_path("/ghost.md"), &repo)).clone();
    match evidence.needle {
        Some(Needle::Path { path }) => {
            assert!(
                !path.is_absolute(),
                "git cannot take {path:?} as a pathspec"
            )
        }
        other => panic!("expected a path needle, got {other:?}"),
    }
}

// llm writes every one of its just recipes quiet (`@cog:`), and defines aliases for
// some. The parser learned about `@` already; the needle did not, so every command
// finding in such a repository degraded from rot to an invisible lie.

#[test]
fn a_just_alias_is_a_recipe() {
    // Must land with the needle fix, not after it: `just mypy` resolves Broken today
    // and is silent only because the needle denies it history. Fix one without the
    // other and this becomes a Tier A false positive.
    let (_dir, repo) = repo_with(&[(
        "justfile",
        "@typecheck:\n  mypy .\n\nalias mypy := typecheck\n",
    )]);
    assert!(matches!(
        resolve::resolve(&command("just mypy", "README.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_quiet_just_recipe_is_matched_by_the_history_needle() {
    let (_dir, repo) = repo_with(&[("justfile", "@build:\n  cargo build\n")]);
    let evidence = broken(&resolve::resolve(
        &command("just ghost", "README.md"),
        &repo,
    ))
    .clone();
    match evidence.needle {
        Some(Needle::Regex { pattern, .. }) => {
            assert!(
                pattern.contains("@?"),
                "needle cannot see a quiet recipe: {pattern}"
            )
        }
        other => panic!("expected a regex needle, got {other:?}"),
    }
}

#[test]
fn a_makefile_with_a_catch_all_rule_answers_for_every_target() {
    // `%:` matches any target, so nothing can be said to be missing. llm's docs
    // Makefile forwards everything to sphinx this way.
    let (_dir, repo) = repo_with(&[("Makefile", "%:\n\t@sphinx-build $@\n")]);
    assert!(matches!(
        resolve::resolve(&command("make html", "README.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_makefile_without_a_catch_all_still_judges_targets() {
    let (_dir, repo) = repo_with(&[("Makefile", "build:\n\techo build\n")]);
    assert!(matches!(
        resolve::resolve(&command("make html", "README.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_manifest_is_named_with_the_case_git_tracks_it_under() {
    // llm tracks `Justfile`. `repo.exists` asks the filesystem, which is
    // case-insensitive on APFS, so the probe for `justfile` succeeded and the needle
    // was scoped to a path git does not have — and git pathspecs are case-sensitive,
    // so the search silently matched nothing.
    let (_dir, repo) = repo_with(&[("Justfile", "@build:\n  cargo build\n")]);
    let evidence = broken(&resolve::resolve(
        &command("just ghost", "README.md"),
        &repo,
    ))
    .clone();
    match evidence.needle {
        Some(Needle::Regex { scope, .. }) => {
            assert_eq!(
                scope,
                vec!["Justfile".to_string()],
                "wrong case reaches git"
            )
        }
        other => panic!("expected a scoped regex needle, got {other:?}"),
    }
}

#[test]
fn an_explicit_heading_id_is_an_anchor() {
    // pydantic writes `## Type hints powering schema validation {#type-hints}`. The
    // explicit id is the anchor mkdocs and pandoc emit; slugging the whole line
    // instead reports a link that works.
    let (_dir, repo) = repo_with(&[(
        "docs/why.md",
        "# Why\n\n## Type hints powering schema validation {#type-hints}\n",
    )]);
    assert!(matches!(
        resolve::resolve(&link("why.md", Some("type-hints"), "docs/index.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_heading_with_an_explicit_id_keeps_its_slug_too() {
    // GitHub ignores the attribute and slugs the whole line, so both spellings have to
    // resolve — the document is correct under either renderer.
    let (_dir, repo) = repo_with(&[("docs/why.md", "# Why\n\n## Strict mode {#strict-lax}\n")]);
    assert!(matches!(
        resolve::resolve(&link("why.md", Some("strict-mode"), "docs/index.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn emphasis_in_a_heading_does_not_change_its_anchor() {
    // black writes `### Cells that _Black_ will skip`. GitHub slugs the *rendered*
    // text, so the anchor is `cells-that-black-will-skip`; keeping the emphasis
    // markers reports a link that works.
    let (_dir, repo) = repo_with(&[(
        "docs/guide.md",
        "# Guide\n\n### Cells that _Black_ will skip\n",
    )]);
    assert!(matches!(
        resolve::resolve(
            &link(
                "guide.md",
                Some("cells-that-black-will-skip"),
                "docs/faq.md"
            ),
            &repo
        ),
        Resolution::True
    ));
}

#[test]
fn an_underscore_inside_an_identifier_still_anchors() {
    // The near-miss: an underscore that is part of a name is not emphasis, and GitHub
    // keeps it. Both readings have to resolve.
    let (_dir, repo) = repo_with(&[("docs/guide.md", "# Guide\n\n## The my_var setting\n")]);
    assert!(matches!(
        resolve::resolve(
            &link("guide.md", Some("the-my_var-setting"), "docs/faq.md"),
            &repo
        ),
        Resolution::True
    ));
}

#[test]
fn a_standalone_anchor_definition_is_an_anchor() {
    // pydantic writes `[](){#typeddict}` — mkdocs' idiom for an id that is not a
    // heading. It is still an anchor the page answers to.
    let (_dir, repo) = repo_with(&[(
        "docs/api/types.md",
        "# Types\n\n[](){#typeddict}\n\nStandard library type.\n",
    )]);
    assert!(matches!(
        resolve::resolve(
            &link("api/types.md", Some("typeddict"), "docs/why.md"),
            &repo
        ),
        Resolution::True
    ));
}

#[test]
fn an_anchor_inside_a_fence_is_not_an_anchor() {
    // The near-miss: a fence is a code sample, not page structure.
    let (_dir, repo) = repo_with(&[(
        "docs/api/types.md",
        "# Types\n\n```markdown\n[](){#typeddict}\n```\n",
    )]);
    assert!(matches!(
        resolve::resolve(
            &link("api/types.md", Some("typeddict"), "docs/why.md"),
            &repo
        ),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_same_document_anchor_resolves_against_its_own_headings() {
    let (_dir, repo) = repo_with(&[("docs/guide.md", "# Guide\n\n## Setup\n")]);
    assert!(matches!(
        resolve::resolve(&link("guide.md", Some("setup"), "docs/guide.md"), &repo),
        Resolution::True
    ));
    assert!(matches!(
        resolve::resolve(&link("guide.md", Some("missing"), "docs/guide.md"), &repo),
        Resolution::Broken(_)
    ));
}

// TypeScript is the second resolver, and the seam the `LanguageResolver` trait exists
// to prove. Everything below would have been a compile error before it shipped.

#[test]
fn a_typescript_function_is_a_definition() {
    let (_dir, repo) = repo_with(&[
        (
            "src/index.ts",
            "export function foldEvents(x: number) {\n  return x;\n}\n",
        ),
        ("src/other.ts", "export const other = 1;\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("foldEvents()"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_typescript_const_class_and_member_are_definitions() {
    let (_dir, repo) = repo_with(&[
        (
            "src/index.ts",
            "export const registry = new Map();\nexport class Engine {\n  restart() {}\n}\n",
        ),
        ("src/other.ts", "export const other = 1;\n"),
    ]);
    for name in ["registry", "Engine", "restart"] {
        assert!(
            matches!(resolve::resolve(&symbol(name), &repo), Resolution::True),
            "{name} is not a definition"
        );
    }
}

#[test]
fn a_tsx_file_parses_with_the_tsx_grammar() {
    // The TypeScript grammar reads `<Thing>` as a type argument; TSX is a separate
    // grammar for the same language.
    //
    // A self-closing element is not enough to tell them apart. `<div className="x" />`
    // — which this test used to rely on — recovers under both, so the grammar choice
    // could be deleted outright and this stayed green. What diverges is a closing tag
    // or a fragment: under the wrong grammar the parse fails there and takes the *next*
    // declaration with it, so a component defined below any ordinary piece of JSX stops
    // existing and every document naming it is reported wrong.
    let (_dir, repo) = repo_with(&[
        (
            "src/App.tsx",
            "export function App() {\n  return <div className=\"x\" />;\n}\n",
        ),
        // A fragment, then a declaration after it.
        (
            "src/Layout.tsx",
            "const shell = <>header</>;\nexport function Layout() {\n  return shell;\n}\n",
        ),
        // A closing tag, then a declaration after it.
        (
            "src/Card.tsx",
            "const label = <Text>hi</Text>;\nexport function Card() {\n  return label;\n}\n",
        ),
        // The same, in a .jsx file: the arm covers both extensions.
        (
            "src/List.jsx",
            "const rows = <ul><li>one</li></ul>;\nexport function List() {\n  return rows;\n}\n",
        ),
        ("src/other.tsx", "export const other = 1;\n"),
    ]);
    for name in ["App()", "Layout()", "Card()", "List()"] {
        assert!(
            matches!(resolve::resolve(&symbol(name), &repo), Resolution::True),
            "{name}: defined after JSX, and still defined",
        );
    }
}

#[test]
fn a_missing_typescript_symbol_is_broken_with_a_typescript_needle() {
    let (_dir, repo) = repo_with(&[
        ("src/index.ts", "export function other() {}\n"),
        ("src/two.ts", "export const two = 2;\n"),
    ]);
    let evidence = broken(&resolve::resolve(&symbol("foldEvents()"), &repo)).clone();
    match evidence.needle {
        Some(Needle::Regex { pattern, scope }) => {
            assert!(pattern.contains("foldEvents"), "got {pattern}");
            assert!(pattern.contains("function|class"), "got {pattern}");
            assert!(scope.contains(&"*.ts".to_string()), "got {scope:?}");
        }
        other => panic!("expected a scoped regex needle, got {other:?}"),
    }
}

#[test]
fn a_python_and_typescript_repository_is_fully_supported() {
    // Two shipped resolvers cover it between them, so symbols are still speakable.
    let (_dir, repo) = repo_with(&[
        ("app.py", "def fold_events():\n    pass\n"),
        ("web/index.ts", "export const ui = 1;\n"),
        ("web/other.ts", "export const other = 2;\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("fold_events()"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_language_with_no_resolver_still_silences_symbols() {
    // The near-miss: adding TypeScript must not weaken ADR-0004.
    let (_dir, repo) = repo_with(&[
        ("web/index.ts", "export const ui = 1;\n"),
        ("main.go", "package main\n"),
        ("util.go", "package main\n"),
    ]);
    assert!(matches!(
        resolve::resolve(&symbol("ui"), &repo),
        Resolution::Ambiguous(AmbiguityReason::UnsupportedLanguage)
    ));
}

// A name the repository imports, and a name the language provides, are both names in
// its namespace. typer documents `Path()` (imported from pathlib) and `print()` (a
// builtin); neither is defined in typer, and both were reported as rot.

#[test]
fn an_imported_name_is_a_definition() {
    let (_dir, repo) = repo_with(&[
        ("app.py", "from pathlib import Path\nimport json\n"),
        ("other.py", "x = 1\n"),
    ]);
    for name in ["Path()", "json"] {
        assert!(
            matches!(resolve::resolve(&symbol(name), &repo), Resolution::True),
            "{name} is imported and should be a definition"
        );
    }
}

#[test]
fn an_aliased_import_is_a_definition() {
    let (_dir, repo) = repo_with(&[
        ("app.py", "import numpy as np\nfrom os import path as p\n"),
        ("other.py", "x = 1\n"),
    ]);
    for name in ["np", "p"] {
        assert!(
            matches!(resolve::resolve(&symbol(name), &repo), Resolution::True),
            "{name} should be a definition"
        );
    }
}

#[test]
fn a_language_builtin_is_a_definition() {
    let (_dir, repo) = repo_with(&[
        ("app.py", "def other():\n    pass\n"),
        ("two.py", "x = 1\n"),
    ]);
    for name in ["print()", "len()", "ValueError"] {
        assert!(
            matches!(resolve::resolve(&symbol(name), &repo), Resolution::True),
            "{name} is a builtin and should be a definition"
        );
    }
}

#[test]
fn a_typescript_import_and_builtin_are_definitions() {
    let (_dir, repo) = repo_with(&[
        (
            "src/a.ts",
            "import { readFile } from \"fs\";\nimport axios from \"axios\";\n",
        ),
        ("src/b.ts", "export const b = 1;\n"),
    ]);
    for name in ["readFile", "axios", "Promise", "JSON"] {
        assert!(
            matches!(resolve::resolve(&symbol(name), &repo), Resolution::True),
            "{name} should be a definition"
        );
    }
}

#[test]
fn a_name_that_is_neither_imported_nor_builtin_is_still_broken() {
    // The near-miss: widening the index must not make everything resolve.
    let (_dir, repo) = repo_with(&[("app.py", "import json\n"), ("two.py", "x = 1\n")]);
    assert!(matches!(
        resolve::resolve(&symbol("fold_events()"), &repo),
        Resolution::Broken(_)
    ));
}

// Paths mutation testing found unconstrained: every one of these survived a mutant.

#[test]
fn every_pin_file_is_consulted() {
    // `pin_file`'s Python, Rust and Go arms could each be deleted without a test
    // noticing. Each pins exactly one version, which is the whole of ADR-0003.
    let cases: &[(&str, &str, &str, &str)] = &[
        ("Python", ".python-version", "3.12\n", "3.9"),
        (
            "Rust",
            "rust-toolchain.toml",
            "[toolchain]\nchannel = \"1.98.0\"\n",
            "1.70",
        ),
        ("Rust", "rust-toolchain", "1.98.0\n", "1.70"),
        ("Go", "go.mod", "module x\n\ngo 1.23\n", "1.19"),
        ("Node", ".nvmrc", "24.3.1\n", "18"),
        ("Node", ".node-version", "24.3.1\n", "18"),
    ];
    for (tool, file, contents, stale) in cases {
        let (_dir, repo) = repo_with(&[(file, contents)]);
        assert!(
            matches!(
                resolve::resolve(&version(tool, stale), &repo),
                Resolution::Broken(_)
            ),
            "{tool} {stale} should contradict {file}"
        );
    }
}

#[test]
fn a_cargo_alias_is_read_from_the_config() {
    // `cargo_aliases` could return an empty vec without a test noticing.
    let (_dir, repo) = repo_with(&[(
        ".cargo/config.toml",
        "[alias]\nlint = \"clippy --all-targets\"\n",
    )]);
    assert!(matches!(
        resolve::resolve(&command("cargo lint", "README.md"), &repo),
        Resolution::True
    ));
    assert!(matches!(
        resolve::resolve(&command("cargo ghost", "README.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_heading_slug_keeps_only_the_characters_github_keeps() {
    // The slugger's character guards each survived a mutant. GitHub keeps letters,
    // digits, hyphens and underscores, turns spaces into hyphens, and drops the rest.
    let (_dir, repo) = repo_with(&[(
        "docs/guide.md",
        "# Guide\n\n## Set up: the my_var flag (v2)!\n",
    )]);
    assert!(matches!(
        resolve::resolve(
            &link("guide.md", Some("set-up-the-my_var-flag-v2"), "docs/faq.md"),
            &repo
        ),
        Resolution::True
    ));
}

#[test]
fn an_attribute_block_must_be_closed_to_name_an_anchor() {
    // `explicit_ids` joins two conditions; replacing `&&` with `||` survived.
    let (_dir, repo) = repo_with(&[("docs/guide.md", "# Guide\n\n[](){#not closed\n")]);
    assert!(matches!(
        resolve::resolve(&link("guide.md", Some("not"), "docs/faq.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_path_is_anchored_document_relative_before_root_relative() {
    // A path claim is read against the document *or* the repository root, so
    // `anchored` offers both. The earlier version of this test put the file at both
    // readings, which means the document-relative candidate always hit and the
    // root-relative one was never needed — the whole second half of the function could
    // be dropped and this stayed green.
    let (_dir, repo) = repo_with(&[
        ("docs/guide/setup.md", "x"),
        ("setup.md", "y"),
        // Only at the root. This is the case that needs the second candidate.
        ("LICENSE", "z"),
    ]);
    assert!(matches!(
        resolve::resolve(&bare_path_in("setup.md", "docs/guide/README.md"), &repo),
        Resolution::True
    ));
    assert!(
        matches!(
            resolve::resolve(&bare_path_in("LICENSE", "docs/guide/README.md"), &repo),
            Resolution::True
        ),
        "a file that exists only at the repository root is still found from a subdirectory",
    );
    assert!(matches!(
        resolve::resolve(&bare_path_in("nowhere.md", "docs/guide/README.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_pin_file_that_names_no_number_pins_nothing() {
    // `is_version_number` refuses a channel, an alias and an implementation. Its last
    // clause also has to refuse the degenerate case: `.nvmrc` holding just `v` trims to
    // the empty string, and `chars().all(..)` is vacuously true for that. Read as a
    // pin, it would contradict every version anyone states.
    for contents in ["v\n", "\n", "stable\n", "lts/*\n", "pypy3.10-7.3.15\n"] {
        let (_dir, repo) = repo_with(&[(".nvmrc", contents)]);
        let claim = claim(
            ClaimKind::Version {
                tool: "Node".into(),
                version: "18".into(),
            },
            "Node 18",
            "README.md",
        );
        assert!(
            matches!(
                resolve::resolve(&claim, &repo),
                Resolution::Skip(SkipReason::NoPin)
            ),
            "{contents:?} names no single version, so it contradicts nothing",
        );
    }

    // The near-miss: a real pin still contradicts a stated version.
    let (_dir, repo) = repo_with(&[(".nvmrc", "24.2.0\n")]);
    let claim = claim(
        ClaimKind::Version {
            tool: "Node".into(),
            version: "18".into(),
        },
        "Node 18",
        "README.md",
    );
    assert!(
        matches!(resolve::resolve(&claim, &repo), Resolution::Broken(_)),
        "a numeric pin is still read",
    );
}

fn bare_path_in(text: &str, document: &str) -> Claim {
    claim(ClaimKind::Path, text, document)
}

#[test]
fn a_deno_task_is_read_from_the_config() {
    let (_dir, repo) = repo_with(&[("deno.json", "{\"tasks\": {\"build\": \"deno compile\"}}")]);
    assert!(matches!(
        resolve::resolve(&command("deno task build", "README.md"), &repo),
        Resolution::True
    ));
    assert!(matches!(
        resolve::resolve(&command("deno task ghost", "README.md"), &repo),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_deno_subcommand_is_not_a_task() {
    // The near-miss: `deno fmt` is built in, not something the config declares.
    let (_dir, repo) = repo_with(&[("deno.json", "{\"tasks\": {\"build\": \"deno compile\"}}")]);
    assert!(matches!(
        resolve::resolve(&command("deno fmt", "README.md"), &repo),
        Resolution::True
    ));
}

#[test]
fn a_module_name_is_only_a_namespace_where_an_import_would_find_it() {
    // `modules()` accepts a file as a module only when it sits at the repository root
    // or directly under a source root, because that is where a bare `import helpers`
    // would resolve. A file buried further down is reachable only through its package,
    // so its stem is not a namespace anyone can write on its own.
    //
    // Drop that condition and every `.py` in the tree donates its stem. `helpers` then
    // looks readable, `helpers.missing()` gets judged against an index that was never
    // about it, and the answer is a finding in a default run.
    let (_dir, nested) = repo_with(&[("src/deep/nested/helpers.py", "def present():\n    pass\n")]);
    assert!(
        matches!(
            resolve::resolve(&symbol("helpers.missing"), &nested),
            Resolution::Ambiguous(AmbiguityReason::UnreadableNamespace)
        ),
        "a stem that no bare import reaches is not a namespace",
    );

    // The near-miss, and the reason the rule is not simply "never trust a stem": the
    // same file directly under `src/` is exactly what `import helpers` finds, so a
    // name genuinely absent from it is still judged.
    let (_dir2, rooted) = repo_with(&[("src/helpers.py", "def present():\n    pass\n")]);
    assert!(
        matches!(
            resolve::resolve(&symbol("helpers.missing"), &rooted),
            Resolution::Broken { .. }
        ),
        "a module under a source root is readable, so an absent name is broken",
    );
    assert!(
        matches!(
            resolve::resolve(&symbol("helpers.present"), &rooted),
            Resolution::True
        ),
        "and a name it does define is true",
    );
}

#[test]
fn a_path_is_normalised_without_touching_the_filesystem() {
    use stilltrue::repo::normalise;

    // `Path::components` folds away an interior `.` on its own, but keeps a leading
    // one — and `./docs/guide.md` is how most relative links in a README are written.
    // Without the `CurDir` arm that leading component falls to the catch-all, the whole
    // path returns `None`, and every link spelled that way stops being checked at all.
    // Silently: an unresolvable path is not a finding.
    assert_eq!(
        normalise(Path::new("./docs/guide.md")).as_deref(),
        Some(Path::new("docs/guide.md")),
        "a leading `./` is a spelling, not a refusal",
    );
    assert_eq!(
        normalise(Path::new("./")).as_deref(),
        Some(Path::new("")),
        "and on its own it is the root of the repository",
    );

    // The rest of the contract, which the same catch-all is responsible for.
    assert_eq!(
        normalise(Path::new("docs/../README.md")).as_deref(),
        Some(Path::new("README.md")),
        "`..` is collapsed rather than resolved on disk",
    );
    assert_eq!(
        normalise(Path::new("../outside")),
        None,
        "a path that climbs out of the repository is refused",
    );
    assert_eq!(
        normalise(Path::new("/etc/passwd")),
        None,
        "and so is an absolute one",
    );
}

#[test]
fn which_argument_names_the_target_depends_on_the_runner() {
    // `target_of` decides which word is the thing that has to exist, and both of its
    // guards were unconstrained.
    //
    // Relax the npm guard and every word after any subcommand becomes a target:
    // `npm install express` starts asking the scripts table for `express`, which is a
    // dependency, not a script. Relax the deno one and `deno run main.ts` asks the task
    // table for `main.ts`, which is a file. Both are findings against things that were
    // never claimed to be tasks.
    let (_dir, repo) = repo_with(&[
        ("package.json", "{\"scripts\":{\"build\":\"tsc\"}}"),
        ("deno.json", "{\"tasks\":{\"dev\":\"deno run main.ts\"}}"),
    ]);

    // What a package name after a subcommand must never do.
    for text in ["npm install express", "pnpm add -D vite", "yarn add react"] {
        assert!(
            matches!(
                resolve::resolve(&command(text, "README.md"), &repo),
                Resolution::True
            ),
            "{text}: the package is not a script and must not be looked up as one",
        );
    }

    // A deno subcommand is not a task, whatever follows it.
    for text in ["deno run main.ts", "deno fmt", "deno test"] {
        assert!(
            matches!(
                resolve::resolve(&command(text, "README.md"), &repo),
                Resolution::True
            ),
            "{text}: a deno subcommand is not a task lookup",
        );
    }

    // And the other half of each guard, or the rules above would be satisfied by never
    // checking these runners at all: yarn names its script directly, npm names it after
    // `run`, and deno names its task after `task`.
    for (text, resolves) in [
        ("yarn build", true),
        ("yarn nope", false),
        ("npm run build", true),
        ("npm run nope", false),
        ("deno task dev", true),
        ("deno task nope", false),
    ] {
        let got = resolve::resolve(&command(text, "README.md"), &repo);
        assert_eq!(
            matches!(got, Resolution::True),
            resolves,
            "{text}: expected resolves={resolves}, got {got:?}",
        );
    }
}

#[test]
fn a_manifest_offers_only_its_real_targets_as_candidates() {
    // Every suggestion the tool prints comes out of these lists, and both parsers guard
    // against putting things in them that are not targets: `.PHONY` and `VAR :=` in a
    // Makefile, an `export` assignment and a parameter list in a justfile.
    //
    // None of those guards was covered. Weakened, the lists grow rather than shrink, so
    // nothing fails to resolve — instead the tool answers "did you mean: .PHONY?" and
    // treats `make .PHONY` as a target that exists. Asserting the exact list is what
    // makes an extra entry a failure; asserting "contains build" would not.
    let (_dir, repo) = repo_with(&[
        (
            "Makefile",
            ".PHONY: build test\nVERSION := 1.0\nbuild:\n\tcc x\ntest:\n\tcc y\n",
        ),
        (
            // The last two lines are malformed rather than typical, and that is the
            // point: `rest.starts_with('=')` already catches a well-formed assignment,
            // so the name guards only ever see input like this. A bare `:` yields an
            // empty name and `a=b: c` a name with an `=` in it, and neither is a recipe
            // anyone can run.
            "justfile",
            "export RUST_LOG := \"debug\"\nbuild:\n  echo\ntest *args:\n  echo\n:\na=b: c\n",
        ),
    ]);

    for runner in ["make", "just"] {
        let broken_claim = command(&format!("{runner} nope"), "README.md");
        let evidence = broken(&resolve::resolve(&broken_claim, &repo)).clone();
        assert_eq!(
            evidence.candidates,
            vec!["build".to_string(), "test".to_string()],
            "{runner}: only real targets are candidates",
        );
    }

    // And the same entries are not targets when claimed directly: a document saying
    // `make .PHONY` is wrong, and the parser is what knows it.
    assert!(
        matches!(
            resolve::resolve(&command("make .PHONY", "README.md"), &repo),
            Resolution::Broken { .. }
        ),
        "`.PHONY` is a declaration, not a target",
    );
    assert!(
        matches!(
            resolve::resolve(&command("just RUST_LOG", "README.md"), &repo),
            Resolution::Broken { .. }
        ),
        "an exported variable is not a recipe",
    );
}

#[test]
fn an_enum_member_is_a_definition() {
    // `Mode.On` was reported as rot in a default run with `enum Mode { On = 1 }` in the
    // file being scanned, blamed on the commit that introduced it. The definitions
    // query indexed the enum's *name* and nothing inside it, so every member of every
    // TypeScript enum was a name this repository could not find.
    let (_dir, repo) = repo_with(&[("src/mode.ts", "export enum Mode {\n  On = 1,\n  Off,\n}\n")]);

    for member in ["Mode.On", "Mode.Off"] {
        assert!(
            matches!(resolve::resolve(&symbol(member), &repo), Resolution::True),
            "{member}: both spellings of a member are definitions",
        );
    }

    // The near-miss, or widening the index would just mean never judging an enum
    // again: the enum is a readable namespace, so a member it does not have is broken.
    assert!(
        matches!(
            resolve::resolve(&symbol("Mode.Sideways"), &repo),
            Resolution::Broken { .. }
        ),
        "a member the enum does not declare is still judged",
    );
}

#[test]
fn a_typescript_namespace_is_read_from_the_declarations_that_make_one() {
    // `namespace_query` is what lets a dotted TypeScript name be judged at all. Empty
    // it, and every `Thing.member` becomes ambiguous — silent, and quietly so: the
    // findings do not move to another tier, they stop existing.
    let (_dir, repo) = repo_with(&[(
        "src/api.ts",
        "export class Widget {\n  render() { return 1; }\n}\n\
         export interface Opts {\n  retries: number;\n}\n\
         export enum Mode {\n  On = 1,\n}\n\
         export type Shape = { kind: string };\n",
    )]);

    // Each kind of declaration the query names is a namespace, so a member it does not
    // have is broken rather than unspeakable.
    for missing in [
        "Widget.missing()",
        "Opts.missing",
        "Mode.Missing",
        "Shape.missing",
    ] {
        assert!(
            matches!(
                resolve::resolve(&symbol(missing), &repo),
                Resolution::Broken { .. }
            ),
            "{missing}: a readable namespace judges what it does not contain",
        );
    }

    // And the members they do have resolve.
    for present in ["Widget.render()", "Opts.retries", "Mode.On", "Shape.kind"] {
        assert!(
            matches!(resolve::resolve(&symbol(present), &repo), Resolution::True),
            "{present}: is declared",
        );
    }

    // The boundary: a namespace this repository never declares cannot be judged at all.
    assert!(
        matches!(
            resolve::resolve(&symbol("Unknown.thing()"), &repo),
            Resolution::Ambiguous(AmbiguityReason::UnreadableNamespace)
        ),
        "an unknown namespace stays ambiguous",
    );
}

#[test]
fn an_anchor_list_does_not_grow_spellings_github_never_emits() {
    // The emphasis-stripped slug and the explicit-id reader both *add* anchors, so
    // getting them wrong makes more links resolve rather than fewer. That is the
    // quiet direction: a link that is genuinely broken stops being reported, and no
    // existing test noticed because they all assert that a real anchor resolves.
    let (_dir, repo) = repo_with(&[(
        "docs/guide.md",
        "# Guide\n\n         ## Using `docker` (v2)\n\n         ## Limits {#hard limits}\n",
    )]);

    // GitHub drops the parentheses when it slugs, so this is the anchor.
    assert!(
        matches!(
            resolve::resolve(
                &link("guide.md", Some("using-docker-v2"), "docs/faq.md"),
                &repo
            ),
            Resolution::True
        ),
        "the real slug resolves",
    );
    // And this is not. Keeping punctuation in the stripped variant would quietly make
    // a link nobody can follow resolve.
    assert!(
        matches!(
            resolve::resolve(
                &link("guide.md", Some("using-docker-(v2)"), "docs/faq.md"),
                &repo
            ),
            Resolution::Broken(_)
        ),
        "punctuation is not part of a heading slug",
    );

    // An attribute id containing whitespace is not an id. The heading still anchors by
    // its own slug, and the malformed attribute contributes nothing.
    assert!(
        matches!(
            resolve::resolve(&link("guide.md", Some("limits"), "docs/faq.md"), &repo),
            Resolution::True
        ),
        "the heading still anchors by its slug",
    );
    assert!(
        matches!(
            resolve::resolve(&link("guide.md", Some("hard limits"), "docs/faq.md"), &repo),
            Resolution::Broken(_)
        ),
        "an id with a space in it is not an anchor",
    );
}

// ---------------------------------------------------------------------------------
// Reason codes (ADR-0019): what a quiet run did not look at, and why.
// ---------------------------------------------------------------------------------

/// Whether this process can be refused a read by file mode alone. Root cannot, and a
/// test that expects a refusal would then fail for a reason unrelated to the code.
fn permissions_bind() -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let probe = dir.path().join("probe");
    std::fs::write(&probe, "x").unwrap();
    set_mode(&probe, 0o000);
    let refused = std::fs::read(&probe).is_err();
    set_mode(&probe, 0o644);
    refused
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_malformed_package_json_is_skipped_not_broken() {
    // The trailing comma is the whole case. Read as "no scripts", `build` looked
    // missing, and `"build":` is in the manifest's history, so it was rot.
    let (_dir, repo) = repo_with(&[("package.json", "{\"scripts\": {\"build\": \"tsc\",}}\n")]);
    assert!(matches!(
        resolve::resolve(&command("npm run build", "README.md"), &repo),
        Resolution::Skip(SkipReason::ManifestUnparseable)
    ));
    // The same manifest, parseable, still reports what is genuinely missing — so the
    // skip above is the parse failure and not the runner.
    let (_dir, fixed) = repo_with(&[("package.json", "{\"scripts\": {\"build\": \"tsc\"}}\n")]);
    assert!(matches!(
        resolve::resolve(&command("npm run lint", "README.md"), &fixed),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_malformed_cargo_config_is_skipped_not_broken() {
    let (_dir, repo) = repo_with(&[(".cargo/config.toml", "[alias]\nxtask = \n")]);
    assert!(matches!(
        resolve::resolve(&command("cargo xtask", "README.md"), &repo),
        Resolution::Skip(SkipReason::ManifestUnparseable)
    ));
    let (_dir, fixed) = repo_with(&[(".cargo/config.toml", "[alias]\nxtask = \"run\"\n")]);
    assert!(matches!(
        resolve::resolve(&command("cargo dist", "README.md"), &fixed),
        Resolution::Broken(_)
    ));
}

#[test]
fn a_malformed_deno_json_is_skipped_not_broken() {
    let (_dir, repo) = repo_with(&[("deno.json", "{\"tasks\": {\"dev\": \"x\"\n")]);
    assert!(matches!(
        resolve::resolve(&command("deno task dev", "README.md"), &repo),
        Resolution::Skip(SkipReason::ManifestUnparseable)
    ));
}

#[test]
fn a_manifest_that_cannot_be_read_is_a_failed_read() {
    if !permissions_bind() {
        eprintln!("skipped: file modes do not bind for this user");
        return;
    }
    let (dir, repo) = repo_with(&[("Makefile", "demo:\n\techo hi\n")]);
    set_mode(&dir.path().join("Makefile"), 0o000);
    let resolution = resolve::resolve(&command("make demo", "README.md"), &repo);
    set_mode(&dir.path().join("Makefile"), 0o644);
    assert!(matches!(
        resolution,
        Resolution::Skip(SkipReason::ManifestUnreadable)
    ));
}

#[test]
fn a_pin_that_cannot_be_read_is_not_an_absent_pin() {
    if !permissions_bind() {
        eprintln!("skipped: file modes do not bind for this user");
        return;
    }
    let (dir, repo) = repo_with(&[(".nvmrc", "20.1.0\n")]);
    set_mode(&dir.path().join(".nvmrc"), 0o000);
    let resolution = resolve::resolve(&version("Node", "18"), &repo);
    set_mode(&dir.path().join(".nvmrc"), 0o644);
    assert!(matches!(
        resolution,
        Resolution::Skip(SkipReason::PinUnreadable)
    ));
}

#[test]
fn a_link_that_leaves_the_repository_is_skipped_as_outside() {
    let (_dir, repo) = repo_with(&[("README.md", "")]);
    assert!(matches!(
        resolve::resolve(&link_claim("../../elsewhere.md", "README.md"), &repo),
        Resolution::Skip(SkipReason::OutsideRepository)
    ));
}

#[test]
fn every_reason_code_is_distinct_and_kebab_case() {
    let skips = [
        SkipReason::NoManifest,
        SkipReason::ManifestUnreadable,
        SkipReason::ManifestUnparseable,
        SkipReason::UnsupportedRunner,
        SkipReason::NoPin,
        SkipReason::PinUnreadable,
        SkipReason::OutsideRepository,
        SkipReason::TargetUnreadable,
        SkipReason::NotAClaim,
    ];
    let ambiguities = [
        AmbiguityReason::UnsupportedLanguage,
        AmbiguityReason::DynamicModule,
        AmbiguityReason::UnreadableNamespace,
        AmbiguityReason::EcosystemFilename,
        AmbiguityReason::AssembledName,
        AmbiguityReason::SiteRelativeLink,
        AmbiguityReason::NoNeedle,
    ];
    let codes: Vec<&str> = skips
        .iter()
        .map(|r| r.code())
        .chain(ambiguities.iter().map(|r| r.code()))
        .collect();
    let distinct: std::collections::BTreeSet<&str> = codes.iter().copied().collect();
    assert_eq!(distinct.len(), codes.len(), "{codes:?}");
    for code in &codes {
        assert!(
            code.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{code} is not kebab-case"
        );
    }
    // Exactly the reads that should have succeeded are failures. An absent manifest is
    // a coverage limitation; a manifest that is there and could not be used is not.
    let failures: Vec<&str> = skips
        .iter()
        .filter(|r| r.is_failure())
        .map(|r| r.code())
        .collect();
    assert_eq!(
        failures,
        [
            "manifest-unreadable",
            "manifest-unparseable",
            "pin-unreadable",
            "target-unreadable"
        ]
    );
}

#[test]
fn a_nested_repository_is_not_part_of_the_walk() {
    // ADR-0025. A submodule or a nested clone has its own `.git` directory, a worktree a
    // `.git` file; either way it is another repository. A plain directory is not.
    let (dir, repo) = repo_with(&[
        ("README.md", "top\n"),
        ("docs/guide.md", "guide\n"),
        ("vendor/lib/README.md", "vendored\n"),
        ("vendor/lib/src/lib.py", "def x(): pass\n"),
        ("worktree/README.md", "worktree\n"),
    ]);
    std::fs::create_dir_all(dir.path().join("vendor/lib/.git")).unwrap();
    std::fs::write(dir.path().join("worktree/.git"), "gitdir: /elsewhere\n").unwrap();
    let files: Vec<String> = repo
        .files()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["README.md", "docs/guide.md"]);
}
