//! The `Command` resolver: does the runner's target exist?
//!
//! This module owns every runner's manifest format *and* its history needle, because
//! only the resolver that failed knows what it looked for (ADR-0001).

use std::path::{Path, PathBuf};

use crate::claim::Claim;
use crate::repo::Repo;
use crate::resolution::{Evidence, Needle, Resolution, SkipReason};

/// Subcommands that always resolve, regardless of any manifest.
fn builtins(runner: &str) -> &'static [&'static str] {
    match runner {
        "cargo" => &[
            "build", "run", "test", "check", "fmt", "clippy", "install", "add", "remove", "update",
            "publish", "bench", "doc", "clean", "init", "new", "tree", "search", "fetch", "vendor",
        ],
        "npm" | "pnpm" | "yarn" => &[
            "install", "i", "ci", "add", "remove", "update", "publish", "exec", "dlx", "init",
            "create", "link", "pack", "audit", "outdated", "why", "list", "ls", "version",
        ],
        "go" => &[
            "build", "run", "test", "mod", "get", "install", "vet", "fmt", "generate", "work",
            "tool", "clean", "doc", "list",
        ],
        "uv" | "uvx" => &[
            "run", "add", "remove", "sync", "lock", "pip", "venv", "tool", "init", "build",
            "publish", "export",
        ],
        _ => &[],
    }
}

pub fn resolve(claim: &Claim, repo: &Repo, runner: &str, args: &[String]) -> Resolution {
    // Runners with no repository-local target list: nothing to contradict.
    if matches!(runner, "docker" | "python") {
        return Resolution::True;
    }

    let Some(target) = target_of(runner, args) else {
        // No target named at all (`cargo`, `npm`) — nothing to check.
        return Resolution::True;
    };
    if builtins(runner).contains(&target.as_str()) {
        return Resolution::True;
    }

    let Some(manifest) = nearest_manifest(repo, &claim.file, runner) else {
        return Resolution::Skip(SkipReason::NoManifest);
    };
    let Some(contents) = read(repo, &manifest) else {
        return Resolution::Skip(SkipReason::ManifestUnreadable);
    };

    // A `%` pattern rule matches any target, so nothing in this Makefile can be said
    // to be missing. Sphinx docs Makefiles forward everything this way.
    if runner == "make" && has_catch_all(&contents) {
        return Resolution::True;
    }

    let targets = match runner {
        "make" => Some(make_targets(&contents)),
        "just" => Some(just_recipes(&contents)),
        "npm" | "pnpm" | "yarn" => package_scripts(&contents),
        "deno" => deno_tasks(&contents),
        "cargo" => cargo_aliases(&contents),
        _ => return Resolution::Skip(SkipReason::UnsupportedRunner),
    };
    // A manifest that did not parse has an unknown target list, not an empty one. Read
    // as empty, one stray comma in `package.json` made every script it names broken —
    // and each of those names is in the manifest's history, so all of them were rot.
    let Some(targets) = targets else {
        return Resolution::Skip(SkipReason::ManifestUnparseable);
    };

    if targets.contains(&target) {
        return Resolution::True;
    }

    let manifest_display = manifest.to_string_lossy().into_owned();
    Resolution::Broken(Evidence {
        needle: Some(Needle::Regex {
            pattern: needle_for(runner, &target),
            scope: vec![manifest_display.clone()],
        }),
        candidates: targets,
        message: format!(
            "command `{}` has no target in {manifest_display}",
            claim.text
        ),
    })
}

/// Which argument names the target. `npm run build` names `build`; `yarn build` names
/// `build`; `make demo` names `demo`.
fn target_of(runner: &str, args: &[String]) -> Option<String> {
    let mut args = args.iter().filter(|a| !a.starts_with('-'));
    let first = args.next()?;
    match runner {
        "npm" | "pnpm" | "yarn" if first == "run" => args.next().cloned(),
        "deno" if first == "task" => args.next().cloned(),
        // `deno fmt`, `deno test` and friends are subcommands, not tasks.
        "deno" => None,
        _ => Some(first.clone()),
    }
}

/// The definition-shaped needle for this runner's target (ADR-0005).
fn needle_for(runner: &str, target: &str) -> String {
    // POSIX ERE — git's pickaxe dialect. `\s` there is a literal `s`.
    let target = crate::resolution::escape_ere(target);
    match runner {
        "npm" | "pnpm" | "yarn" | "deno" => format!("\"{target}\"[[:space:]]*:"),
        "cargo" => format!("^{target}[[:space:]]*="),
        // `@` makes a just recipe quiet, and the parser has always read past it. The
        // needle has to as well, or every command finding in a repository that writes
        // its recipes that way degrades from rot to an invisible lie.
        "just" => format!("^@?{target}[[:space:]]*:"),
        _ => format!("^{target}[[:space:]]*:"),
    }
}

/// Walk up from the document to the repository root looking for this runner's manifest.
fn nearest_manifest(repo: &Repo, document: &Path, runner: &str) -> Option<PathBuf> {
    let names: &[&str] = match runner {
        "make" => &["Makefile", "makefile", "GNUmakefile"],
        "just" => &["justfile", "Justfile", ".justfile"],
        "deno" => &["deno.json", "deno.jsonc"],
        "npm" | "pnpm" | "yarn" => &["package.json"],
        "cargo" => &[".cargo/config.toml", ".cargo/config"],
        _ => return None,
    };

    let mut dir = document.parent().map(Path::to_path_buf).unwrap_or_default();
    loop {
        for name in names {
            let candidate = dir.join(name);
            // `tracks`, not `exists`: this name becomes a git pathspec, and git is
            // case-sensitive where APFS is not.
            if repo.tracks(&candidate) {
                return Some(candidate);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn read(repo: &Repo, relative: &Path) -> Option<String> {
    std::fs::read_to_string(repo.root().join(relative)).ok()
}

/// Target names from a Makefile: a line beginning with a name followed by `:`.
fn make_targets(contents: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in contents.lines() {
        if line.starts_with(['\t', ' ', '#']) {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        // `VAR := value` is an assignment, not a target.
        if rest.starts_with('=') {
            continue;
        }
        let name = head.trim();
        if name.is_empty() || name.starts_with('.') || name.contains(['=', '$', '%']) {
            continue;
        }
        for target in name.split_whitespace() {
            if !out.contains(&target.to_string()) {
                out.push(target.to_string());
            }
        }
    }
    out
}

/// Recipe names from a justfile.
fn just_recipes(contents: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in contents.lines() {
        // `[group: 'dev']` and `[private]` are attributes, and an attribute contains a
        // colon without naming a recipe.
        if line.starts_with([' ', '\t', '#', '[']) {
            continue;
        }
        // `alias mypy := typecheck` makes `just mypy` work, so it names a recipe too.
        // It has the shape of an assignment, so it must be read before the assignment
        // skip below discards it — and it has to land with the `@?` needle rather than
        // after it, or every aliased recipe becomes a Tier A false positive.
        if let Some(rest) = line.strip_prefix("alias ")
            && let Some(name) = rest.split_whitespace().next()
            && !out.contains(&name.to_string())
        {
            out.push(name.to_string());
            continue;
        }
        // A leading `@` makes a recipe quiet. It is still a recipe: llm writes every
        // one of its own that way, and skipping the line lost all of them.
        let line = line.strip_prefix('@').unwrap_or(line);
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        // `RUST_LOG := "debug"` is an assignment, not a recipe.
        if rest.starts_with('=') {
            continue;
        }
        // Anything after the name is the recipe's parameters: `test *options:`.
        let name = head.split_whitespace().next().unwrap_or_default();
        if !name.is_empty() && !name.contains('=') && !out.contains(&name.to_string()) {
            out.push(name.to_string());
        }
    }
    out
}

/// Script names from a `package.json`, or `None` when it does not parse.
fn package_scripts(contents: &str) -> Option<Vec<String>> {
    let value = serde_json::from_str::<serde_json::Value>(contents).ok()?;
    Some(
        value
            .get("scripts")
            .and_then(serde_json::Value::as_object)
            .map(|scripts| scripts.keys().cloned().collect())
            .unwrap_or_default(),
    )
}

/// Alias names from `.cargo/config.toml`, or `None` when it does not parse.
fn cargo_aliases(contents: &str) -> Option<Vec<String>> {
    // `toml::from_str`, not `str::parse`: the `FromStr` impl rejects a document that
    // `from_str` accepts, and this returned an empty alias list for every repository —
    // silently, because the error went nowhere. Every `cargo <alias>` was broken.
    let value = toml::from_str::<toml::Value>(contents).ok()?;
    Some(
        value
            .get("alias")
            .and_then(toml::Value::as_table)
            .map(|aliases| aliases.keys().cloned().collect())
            .unwrap_or_default(),
    )
}

/// Whether a Makefile forwards every unmatched target to a pattern rule.
fn has_catch_all(contents: &str) -> bool {
    contents.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(head, rest)| head.trim() == "%" && !rest.starts_with('='))
    })
}

/// Task names from a `deno.json`, or `None` when it does not parse.
fn deno_tasks(contents: &str) -> Option<Vec<String>> {
    let value = serde_json::from_str::<serde_json::Value>(contents).ok()?;
    Some(
        value
            .get("tasks")
            .and_then(serde_json::Value::as_object)
            .map(|tasks| tasks.keys().cloned().collect())
            .unwrap_or_default(),
    )
}
