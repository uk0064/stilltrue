//! The repository a document makes claims about.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The repository root is the git toplevel, or the working directory when there is no
/// git repository.
pub struct Repo {
    root: PathBuf,
    files: OnceLock<Walk>,
    /// Indexes built once per run and shared. Building them per claim is the
    /// difference between a warm run of under a second and one of half a minute.
    identifiers: OnceLock<HashSet<String>>,
    definitions: OnceLock<BTreeSet<String>>,
    namespaces: OnceLock<BTreeSet<String>>,
    modules: OnceLock<BTreeSet<String>>,
    module_files: OnceLock<BTreeSet<(String, PathBuf)>>,
    dynamic_modules: OnceLock<BTreeSet<String>>,
    file_set: OnceLock<HashSet<PathBuf>>,
    symbols_supported: OnceLock<bool>,
}

impl Repo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            files: OnceLock::new(),
            identifiers: OnceLock::new(),
            definitions: OnceLock::new(),
            namespaces: OnceLock::new(),
            modules: OnceLock::new(),
            module_files: OnceLock::new(),
            dynamic_modules: OnceLock::new(),
            file_set: OnceLock::new(),
            symbols_supported: OnceLock::new(),
        }
    }

    /// Discover the root by asking git, falling back to `start` itself.
    pub fn discover(start: &Path) -> Self {
        let toplevel = std::process::Command::new("git")
            .arg("-C")
            .arg(start)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.is_dir());
        Self::new(toplevel.unwrap_or_else(|| start.to_path_buf()))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every file in the repository, gitignore-aware, as repository-relative paths.
    /// Walked once.
    pub fn files(&self) -> &[PathBuf] {
        &self.walk().files
    }

    /// What the walk could not enter or read. A directory it was refused is a set of
    /// documents nobody looked at, so these make a run incomplete (ADR-0019).
    pub fn walk_errors(&self) -> &[String] {
        &self.walk().errors
    }

    fn walk(&self) -> &Walk {
        self.files.get_or_init(|| {
            let mut files = Vec::new();
            let mut errors = Vec::new();
            for entry in ignore::WalkBuilder::new(&self.root)
                .hidden(false)
                .filter_entry(|e| e.file_name() != ".git" && !nested_repository(e))
                .build()
            {
                match entry {
                    Ok(entry) => {
                        if entry.file_type().is_some_and(|t| t.is_file())
                            && let Ok(relative) = entry.path().strip_prefix(&self.root)
                        {
                            files.push(relative.to_path_buf());
                        }
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
            files.sort();
            errors.sort();
            Walk { files, errors }
        })
    }

    /// Every env-var-shaped name appearing in a file that can serve as evidence — code
    /// and config, never prose. Built once; an `EnvVar` claim is a set lookup.
    ///
    /// Filtered to the shape `EnvVar` claims actually take, rather than every token in
    /// the repository: the unfiltered set is the whole codebase's vocabulary held for
    /// the lifetime of the run, to answer a few hundred lookups.
    pub fn env_names(&self) -> &HashSet<String> {
        self.identifiers.get_or_init(|| {
            let mut out = HashSet::new();
            for file in self.files() {
                if !crate::resolve::universal::is_evidence(file) {
                    continue;
                }
                let Ok(contents) = std::fs::read_to_string(self.root.join(file)) else {
                    continue;
                };
                out.extend(
                    contents
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .filter(|token| crate::extract::is_env_var(token))
                        .map(str::to_string),
                );
            }
            out
        })
    }

    /// Every definition name a shipped language resolver can find. Built once.
    pub fn definitions(&self) -> &BTreeSet<String> {
        self.definitions.get_or_init(|| {
            // The union across every resolver with something to read here. A document
            // never says what language a symbol is in (ADR-0004), so neither does this.
            crate::resolve::lang::active(self)
                .iter()
                .flat_map(|resolver| resolver.definitions(self))
                .collect()
        })
    }

    /// Every namespace whose contents this repository can actually read: a class, an
    /// interface, an enum. Judging `Foo.bar` means being able to read `Foo`, and a
    /// name that is merely *defined* — a parameter, a local — is not one (ADR-0013).
    pub fn namespaces(&self) -> &BTreeSet<String> {
        self.namespaces.get_or_init(|| {
            crate::resolve::lang::active(self)
                .iter()
                .flat_map(|resolver| resolver.namespaces(self))
                .collect()
        })
    }

    /// Every importable namespace in the repository: a `.py` file or a package
    /// directory at the repository root or directly under a source root. Built once.
    ///
    /// Only the top level counts, because only the top level is what `import X` finds.
    /// poetry carries `tests/fixtures/…/project/`, and counting that made
    /// `project.maintainers` — a `pyproject.toml` table — look judgeable.
    pub fn modules(&self) -> &BTreeSet<String> {
        self.modules
            .get_or_init(|| self.module_files().iter().map(|(n, _)| n.clone()).collect())
    }

    /// Modules that compute their own attributes with a module-level `__getattr__`,
    /// and are therefore no more readable than an instance is (ADR-0013).
    ///
    /// A module-level `def` sits at column zero, which is the whole test: a method
    /// named `__getattr__` on a class is indented, and describes that class rather
    /// than the module. click's `__init__.py` serves `get_text_stream` this way, from
    /// a name matched as a string — the index holds `_get_text_stream`, which is not
    /// what anybody writes down.
    pub fn dynamic_modules(&self) -> &BTreeSet<String> {
        self.dynamic_modules.get_or_init(|| {
            self.module_files()
                .iter()
                .filter(|(_, file)| {
                    std::fs::read_to_string(self.root.join(file)).is_ok_and(|source| {
                        source
                            .lines()
                            .any(|line| line.starts_with("def __getattr__"))
                    })
                })
                .map(|(name, _)| name.clone())
                .collect()
        })
    }

    fn module_files(&self) -> &BTreeSet<(String, PathBuf)> {
        self.module_files.get_or_init(|| {
            let mut out = BTreeSet::new();
            const SOURCE_ROOTS: &[&str] = &["src", "lib"];
            let top_level = |dir: &Path| {
                dir.as_os_str().is_empty()
                    || dir.to_str().is_some_and(|d| SOURCE_ROOTS.contains(&d))
            };
            let readable: Vec<&str> = crate::resolve::lang::resolvers()
                .iter()
                .flat_map(|resolver| resolver.extensions().iter().copied())
                .collect();
            for file in self.files() {
                let Some(ext) = file.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if !readable.contains(&ext) {
                    continue;
                }
                let Some(dir) = file.parent() else { continue };
                match file.file_stem().and_then(|s| s.to_str()) {
                    Some("__init__") => {
                        let Some(package) = dir.file_name() else {
                            continue;
                        };
                        if dir.parent().is_some_and(top_level) {
                            out.insert((package.to_string_lossy().into_owned(), file.clone()));
                        }
                    }
                    Some(stem) if top_level(dir) => {
                        out.insert((stem.to_string(), file.clone()));
                    }
                    _ => {}
                }
            }
            out
        })
    }

    /// Whether every code language present has a shipped resolver (ADR-0004).
    pub fn symbols_supported(&self) -> bool {
        *self
            .symbols_supported
            .get_or_init(|| crate::resolve::lang::fully_supported(self))
    }

    pub fn exists(&self, relative: &Path) -> bool {
        normalise(relative).is_some_and(|p| self.root.join(p).exists())
    }

    /// Whether the walk found this exact path, case included.
    ///
    /// `exists` asks the filesystem, which is case-insensitive on macOS and Windows.
    /// Git pathspecs are case-sensitive everywhere, so anything whose name will be
    /// handed to git has to be matched against the names the walk actually saw.
    pub fn tracks(&self, relative: &Path) -> bool {
        self.file_set
            .get_or_init(|| self.files().iter().cloned().collect())
            .contains(relative)
    }
}

/// Whether an entry below the root is the top of another repository: a submodule, a
/// nested clone, or a worktree an agent keeps inside this checkout (ADR-0025).
///
/// Git does not track what is inside one, so neither does this: its documents make
/// claims about their own repository, and its files are not evidence for this one's.
/// Walking into an agent's worktree made a session in the main checkout lint that
/// worktree's documents as its own, against this repository's history.
///
/// The walk never filters its own root, which is the one directory here whose `.git`
/// belongs to this repository.
fn nested_repository(entry: &ignore::DirEntry) -> bool {
    entry.file_type().is_some_and(|t| t.is_dir()) && entry.path().join(".git").exists()
}

/// One walk of the working tree: what it found, and what it could not read.
struct Walk {
    files: Vec<PathBuf>,
    errors: Vec<String>,
}

/// Collapse `.` and `..` without touching the filesystem, refusing anything that
/// escapes the repository root.
pub fn normalise(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            std::path::Component::Normal(part) => out.push(part),
            _ => return None,
        }
    }
    Some(out)
}
