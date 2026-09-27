//! Seam: `gate` decides whether a broken claim deserves a human's attention.
//! History is part of the logic, so these fixtures are real git repositories.

use std::path::Path;
use std::process::Command;

use stilltrue::claim::{Claim, ClaimKind};
use stilltrue::gate::{self, Tier};
use stilltrue::git::{Git, Subprocess};
use stilltrue::resolution::{Evidence, Needle};

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git");
    assert!(status.status.success(), "git {args:?} failed");
}

/// A repository where `make demo` was once real and then was not.
fn repo_where_demo_was_removed() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    run(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("Makefile"), "demo:\n\techo demo\n").unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-q", "-m", "add demo target"]);
    std::fs::write(
        p.join("Makefile"),
        "seed:\n\techo seed\n\nserve:\n\techo serve\n",
    )
    .unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-q", "-m", "split demo into seed+serve"]);
    dir
}

fn claim() -> Claim {
    Claim {
        kind: ClaimKind::Command {
            runner: "make".into(),
            args: vec!["demo".into()],
        },
        text: "make demo".into(),
        file: Path::new("CLAUDE.md").to_path_buf(),
        line: 14,
        column: 8,
        end_line: 14,
        end_column: 17,
        span: 0..9,
    }
}

fn evidence(pattern: &str, candidates: &[&str]) -> Evidence {
    Evidence {
        needle: Some(Needle::Regex {
            pattern: pattern.to_string(),
            scope: vec!["Makefile".to_string()],
        }),
        candidates: candidates.iter().map(|s| s.to_string()).collect(),
        message: "command `make demo` has no target in Makefile".to_string(),
    }
}

#[test]
fn a_needle_that_history_proves_once_existed_is_rot() {
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    let finding = gate::gate(&claim(), &evidence(r"^demo\s*:", &[]), &git, false)
        .expect("a Tier A finding should be reported by default");
    assert_eq!(finding.tier, Tier::Rot);
    assert_eq!(finding.rule_id, "stilltrue/command/rot");
    let commit = finding
        .breaking_commit
        .expect("rot names the likely commit");
    assert_eq!(commit.subject, "split demo into seed+serve");
}

#[test]
fn a_needle_with_no_history_is_a_lie_and_is_silent_by_default() {
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    let ev = evidence(r"^never-existed\s*:", &[]);
    assert!(gate::gate(&claim(), &ev, &git, false).is_none());

    let strict = gate::gate(&claim(), &ev, &git, true).expect("strict reports Tier B");
    assert_eq!(strict.tier, Tier::Lie);
    assert_eq!(strict.rule_id, "stilltrue/command/lie");
    assert!(strict.breaking_commit.is_none());
}

#[test]
fn a_claim_with_no_definition_shaped_needle_is_never_reported() {
    // ADR-0005: no definition-shaped needle means Tier C, unreachable by any flag.
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    let ev = Evidence {
        needle: None,
        candidates: vec![],
        message: "whatever".into(),
    };
    assert!(gate::gate(&claim(), &ev, &git, false).is_none());
    assert!(gate::gate(&claim(), &ev, &git, true).is_none());
}

#[test]
fn without_history_nothing_can_reach_rot() {
    // A degraded run reports nothing by default; it must never mislabel (ADR-0006).
    let dir = tempfile::tempdir().unwrap();
    let git = Subprocess::new(dir.path());
    assert!(!git.available());
    let ev = evidence(r"^demo\s*:", &[]);
    assert!(gate::gate(&claim(), &ev, &git, false).is_none());
    assert_eq!(
        gate::gate(&claim(), &ev, &git, true).map(|f| f.tier),
        Some(Tier::Lie)
    );
}

#[test]
fn a_small_candidate_set_is_enumerated_rather_than_guessed_at() {
    // When a Makefile has two targets, naming both is not a guess — it is telling the
    // reader what exists. This is what makes `did you mean: seed, serve?` reachable;
    // `demo` is edit distance 3 and 4 from them, so no distance rule would find them.
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    let ev = evidence(r"^demo\s*:", &["seed", "serve"]);
    let finding = gate::gate(&claim(), &ev, &git, false).unwrap();
    assert_eq!(
        finding.suggestions,
        vec!["seed".to_string(), "serve".to_string()]
    );
}

#[test]
fn a_large_candidate_set_falls_back_to_near_matches_only() {
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    let many: Vec<&str> = vec![
        "demos",
        "seed",
        "serve",
        "publish-documentation",
        "lint",
        "check",
        "release",
    ];
    let ev = evidence(r"^demo\s*:", &many);
    let finding = gate::gate(&claim(), &ev, &git, false).unwrap();
    assert_eq!(finding.suggestions, vec!["demos".to_string()]);
}

#[test]
fn suggestions_are_dropped_entirely_for_an_enormous_candidate_set() {
    // In a large symbol index, coincidental near-matches stop being informative.
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    let many: Vec<String> = (0..250).map(|i| format!("demo{i}")).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let finding = gate::gate(&claim(), &evidence(r"^demo\s*:", &refs), &git, false).unwrap();
    assert!(finding.suggestions.is_empty());
}

#[test]
fn a_path_needle_is_searched_literally_not_as_a_pathspec() {
    // `git log -- 'docs/**'` treats its argument as a glob and matches real commits,
    // which manufactures a Tier A finding for a path that never existed.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    run(p, &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(p.join("docs")).unwrap();
    std::fs::write(p.join("docs/setup.md"), "# Install\n").unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-q", "-m", "add docs"]);

    let git = Subprocess::new(p);
    let literal = git.search(&Needle::Path {
        path: Path::new("docs/**").to_path_buf(),
    });
    assert!(
        literal.is_none(),
        "a glob must not match history, got {literal:?}"
    );

    let real = git.search(&Needle::Path {
        path: Path::new("docs/setup.md").to_path_buf(),
    });
    assert!(real.is_some(), "a real path must still be found");
}

/// The ADR-0001 contract end to end: a needle a *resolver* produced must actually find
/// history. Hand-written needles in the tests above cannot catch a dialect mismatch —
/// git's pickaxe is POSIX ERE, where `\b` matches nothing and `\s` is a literal `s`.
mod resolver_needles_work_against_real_git {
    use super::*;
    use stilltrue::repo::Repo;
    use stilltrue::resolution::Resolution;

    fn rot_for(claim: &Claim, files_before: &[(&str, &str)], files_after: &[(&str, &str)]) -> Tier {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        run(p, &["init", "-q", "-b", "main"]);
        for (name, body) in files_before {
            std::fs::create_dir_all(p.join(name).parent().unwrap()).unwrap();
            std::fs::write(p.join(name), body).unwrap();
        }
        run(p, &["add", "-A"]);
        run(p, &["commit", "-q", "-m", "before"]);
        for (name, body) in files_after {
            std::fs::write(p.join(name), body).unwrap();
        }
        run(p, &["add", "-A"]);
        run(p, &["commit", "-q", "-m", "after"]);

        let repo = Repo::new(p);
        let evidence = match stilltrue::resolve::resolve(claim, &repo) {
            Resolution::Broken(e) => e,
            other => panic!("expected Broken, got {other:?}"),
        };
        let git = Subprocess::new(p);
        gate::gate(claim, &evidence, &git, true)
            .expect("a finding")
            .tier
    }

    fn bare(kind: ClaimKind, text: &str) -> Claim {
        Claim {
            kind,
            text: text.into(),
            file: Path::new("CLAUDE.md").to_path_buf(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1 + text.chars().count(),
            span: 0..text.len(),
        }
    }

    #[test]
    fn a_removed_typescript_definition_is_rot() {
        // The coupling test, for the second language: the index and the needle have to
        // describe the same set, and only real git can say whether they do.
        let tier = rot_for(
            &bare(ClaimKind::Symbol, "foldEvents()"),
            &[
                ("src/index.ts", "export function foldEvents() {}\n"),
                ("src/two.ts", "export const two = 2;\n"),
            ],
            &[("src/index.ts", "export function collapseEvents() {}\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }

    #[test]
    fn a_typescript_const_removed_from_code_is_rot() {
        let tier = rot_for(
            &bare(ClaimKind::Symbol, "registry"),
            &[
                ("src/index.ts", "export const registry = new Map();\n"),
                ("src/two.ts", "export const two = 2;\n"),
            ],
            &[("src/index.ts", "export const store = new Map();\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }

    #[test]
    fn a_retitled_heading_is_rot() {
        // Retitling a heading is the commonest documentation refactor there is. The
        // file half of a link was checked and the anchor half thrown away, so this —
        // 41% of the relative links in the corpus carry an anchor — was unreportable
        // at any flag.
        let claim = Claim {
            kind: ClaimKind::Link {
                target: "docs/guide.md".into(),
                anchor: Some("union-modes".into()),
            },
            text: "docs/guide.md#union-modes".into(),
            file: Path::new("CLAUDE.md").to_path_buf(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 26,
            span: 0..25,
        };
        let tier = rot_for(
            &claim,
            &[("docs/guide.md", "# Guide\n\n## Union Modes\n")],
            &[("docs/guide.md", "# Guide\n\n## Unions\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }

    #[test]
    fn an_anchor_that_is_a_prefix_of_a_real_heading_is_not_rot() {
        // `#install` against `## Installation`: the heading slugs to `installation`,
        // so the anchor never existed. An unbounded needle matched it anyway and
        // blamed the commit that *added* the heading — a Tier A false positive, on by
        // default, from the check that reports broken anchors.
        let claim = Claim {
            kind: ClaimKind::Link {
                target: "guide.md".into(),
                anchor: Some("install".into()),
            },
            text: "guide.md#install".into(),
            file: Path::new("CLAUDE.md").to_path_buf(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 17,
            span: 0..16,
        };
        let tier = rot_for(
            &claim,
            &[("guide.md", "# Guide\n\n## Installation\n")],
            &[("guide.md", "# Guide\n\n## Installation notes\n")],
        );
        assert_eq!(tier, Tier::Lie);
    }

    #[test]
    fn an_anchor_that_never_existed_is_only_a_lie() {
        // The near-miss: a heading needle must not manufacture history for an anchor
        // nobody ever wrote.
        let claim = Claim {
            kind: ClaimKind::Link {
                target: "docs/guide.md".into(),
                anchor: Some("never-written".into()),
            },
            text: "docs/guide.md#never-written".into(),
            file: Path::new("CLAUDE.md").to_path_buf(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 28,
            span: 0..27,
        };
        let tier = rot_for(
            &claim,
            &[("docs/guide.md", "# Guide\n\n## Union Modes\n")],
            &[("docs/guide.md", "# Guide\n\n## Unions\n")],
        );
        assert_eq!(tier, Tier::Lie);
    }

    #[test]
    fn history_on_an_unmerged_branch_is_not_evidence() {
        // The search is `git log --pickaxe-regex -S<needle> -- <scope>`.
        // Searching every ref instead would let an abandoned branch prove a claim, and
        // would make the answer depend on which refs a CI checkout happened to fetch.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        run(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("app.py"), "def other():\n    pass\n").unwrap();
        run(p, &["add", "-A"]);
        run(p, &["commit", "-q", "-m", "main"]);

        run(p, &["checkout", "-q", "-b", "abandoned"]);
        std::fs::write(p.join("app.py"), "def fold_events():\n    pass\n").unwrap();
        run(p, &["add", "-A"]);
        run(p, &["commit", "-q", "-m", "on a branch nobody merged"]);
        run(p, &["checkout", "-q", "main"]);

        let claim = bare(ClaimKind::Symbol, "fold_events()");
        let repo = Repo::new(p);
        let evidence = match stilltrue::resolve::resolve(&claim, &repo) {
            Resolution::Broken(e) => e,
            other => panic!("expected Broken, got {other:?}"),
        };
        let git = Subprocess::new(p);
        let tier = gate::gate(&claim, &evidence, &git, true)
            .expect("a finding")
            .tier;
        assert_eq!(tier, Tier::Lie);
    }

    #[test]
    fn an_env_var_that_only_ever_appeared_in_prose_is_not_rot() {
        // The index refuses prose as evidence, so the needle must refuse it too.
        // click documents `WEB_RUN_RELOAD` as a worked example of `auto_envvar_prefix`;
        // its only history is a commit that moved the docs section. An unscoped needle
        // reads that as proof the variable once existed and promotes it to Tier A.
        let tier = rot_for(
            &bare(ClaimKind::EnvVar, "WEB_RUN_RELOAD"),
            &[
                ("app.py", "x = 1\n"),
                ("docs/guide.md", "Set `WEB_RUN_RELOAD` to reload.\n"),
            ],
            &[("docs/guide.md", "Reloading is described in the context.\n")],
        );
        assert_eq!(tier, Tier::Lie);
    }

    #[test]
    fn an_env_var_removed_from_code_is_still_rot() {
        // The near-miss for the rule above: code is evidence, and losing it is rot.
        let tier = rot_for(
            &bare(ClaimKind::EnvVar, "WEB_RUN_RELOAD"),
            &[("app.py", "import os\nos.environ[\"WEB_RUN_RELOAD\"]\n")],
            &[("app.py", "import os\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }

    #[test]
    fn a_removed_python_definition_is_rot() {
        let tier = rot_for(
            &bare(ClaimKind::Symbol, "fold_events()"),
            &[
                ("app.py", "def fold_events(x):\n    return x\n"),
                ("CLAUDE.md", ""),
            ],
            &[("app.py", "def other(x):\n    return x\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }

    #[test]
    fn a_removed_env_var_is_rot() {
        let tier = rot_for(
            &bare(ClaimKind::EnvVar, "GEMINI_API_KEY"),
            &[
                ("app.py", "KEY = os.environ[\"GEMINI_API_KEY\"]\n"),
                ("CLAUDE.md", ""),
            ],
            &[("app.py", "KEY = 1\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }

    #[test]
    fn a_removed_make_target_is_rot() {
        let tier = rot_for(
            &bare(
                ClaimKind::Command {
                    runner: "make".into(),
                    args: vec!["demo".into()],
                },
                "make demo",
            ),
            &[("Makefile", "demo:\n\techo\n"), ("CLAUDE.md", "")],
            &[("Makefile", "seed:\n\techo\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }

    #[test]
    fn a_removed_npm_script_is_rot() {
        let tier = rot_for(
            &bare(
                ClaimKind::Command {
                    runner: "npm".into(),
                    args: vec!["run".into(), "build".into()],
                },
                "npm run build",
            ),
            &[
                ("package.json", "{\"scripts\": {\"build\": \"tsc\"}}\n"),
                ("CLAUDE.md", ""),
            ],
            &[("package.json", "{\"scripts\": {\"compile\": \"tsc\"}}\n")],
        );
        assert_eq!(tier, Tier::Rot);
    }
}

#[test]
fn a_relative_date_is_computed_at_render_time_not_stored() {
    // A cached positive result never expires, so a stored `%cr` string would keep
    // saying "4 months ago" forever.
    use stilltrue::report::relative_date;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert_eq!(relative_date(now - 30), "30 seconds ago");
    // git also says "60 minutes ago" at exactly one hour; hours start above 90 minutes.
    assert_eq!(relative_date(now - 3600), "60 minutes ago");
    assert_eq!(relative_date(now - 2 * 3600), "2 hours ago");
    assert_eq!(relative_date(now - 3 * 86_400), "3 days ago");
    assert_eq!(relative_date(now - 21 * 86_400), "3 weeks ago");
    assert_eq!(relative_date(now - 4 * 2_629_800), "4 months ago");
    assert_eq!(relative_date(now - 2 * 12 * 2_629_800), "2 years ago");
}

// ---------------------------------------------------------------------------------
// The suggestion thresholds. Each is a single number, and each divides "telling the
// reader what exists" from "guessing at what they meant" — the line every suggestion
// rule is about. The `make demo` claim above measures against its last word, `demo`.
// ---------------------------------------------------------------------------------

/// The suggestions a broken claim carries, given this candidate set.
fn suggested(candidates: &[String]) -> Vec<String> {
    suggested_for("make demo", candidates)
}

/// The same, for a claim whose last word is the needle measured against.
fn suggested_for(text: &str, candidates: &[String]) -> Vec<String> {
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path().to_path_buf());
    let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
    let mut c = claim();
    c.text = text.to_string();
    gate::gate(&c, &evidence(r"^demo[[:space:]]*:", &refs), &git, false)
        .expect("the claim is rot")
        .suggestions
}

fn names(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}{i:04}")).collect()
}

#[test]
fn a_set_of_five_or_fewer_is_enumerated_entirely() {
    // Not "the best of five" — all of them. Showing three of five would be guessing
    // again, just silently. None of these is near `demo` by distance, which is the
    // point: at this size the tool is reporting what exists, not ranking.
    for n in 1..=5 {
        let candidates = names("zzz", n);
        let out = suggested(&candidates);
        assert_eq!(out.len(), n, "{n} candidates should all be named: {out:?}");
        assert_eq!(out, candidates, "and in sorted order");
    }
}

#[test]
fn a_set_of_six_switches_to_distance_and_shows_at_most_three() {
    // One past the boundary, the tool stops enumerating and starts ranking.
    let far = names("zzz", 6);
    assert!(
        suggested(&far).is_empty(),
        "nothing within distance of `demo` should yield nothing"
    );

    let mut near = names("zzz", 5);
    near.extend(["demo1".into(), "demo2".into(), "demp".into(), "dem".into()]);
    let out = suggested(&near);
    assert_eq!(out.len(), 3, "at most three, got {out:?}");
    assert!(
        out.iter().all(|s| s.starts_with("dem")),
        "ranked by distance, not enumerated: {out:?}"
    );
}

#[test]
fn distance_is_two_for_a_short_name_and_a_tenth_for_a_long_one() {
    // `demo` is four characters, so the limit is 2 rather than 30% of 4.
    let mut set = names("zzz", 5);
    set.push("demox".into()); // distance 1
    set.push("demoxy".into()); // distance 2
    set.push("demoxyz".into()); // distance 3 — past the limit
    let out = suggested(&set);
    assert!(out.contains(&"demox".to_string()), "{out:?}");
    assert!(out.contains(&"demoxy".to_string()), "{out:?}");
    assert!(
        !out.contains(&"demoxyz".to_string()),
        "distance 3 is too far: {out:?}"
    );
}

#[test]
fn a_long_name_is_allowed_a_proportional_distance() {
    // Past eight characters the limit stops being a flat 2 and becomes three tenths of
    // the name's length — `deployment-check` is sixteen, so four edits. A flat 2 would
    // refuse every real near-miss in a long name.
    let mut set = names("zzz", 5);
    set.push("deployment-chek".into()); // distance 1
    set.push("deployment-ch".into()); // distance 3
    // Distance 6: inside three tenths of sixteen only if the tenths are computed
    // wrongly, which is the whole reason this candidate is here rather than a far one.
    set.push("deployment".into());
    set.push("deploy".into()); // distance 10 — far past the limit

    let out = suggested_for("make deployment-check", &set);
    assert!(out.contains(&"deployment-chek".to_string()), "{out:?}");
    assert!(out.contains(&"deployment-ch".to_string()), "{out:?}");
    assert!(
        !out.contains(&"deployment".to_string()),
        "distance 6 is past the limit of 4: {out:?}"
    );
    assert!(
        !out.contains(&"deploy".to_string()),
        "distance 10 is past three tenths of sixteen: {out:?}"
    );

    // The same candidates against a four-character name keep the flat limit of 2, so
    // none of them is near enough to offer.
    assert!(
        suggested_for("make demo", &set).is_empty(),
        "a short name allows 2 edits, not 4"
    );
}

#[test]
fn a_candidate_set_large_enough_for_coincidence_yields_nothing() {
    // At 200 the tool still speaks; at 201 a near match stops being informative.
    let mut at_limit = names("zzz", 199);
    at_limit.push("demox".into());
    assert_eq!(at_limit.len(), 200);
    assert_eq!(
        suggested(&at_limit),
        vec!["demox".to_string()],
        "200 is still small enough to rank"
    );

    let mut past_limit = at_limit.clone();
    past_limit.push("zzzextra".into());
    assert_eq!(past_limit.len(), 201);
    assert!(
        suggested(&past_limit).is_empty(),
        "past 200, coincidental near matches stop being informative"
    );
}

#[test]
fn suggestions_are_ordered_so_that_snapshots_are_stable() {
    let mut set = names("zzz", 5);
    set.extend(["demp".into(), "demq".into(), "demn".into()]);
    assert_eq!(
        suggested(&set),
        vec!["demn".to_string(), "demp".to_string(), "demq".to_string()],
        "equal distance sorts lexically"
    );
}

#[test]
fn a_degraded_clone_answers_nothing_rather_than_answering_from_its_tip() {
    // A shallow clone has history — one commit of it — and `git log -S` treats that tip
    // as a root, so everything in it reads as added there. Asking anyway would return
    // the tip for any needle the working tree satisfies, and that answer would be
    // dressed as "likely broke in", pointing at a commit that broke nothing.
    //
    // The guard in `search` is what stops it. Without it a shallow CI checkout would
    // not merely under-report, which ADR-0006 accepts loudly; it would misattribute.
    let dir = repo_where_demo_was_removed();
    let shallow = tempfile::tempdir().unwrap();
    let url = format!("file://{}", dir.path().display());
    let status = Command::new("git")
        .args(["clone", "-q", "--depth", "1", &url])
        .arg(shallow.path().join("clone"))
        .output()
        .expect("git clone");
    assert!(status.status.success(), "clone failed");

    let clone = shallow.path().join("clone");
    let git = Subprocess::new(clone.clone());
    assert!(git.shallow(), "the clone should be shallow");

    // `seed` is in the tip's tree, so an unguarded search would find it there.
    let needle = Needle::Regex {
        pattern: r"^seed[[:space:]]*:".to_string(),
        scope: vec!["Makefile".to_string()],
    };
    assert!(
        git.search(&needle).is_none(),
        "a shallow clone must not answer from its own tip"
    );

    // And the full clone does answer, so the test above is about the guard rather than
    // about the needle finding nothing anywhere.
    let full = Subprocess::new(dir.path().to_path_buf());
    assert!(
        full.search(&needle).is_some(),
        "the same needle resolves in a complete clone"
    );
}

/// The current HEAD sha of `dir`.
fn head_of(dir: &Path) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn reachability_is_what_keeps_a_cached_commit_honest() {
    // `contains` exists so the cache cannot report a commit that a rewrite dropped
    // (ADR-0007, amended). Every answer it gives has to be its own: replacing the whole
    // function with `true` or with `false` was invisible until this test, and `true` is
    // the dangerous one — it is the answer that lets stale attribution through.
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());

    let head = head_of(dir.path());
    assert!(git.contains(&head), "HEAD is reachable from HEAD");

    let parent = {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(["rev-parse", "HEAD~1"])
            .output()
            .expect("git");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert!(git.contains(&parent), "so is its parent");

    // Rewrite history: the old tip is still an object in the repository, and is no
    // longer in it. That distinction is the whole point — `git cat-file -e` would say
    // yes here, which is why the check is an ancestry test and not an existence test.
    let orphaned = head;
    run(dir.path(), &["commit", "-q", "--amend", "-m", "reworded"]);
    let git = Subprocess::new(dir.path());
    assert!(
        !git.contains(&orphaned),
        "an amended-away commit is not reachable, though the object survives",
    );
    assert!(git.contains(&head_of(dir.path())), "and the new tip is",);

    assert!(
        !git.contains("0000000000000000000000000000000000000000"),
        "nor is a sha that was never in this repository",
    );
}

#[test]
fn a_clone_that_cannot_answer_says_no() {
    // `false` is the safe answer: on a yes the caller reuses the stored attribution, so
    // a repository that cannot check has to decline rather than agree. A shallow clone
    // holds a truncated history, where "not an ancestor" means "not in the part I was
    // given" — a different question.
    let source = repo_where_demo_was_removed();
    let head = head_of(source.path());

    let shallow_dir = tempfile::tempdir().unwrap();
    let dest = shallow_dir.path().join("shallow");
    let out = Command::new("git")
        .args(["clone", "-q", "--depth", "1"])
        .arg(format!("file://{}", source.path().display()))
        .arg(&dest)
        .output()
        .expect("git");
    assert!(out.status.success(), "shallow clone failed");

    let shallow = Subprocess::new(&dest);
    assert!(shallow.shallow(), "the fixture is actually shallow");
    assert!(
        !shallow.contains(&head_of(&dest)),
        "a shallow clone declines even about its own tip",
    );

    // Not a repository at all: nothing is available, so nothing is reachable.
    let empty = tempfile::tempdir().unwrap();
    let absent = Subprocess::new(empty.path());
    assert!(!absent.contains(&head), "no git, no answer");
}

// ---------------------------------------------------------------------------------
// ADR-0020: a failed history search is not an answer.
// ---------------------------------------------------------------------------------

/// A `Git` whose every search fails.
struct Failing;

impl Git for Failing {
    fn available(&self) -> bool {
        true
    }
    fn shallow(&self) -> bool {
        false
    }
    fn search(&self, _needle: &Needle) -> Option<stilltrue::git::Commit> {
        None
    }
    fn try_search(
        &self,
        _needle: &Needle,
    ) -> Result<Option<stilltrue::git::Commit>, stilltrue::git::HistoryError> {
        Err(stilltrue::git::HistoryError)
    }
}

#[test]
fn a_failed_search_is_history_failed_not_a_lie_under_strict() {
    // Under --strict a lie is reported. A search that failed has found no evidence
    // either way, so reporting it as "never existed" would be a false finding.
    let judgement = gate::judge(&claim(), &evidence("^demo:", &[]), &Failing, true);
    assert!(
        matches!(judgement, gate::Judgement::HistoryFailed),
        "{judgement:?}"
    );
    assert!(gate::gate(&claim(), &evidence("^demo:", &[]), &Failing, true).is_none());
}

#[test]
fn a_needleless_claim_is_abstained_whatever_git_would_say() {
    let mut needleless = evidence("^demo:", &[]);
    needleless.needle = None;
    assert!(matches!(
        gate::judge(&claim(), &needleless, &Failing, true),
        gate::Judgement::Abstained
    ));
}

#[test]
fn an_unreported_lie_is_counted_rather_than_reported() {
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    // `^never:` was never in the Makefile: a lie.
    let default = gate::judge(&claim(), &evidence("^never:", &[]), &git, false);
    assert!(
        matches!(default, gate::Judgement::Unreported(Tier::Lie)),
        "{default:?}"
    );
    let strict = gate::judge(&claim(), &evidence("^never:", &[]), &git, true);
    assert!(
        matches!(&strict, gate::Judgement::Reported(f) if f.tier == Tier::Lie),
        "{strict:?}"
    );
}

#[test]
fn a_git_that_exits_non_zero_is_a_failed_search_not_an_empty_one() {
    // `(` is not a valid ERE, so git's pickaxe refuses it and exits 128. That is the
    // real failure path, through the real subprocess, rather than a fake's say-so.
    let dir = repo_where_demo_was_removed();
    let git = Subprocess::new(dir.path());
    let broken = Needle::Regex {
        pattern: "(".into(),
        scope: vec!["Makefile".into()],
    };
    assert_eq!(git.try_search(&broken), Err(stilltrue::git::HistoryError));
    // And a search that genuinely finds nothing is not a failure.
    let empty = Needle::Regex {
        pattern: "^never:".into(),
        scope: vec!["Makefile".into()],
    };
    assert_eq!(git.try_search(&empty), Ok(None));
}

#[test]
fn a_non_utf8_commit_subject_does_not_hide_the_commit() {
    // A Latin-1 subject is still a commit. Strict decoding threw the whole log line
    // away, and the claim it proved fell from rot to "never existed".
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    run(p, &["init", "-q", "-b", "main"]);
    run(p, &["config", "i18n.commitEncoding", "ISO-8859-1"]);
    std::fs::write(p.join("Makefile"), "demo:\n\techo demo\n").unwrap();
    run(p, &["add", "-A"]);
    let message = p.join("message");
    std::fs::write(&message, b"caf\xe9 demo\n").unwrap();
    run(
        p,
        &[
            "commit",
            "-q",
            "-F",
            message.to_str().unwrap(),
            "--cleanup=verbatim",
        ],
    );
    std::fs::write(p.join("Makefile"), "seed:\n\techo seed\n").unwrap();
    run(p, &["add", "Makefile"]);
    std::fs::write(&message, b"d\xe9mo removed\n").unwrap();
    run(
        p,
        &[
            "commit",
            "-q",
            "-F",
            message.to_str().unwrap(),
            "--cleanup=verbatim",
        ],
    );
    let git = Subprocess::new(p);
    let found = git
        .try_search(&Needle::Regex {
            pattern: "^demo[[:space:]]*:".into(),
            scope: vec!["Makefile".into()],
        })
        .expect("the search ran");
    assert!(
        found.is_some(),
        "the commit was lost to its subject's encoding"
    );
}
