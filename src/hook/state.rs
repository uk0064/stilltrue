//! Session state: disposable, outside the repository, one file per worktree and host
//! session.
//!
//! Keyed so that two worktrees, or two sessions in one worktree, never share a file —
//! otherwise one session's findings become another's "pre-existing backlog". Written
//! atomically, bounded in count and age, and never a reason to fail: a state file that
//! cannot be read starts a new comparison epoch, and says so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::hook::policy::Finding;

pub const SCHEMA: u32 = 1;
/// Inactive state older than this is removed.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// At most this many sessions are remembered; the least recently used go first.
pub const MAX_SESSIONS: usize = 256;

/// One scan, as a session remembers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scan {
    /// `complete`, `incomplete`, `no-input`, `timed-out`, `missing`, `malformed` or
    /// `failed`.
    pub completion: String,
    /// The working-tree identity the scan started from.
    pub identity: String,
    pub head: Option<String>,
    pub findings: Vec<Finding>,
}

impl Scan {
    pub fn complete(&self) -> bool {
        self.completion == "complete"
    }
}

/// A comparison epoch: the span over which "new" means new since `initial`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Epoch {
    pub id: u32,
    pub started: i64,
    /// Why this epoch began: `new-session`, `cleared`, `forked`, `state-missing`,
    /// `tool-changed`, `config-changed`, `branch-changed`, `baseline-established`.
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    /// Continuations requested in this turn. At most one is ever requested.
    pub continuations: u32,
    pub interrupted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub schema: u32,
    pub host: String,
    pub session: String,
    pub worktree: PathBuf,
    pub analysis: String,
    pub config: String,
    pub branch: String,
    pub epoch: Epoch,
    /// What "new" is measured against. Absent or incomplete means no comparison is
    /// available until a complete scan establishes one.
    pub initial: Option<Scan>,
    /// Findings still unresolved when the previous epoch ended. Advisory context only:
    /// they are neither new nor this session's backlog.
    pub carried: Vec<Finding>,
    pub last: Option<Scan>,
    pub turn: Turn,
    /// Digest of the last message shown, so an unchanged one is not repeated.
    pub last_message: Option<String>,
    /// Advice from a completion check, to be given at the next prompt where the host
    /// can carry context.
    pub pending: Option<String>,
    /// How many times each event reached this session, for diagnosis.
    pub events: BTreeMap<String, u32>,
    pub updated: i64,
}

/// Where a key's state was, when asked for.
#[derive(Debug)]
pub enum Load {
    Missing,
    /// Present but unusable: unparseable, or written by another schema.
    Unusable,
    Found(Box<State>),
}

pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The default location: beside the history cache, which `STILLTRUE_CACHE_DIR`
    /// moves as well.
    pub fn default_dir() -> PathBuf {
        crate::cache::cache_root()
            .unwrap_or_else(|| std::env::temp_dir().join("stilltrue"))
            .join("sessions")
    }

    /// One file per worktree and host session.
    pub fn key(worktree: &Path, host: &str, session: &str) -> String {
        format!(
            "{}-{}",
            crate::cache::fnv1a(&worktree.to_string_lossy()),
            crate::cache::fnv1a(&format!("{host}\u{1f}{session}"))
        )
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// Where the full report of a scan is kept for diagnosis: the entry scan and the
    /// latest one, per session.
    pub fn report_path(&self, key: &str, which: &str) -> PathBuf {
        self.dir.join("reports").join(format!("{key}-{which}.json"))
    }

    pub fn load(&self, key: &str) -> Load {
        let Ok(text) = std::fs::read_to_string(self.path(key)) else {
            return Load::Missing;
        };
        match serde_json::from_str::<State>(&text) {
            Ok(state) if state.schema == SCHEMA => Load::Found(Box::new(state)),
            _ => Load::Unusable,
        }
    }

    /// Replace the state atomically: a sibling written, then renamed into place.
    pub fn save(&self, key: &str, state: &State) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(key);
        let temporary = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        let json = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
        std::fs::write(&temporary, json)?;
        std::fs::rename(&temporary, &path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }

    /// Expire inactive sessions and bound how many are kept.
    pub fn prune(&self) {
        let now = SystemTime::now();
        let age = |path: &Path| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .unwrap_or_default()
        };
        let listed = |dir: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_file())
                        .collect()
                })
                .unwrap_or_default()
        };
        let reports = self.dir.join("reports");
        for path in listed(&self.dir).into_iter().chain(listed(&reports)) {
            if age(&path) > MAX_AGE {
                let _ = std::fs::remove_file(&path);
            }
        }
        let mut sessions: Vec<PathBuf> = listed(&self.dir)
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        if sessions.len() <= MAX_SESSIONS {
            return;
        }
        sessions.sort_by_key(|p| std::cmp::Reverse(age(p)));
        let excess = sessions.len() - MAX_SESSIONS;
        for path in sessions.into_iter().take(excess) {
            if let Some(key) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) {
                for which in ["entry", "latest"] {
                    let _ = std::fs::remove_file(self.report_path(&key, which));
                }
            }
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(session: &str) -> State {
        State {
            schema: SCHEMA,
            host: "claude-code".into(),
            session: session.into(),
            worktree: "/w".into(),
            analysis: "a".into(),
            config: "c".into(),
            branch: "main".into(),
            epoch: Epoch {
                id: 1,
                started: 0,
                reason: "new-session".into(),
            },
            initial: None,
            carried: vec![],
            last: None,
            turn: Turn::default(),
            last_message: None,
            pending: None,
            events: BTreeMap::new(),
            updated: 0,
        }
    }

    #[test]
    fn worktrees_hosts_and_sessions_each_get_their_own_key() {
        let a = Store::key(Path::new("/w/one"), "claude-code", "s1");
        assert_ne!(a, Store::key(Path::new("/w/two"), "claude-code", "s1"));
        assert_ne!(a, Store::key(Path::new("/w/one"), "codex", "s1"));
        assert_ne!(a, Store::key(Path::new("/w/one"), "claude-code", "s2"));
        assert_eq!(a, Store::key(Path::new("/w/one"), "claude-code", "s1"));
    }

    #[test]
    fn a_saved_state_loads_back_and_a_damaged_one_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        assert!(matches!(store.load("k"), Load::Missing));
        store.save("k", &state("s")).unwrap();
        assert!(matches!(store.load("k"), Load::Found(s) if *s == state("s")));
        std::fs::write(dir.path().join("k.json"), "{\"schema\":").unwrap();
        assert!(matches!(store.load("k"), Load::Unusable));
        let mut future = state("s");
        future.schema = SCHEMA + 1;
        std::fs::write(
            dir.path().join("k.json"),
            serde_json::to_string(&future).unwrap(),
        )
        .unwrap();
        assert!(matches!(store.load("k"), Load::Unusable));
    }

    #[test]
    fn saving_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.save("k", &state("s")).unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["k.json"]);
    }

    fn age(path: &Path, by: Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - by).unwrap();
    }

    #[test]
    fn a_week_is_the_limit_in_days_not_just_relative_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let day = Duration::from_secs(24 * 60 * 60);
        store.save("six-days", &state("a")).unwrap();
        age(&dir.path().join("six-days.json"), day * 6);
        store.save("eight-days", &state("b")).unwrap();
        age(&dir.path().join("eight-days.json"), day * 8);
        store.prune();
        assert!(matches!(store.load("six-days"), Load::Found(_)));
        assert!(matches!(store.load("eight-days"), Load::Missing));
    }

    #[test]
    fn only_a_complete_scan_is_complete() {
        let scan = |completion: &str| Scan {
            completion: completion.into(),
            identity: String::new(),
            head: None,
            findings: vec![],
        };
        assert!(scan("complete").complete());
        for other in [
            "incomplete",
            "no-input",
            "timed-out",
            "missing",
            "malformed",
            "failed",
            "stale",
        ] {
            assert!(!scan(other).complete(), "{other}");
        }
    }

    #[test]
    fn session_state_lives_beside_the_history_cache() {
        let dir = Store::default_dir();
        assert!(dir.ends_with("sessions"), "{}", dir.display());
        assert_eq!(dir.parent(), crate::cache::cache_root().as_deref());
    }

    #[test]
    fn inactive_state_expires_and_the_count_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.save("old", &state("old")).unwrap();
        age(
            &dir.path().join("old.json"),
            MAX_AGE + Duration::from_secs(60),
        );
        store.save("fresh", &state("fresh")).unwrap();
        store.prune();
        assert!(matches!(store.load("old"), Load::Missing));
        assert!(matches!(store.load("fresh"), Load::Found(_)));

        for i in 0..MAX_SESSIONS + 10 {
            let key = format!("s{i}");
            store.save(&key, &state(&key)).unwrap();
            // Oldest first: s0 was used longest ago.
            age(
                &dir.path().join(format!("{key}.json")),
                Duration::from_secs(10_000 - i as u64),
            );
        }
        store.prune();
        let kept = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().is_file())
            .count();
        assert_eq!(kept, MAX_SESSIONS);
        assert!(matches!(store.load("s0"), Load::Missing), "the oldest went");
        assert!(
            matches!(store.load("fresh"), Load::Found(_)),
            "the newest stayed"
        );
    }
}
