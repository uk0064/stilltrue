//! What a repository looked like at one moment, as a value another moment can be
//! compared with.
//!
//! HEAD alone is not a working-tree identity: an agent's edits are uncommitted, and a
//! shell command can change files no edit event reported. So the identity is the
//! content of every file the checker's own walk would see — gitignore-aware, untracked
//! files included, ignore files included — plus HEAD, because history decides tiers.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::repo::Repo;

/// Files larger than this are identified by size and modification time rather than
/// content. None of them is a document or a manifest a resolver reads; for them only
/// existence matters, and hashing a checked-in video on every stop is not free.
pub const LARGE: u64 = 8 * 1024 * 1024;

/// The repository a hook was invoked for.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Canonical worktree root. Two worktrees of one repository are two workspaces.
    pub root: PathBuf,
    pub head: Option<String>,
    /// The branch name, or `(detached)`.
    pub branch: String,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn workspace(cwd: &Path) -> Workspace {
    let root = git(cwd, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.to_path_buf());
    let root = root.canonicalize().unwrap_or(root);
    Workspace {
        head: git(&root, &["rev-parse", "HEAD"]),
        branch: git(&root, &["symbolic-ref", "-q", "--short", "HEAD"])
            .unwrap_or_else(|| "(detached)".to_string()),
        root,
    }
}

/// FNV-1a over bytes, streamed.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
        // A separator, so `ab` + `c` and `a` + `bc` differ.
        self.0 ^= 0xff;
        self.0 = self.0.wrapping_mul(0x100_0000_01b3);
    }

    fn finish(&self) -> String {
        format!("{:016x}", self.0)
    }
}

/// The content identity of the working tree at `root`, with `head`.
pub fn identity(root: &Path, head: Option<&str>) -> String {
    let repo = Repo::new(root);
    let mut hash = Fnv::new();
    hash.write(head.unwrap_or("-").as_bytes());
    for file in repo.files() {
        hash.write(file.to_string_lossy().as_bytes());
        let path = root.join(file);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > LARGE => {
                hash.write(&meta.len().to_le_bytes());
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                hash.write(&modified.to_le_bytes());
            }
            Ok(_) => match std::fs::read(&path) {
                Ok(bytes) => hash.write(&bytes),
                Err(_) => hash.write(b"\0unreadable"),
            },
            Err(_) => hash.write(b"\0missing"),
        }
    }
    for error in repo.walk_errors() {
        hash.write(error.as_bytes());
    }
    hash.finish()
}

/// Everything besides the documents that decides what a scan reports: the
/// configuration file and any baseline. A change here starts a new comparison epoch, so
/// editing a config cannot masquerade as the agent introducing rot.
pub fn configuration(root: &Path, baseline: Option<&Path>) -> String {
    let mut hash = Fnv::new();
    match std::fs::read(root.join("stilltrue.toml")) {
        Ok(bytes) => hash.write(&bytes),
        Err(_) => hash.write(b"\0absent"),
    }
    match baseline.map(|b| std::fs::read(root.join(b))) {
        Some(Ok(bytes)) => hash.write(&bytes),
        Some(Err(_)) => hash.write(b"\0unreadable baseline"),
        None => hash.write(b"\0no baseline"),
    }
    hash.finish()
}

/// Which analysis produced a result: this adapter's version and the checker binary it
/// ran, by path, size and modification time. An upgrade of either starts a new epoch.
pub fn analysis(checker: Option<&Path>) -> String {
    let mut hash = Fnv::new();
    hash.write(env!("CARGO_PKG_VERSION").as_bytes());
    if let Some(checker) = checker {
        hash.write(checker.to_string_lossy().as_bytes());
        if let Ok(meta) = std::fs::metadata(checker) {
            hash.write(&meta.len().to_le_bytes());
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            hash.write(&modified.to_le_bytes());
        }
    }
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "one\n").unwrap();
        dir
    }

    #[test]
    fn identity_follows_content_not_timestamps() {
        let dir = repo();
        let before = identity(dir.path(), Some("abc"));
        std::fs::write(dir.path().join("README.md"), "one\n").unwrap();
        assert_eq!(
            identity(dir.path(), Some("abc")),
            before,
            "rewriting the same bytes"
        );
        std::fs::write(dir.path().join("README.md"), "two\n").unwrap();
        assert_ne!(identity(dir.path(), Some("abc")), before);
    }

    #[test]
    fn identity_sees_untracked_files_heads_and_ignore_rules() {
        let dir = repo();
        let base = identity(dir.path(), Some("abc"));
        assert_ne!(identity(dir.path(), Some("def")), base, "HEAD moved");
        std::fs::write(dir.path().join("new.py"), "x = 1\n").unwrap();
        let with_file = identity(dir.path(), Some("abc"));
        assert_ne!(with_file, base, "an untracked file appeared");
        std::fs::write(dir.path().join(".ignore"), "new.py\n").unwrap();
        assert_ne!(
            identity(dir.path(), Some("abc")),
            with_file,
            "ignore state changed"
        );
    }

    #[test]
    fn identity_separates_names_from_contents() {
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("ab"), "c").unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("a"), "bc").unwrap();
        assert_ne!(identity(a.path(), None), identity(b.path(), None));
    }

    fn touch(path: &Path) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
    }

    #[test]
    fn a_large_file_is_identified_by_size_and_time_and_a_smaller_one_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.bin");
        // Up to and including LARGE bytes, content decides: a new timestamp alone is the
        // same file. Two mebibytes is well inside that; LARGE itself is the edge.
        for size in [2 * 1024 * 1024, LARGE] {
            std::fs::write(&path, vec![7u8; size as usize]).unwrap();
            let before = identity(dir.path(), None);
            touch(&path);
            assert_eq!(identity(dir.path(), None), before, "{size} bytes");
        }
        // Past it, size and time decide, and a new timestamp is a change.
        std::fs::write(&path, vec![7u8; LARGE as usize + 1]).unwrap();
        let before = identity(dir.path(), None);
        touch(&path);
        assert_ne!(identity(dir.path(), None), before, "LARGE + 1 bytes");
    }

    #[test]
    fn the_identity_is_fnv_1a_with_a_separator_after_each_field() {
        // An external witness, computed independently: a separator-free or differently
        // mixed hash would still distinguish trees, so only a known value pins it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        assert_eq!(identity(dir.path(), Some("abc")), EXPECTED);
    }

    /// FNV-1a 64 over `abc`, `a.txt` and `hello\n`, with `0xff` folded in after each.
    const EXPECTED: &str = "972e85581ac6fe15";

    #[test]
    fn the_analysis_names_the_checker_it_ran() {
        let dir = tempfile::tempdir().unwrap();
        let checker = dir.path().join("stilltrue");
        std::fs::write(&checker, "one").unwrap();
        let with = analysis(Some(&checker));
        assert_ne!(with, analysis(None));
        assert_eq!(
            with,
            analysis(Some(&checker)),
            "stable while nothing changes"
        );
        std::fs::write(&checker, "two, longer").unwrap();
        assert_ne!(
            analysis(Some(&checker)),
            with,
            "a replaced checker is another analysis"
        );
        assert_eq!(analysis(None).len(), 16);
    }

    #[test]
    fn configuration_changes_with_config_and_baseline() {
        let dir = repo();
        let none = configuration(dir.path(), None);
        std::fs::write(dir.path().join("stilltrue.toml"), "strict = true\n").unwrap();
        let with_config = configuration(dir.path(), None);
        assert_ne!(with_config, none);
        std::fs::write(dir.path().join("base"), "abc\n").unwrap();
        assert_ne!(
            configuration(dir.path(), Some(Path::new("base"))),
            with_config
        );
    }
}
