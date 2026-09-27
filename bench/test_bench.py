"""Tests for the agent-task benchmark harness.

    python3 -m unittest discover -s bench -p 'test_*.py'

Everything runs without a model: the end-to-end cases use the scripted agent and the
real `stilltrue-hook` and `stilltrue` binaries (build them with `cargo build`; those
cases skip, saying so, when the binaries are absent). No test starts a live host.
"""

from __future__ import annotations

import ast
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

BENCH = Path(__file__).resolve().parent
ROOT = BENCH.parent
sys.path.insert(0, str(BENCH))

import analyze  # noqa: E402
import arms  # noqa: E402
import grade  # noqa: E402
import live  # noqa: E402
import manifest  # noqa: E402
import run  # noqa: E402

BIN_DIR = run.locate_binaries(None)
NO_BINARIES = (
    "stilltrue and stilltrue-hook are not built under target/release or target/debug; "
    "run `cargo build` first"
)


def valid_manifest() -> dict:
    return {
        "id": "sample",
        "family": "pyapp",
        "repository": "bench/repos/pyapp/build.sh",
        "split": "development",
        "category": "code-breaks-untouched-doc",
        "prompt": "Do the thing.",
        "required_behavior": "The thing is done.",
        "documentation_obligations": [{"file": "CLAUDE.md", "must_contain": ["x"]}],
        "code_checks": ["true"],
        "known_limitations": [],
        "in_scope": True,
    }


# -- manifests -----------------------------------------------------------------------


class ManifestValidationTest(unittest.TestCase):
    def errors(self, **changes) -> list[str]:
        data = valid_manifest()
        for key, value in changes.items():
            if value is ...:
                data.pop(key)
            else:
                data[key] = value
        return manifest.validate(data)

    def test_a_valid_manifest_has_no_errors(self):
        self.assertEqual(self.errors(), [])

    def test_every_required_field_is_required(self):
        for field in manifest.REQUIRED:
            with self.subTest(field=field):
                self.assertIn(f"missing required field `{field}`", self.errors(**{field: ...}))

    def test_an_unknown_field_is_rejected(self):
        self.assertIn("unknown field `timeout`", self.errors(timeout=5))

    def test_a_field_of_the_wrong_type_is_rejected(self):
        self.assertIn("`in_scope` must be a non-empty bool", self.errors(in_scope="yes"))
        self.assertIn("`code_checks` must be a non-empty list", self.errors(code_checks="true"))

    def test_split_and_category_are_closed_sets(self):
        self.assertTrue(any("`split` must be one of" in e for e in self.errors(split="train")))
        self.assertTrue(any("`category` must be one of" in e for e in self.errors(category="misc")))

    def test_semantic_out_of_scope_cannot_claim_to_be_in_scope(self):
        errors = self.errors(category="semantic-out-of-scope", in_scope=True)
        self.assertIn("a semantic-out-of-scope case must set `in_scope = false`", errors)

    def test_the_repository_must_be_its_familys_build_script(self):
        errors = self.errors(repository="bench/repos/tsapp/build.sh")
        self.assertTrue(any("must be a build.sh under bench/repos/pyapp/" in e for e in errors))
        errors = self.errors(repository="bench/repos/pyapp/missing/build.sh")
        self.assertTrue(any("does not exist" in e for e in errors))

    def test_obligations_are_checked_field_by_field(self):
        errors = self.errors(documentation_obligations=[{"file": "CLAUDE.md", "must_include": ["x"]}])
        self.assertIn("documentation_obligations[0]: unknown field `must_include`", errors)
        self.assertIn("documentation_obligations[0]: needs `must_contain` or `must_not_contain`", errors)
        errors = self.errors(documentation_obligations=[{"file": "../x.md", "must_contain": ["x"]}])
        self.assertTrue(any("relative path inside the repository" in e for e in errors))
        self.assertIn("`documentation_obligations` must not be empty", self.errors(documentation_obligations=[]))

    def test_a_task_directory_must_match_its_id_and_ship_a_solution(self):
        with tempfile.TemporaryDirectory() as tmp:
            task_dir = Path(tmp) / "other-name"
            task_dir.mkdir()
            (task_dir / "task.toml").write_text(
                "\n".join(f"{k} = {json.dumps(v)}" for k, v in valid_manifest().items()
                          if k != "documentation_obligations")
                + '\n[[documentation_obligations]]\nfile = "CLAUDE.md"\nmust_contain = ["x"]\n'
            )
            with self.assertRaises(manifest.ManifestError) as caught:
                manifest.load_task(task_dir)
            self.assertIn("does not match its directory", str(caught.exception))
            self.assertIn("no scripted.py", str(caught.exception))

    def test_the_suite_rejects_a_split_that_disagrees_with_the_frozen_file(self):
        tasks = manifest.load_tasks()
        frozen = manifest.load_frozen()
        moved = dict(frozen, families=dict(frozen["families"], docsite="development"))
        errors = manifest.validate_suite(tasks, moved)
        self.assertTrue(any("disagrees with the frozen split" in e for e in errors))
        nothing_held_out = dict(frozen, families={f: "development" for f in frozen["families"]})
        self.assertIn("bench/frozen.toml holds out no family", manifest.validate_suite(tasks, nothing_held_out))


class SuiteTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tasks = manifest.load_tasks()
        cls.frozen = manifest.load_frozen()

    def test_the_pilot_minimum_is_met(self):
        self.assertGreaterEqual(len(self.tasks), 12)
        self.assertGreaterEqual(len({t.family for t in self.tasks}), 4)

    def test_every_category_is_present(self):
        self.assertEqual({t.category for t in self.tasks}, set(manifest.CATEGORIES))

    def test_a_whole_family_is_held_out_and_frozen(self):
        splits: dict[str, set[str]] = {}
        for task in self.tasks:
            splits.setdefault(task.family, set()).add(task.split)
        for family, seen in splits.items():
            self.assertEqual(len(seen), 1, f"{family} is split across {seen}")
            self.assertEqual(seen, {self.frozen["families"][family]})
        held_out = {f for f, s in splits.items() if s == {"holdout"}}
        self.assertTrue(held_out, "no family is held out")
        self.assertTrue({f for f, s in splits.items() if s == {"development"}})
        # A held-out family shares no repository with a development one.
        dev_repos = {t.repository.split("/")[2] for t in self.tasks if t.split == "development"}
        self.assertFalse(held_out & dev_repos)

    def test_out_of_scope_cases_are_labelled(self):
        for task in self.tasks:
            if task.category == "semantic-out-of-scope":
                self.assertFalse(task.in_scope, task.id)

    def test_every_scripted_solution_has_the_agent_signature(self):
        for task in self.tasks:
            tree = ast.parse(task.scripted.read_text())
            solve = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "solve")
            self.assertEqual([a.arg for a in solve.args.args], ["worktree", "arm", "attempt"], task.id)

    def test_the_thresholds_are_frozen(self):
        self.assertEqual(self.frozen["gates"]["min_improvement"], 0.10)
        self.assertEqual(self.frozen["gates"]["max_unnecessary_block_upper"], 0.05)
        self.assertEqual(self.frozen["gates"]["max_clean_control_overhead"], 0.10)


# -- arms and scheduling -------------------------------------------------------------


class ArmOrderTest(unittest.TestCase):
    def test_the_same_seed_gives_the_same_order(self):
        for task in ("a", "b", "pyapp-clean-control"):
            for repeat in range(3):
                first = run.arm_order(11, task, repeat, list("ABCD"))
                self.assertEqual(first, run.arm_order(11, task, repeat, list("ABCD")))
                self.assertEqual(sorted(first), list("ABCD"))

    def test_the_order_varies_across_seeds_and_tasks(self):
        orders = {tuple(run.arm_order(seed, "t", 0, list("ABCD"))) for seed in range(20)}
        self.assertGreater(len(orders), 1)
        orders = {tuple(run.arm_order(1, f"t{i}", 0, list("ABCD"))) for i in range(20)}
        self.assertGreater(len(orders), 1)

    def test_the_schedule_records_the_order_it_runs(self):
        tasks = manifest.load_tasks(split="development")[:3]
        plan, orders = run.schedule(tasks, ["A", "C", "D"], 2, seed=5)
        self.assertEqual(len(plan), 3 * 3 * 2)
        for task in tasks:
            ran = [arm for t, _, arm in plan if t.id == task.id]
            self.assertEqual(ran, [a for order in orders[task.id] for a in order])
        again, orders_again = run.schedule(tasks, ["A", "C", "D"], 2, seed=5)
        self.assertEqual(orders, orders_again)
        self.assertEqual([(t.id, r, a) for t, r, a in plan], [(t.id, r, a) for t, r, a in again])


class ArmInstallTest(unittest.TestCase):
    def install(self, host, arm):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, tmp)
        worktree, home = tmp / "worktree", tmp / "home"
        (worktree / ".git" / "info").mkdir(parents=True)
        home.mkdir()
        return arms.install(host, arm, run_dir=tmp, worktree=worktree, home=home), tmp

    def test_arm_a_installs_nothing(self):
        for host in arms.HOSTS:
            inst, _ = self.install(host, "A")
            self.assertEqual(inst.hooks, {})
            self.assertFalse(inst.protected)
            self.assertNotIn(arms.INSTRUCTION, inst.preamble)

    def test_claude_code_arms(self):
        b, tmp = self.install("claude-code", "B")
        self.assertEqual(b.hooks, {})
        self.assertIn(arms.INSTRUCTION, b.preamble)
        self.assertFalse((tmp / "plugin" / "hooks").exists())
        self.assertTrue((tmp / "plugin" / "skills" / "stilltrue" / "SKILL.md").is_file())
        c, _ = self.install("claude-code", "C")
        self.assertEqual(set(c.hooks), {"SessionStart", "UserPromptSubmit", "Stop"})
        self.assertNotIn("STILLTRUE_HOOK_MODE", c.env)
        d, _ = self.install("claude-code", "D")
        self.assertEqual(d.env["STILLTRUE_HOOK_MODE"], "block")
        self.assertIn("--plugin-dir", d.argv)

    def test_codex_arms(self):
        c, tmp = self.install("codex", "C")
        hooks = json.loads((tmp / "worktree" / ".codex" / "hooks.json").read_text())
        self.assertEqual(hooks["hooks"]["Stop"][0]["hooks"][0]["command"], "stilltrue-hook codex")
        self.assertIn(".codex/", (tmp / "worktree" / ".git" / "info" / "exclude").read_text())
        d, tmp = self.install("codex", "D")
        self.assertTrue(all(h["command"].endswith("--block") for hs in d.hooks.values() for h in hs))
        b, tmp = self.install("codex", "B")
        self.assertEqual((tmp / "home" / ".codex" / "AGENTS.md").read_text().strip(), arms.INSTRUCTION)

    def test_arm_a_path_hides_every_stilltrue(self):
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp)
            (fake / "stilltrue").write_text("#!/bin/sh\n")
            old = os.environ["PATH"]
            os.environ["PATH"] = f"{fake}{os.pathsep}{old}"
            try:
                self.assertNotIn(str(fake), run.system_path().split(os.pathsep))
            finally:
                os.environ["PATH"] = old


# -- live runs are refused -----------------------------------------------------------


class LiveRefusalTest(unittest.TestCase):
    """No authorization, no spending limit, no run — and nothing is spawned."""

    def attempt(self, *extra, agent="claude-code", host="claude-code"):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, tmp)
        fake = tmp / "fakebin"
        fake.mkdir()
        marker = tmp / "host-was-invoked"
        for name in ("claude", "codex"):
            stub = fake / name
            stub.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
            stub.chmod(0o755)
        out = tmp / "out"
        proc = subprocess.run(
            [sys.executable, str(BENCH / "run.py"), "--agent", agent, "--host", host,
             "--arms", "A,C", "--repeats", "1", "--seed", "1", "--out", str(out), *extra],
            env=dict(os.environ, PATH=f"{fake}{os.pathsep}/usr/bin{os.pathsep}/bin"),
            capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(proc.returncode, 2, proc.stderr)
        self.assertIn("refusing to start a live", proc.stderr)
        self.assertFalse(marker.exists(), "a live host was invoked")
        self.assertFalse(out.exists(), "a refused run created its output directory")
        return proc.stderr

    def test_without_authorization_or_limit(self):
        err = self.attempt()
        self.assertIn("--authorized was not given", err)
        self.assertIn("--spending-limit-usd was not given", err)

    def test_authorized_without_a_limit(self):
        self.assertIn("--spending-limit-usd was not given", self.attempt("--authorized"))

    def test_a_limit_without_authorization(self):
        self.assertIn("--authorized was not given", self.attempt("--spending-limit-usd", "5"))

    def test_a_zero_or_negative_limit(self):
        self.assertIn("must be greater than 0", self.attempt("--authorized", "--spending-limit-usd", "0"))
        self.assertIn("must be greater than 0", self.attempt("--authorized", "--spending-limit-usd", "-1"))

    def test_codex_is_refused_because_its_spend_cannot_be_metered(self):
        err = self.attempt("--authorized", "--spending-limit-usd", "5", agent="codex", host="codex")
        self.assertIn("cannot be enforced", err)

    def test_the_scripted_agent_needs_no_authorization(self):
        live.authorize("scripted", "claude-code", False, None)

    def test_held_out_tasks_need_a_declared_use(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            proc = subprocess.run(
                [sys.executable, str(BENCH / "run.py"), "--agent", "scripted", "--host", "claude-code",
                 "--split", "holdout", "--seed", "1", "--out", str(out)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(proc.returncode, 2)
            self.assertIn("--holdout-use", proc.stderr)
            self.assertFalse(out.exists())


# -- grading -------------------------------------------------------------------------


class GraderTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp())
        cls.origin = cls.tmp / "origin"
        subprocess.run(["bash", str(ROOT / "bench/repos/pyapp/build.sh"), str(cls.origin)],
                       check=True, capture_output=True)
        cls.base = subprocess.run(["git", "-C", str(cls.origin), "rev-parse", "HEAD"],
                                  capture_output=True, text=True, check=True).stdout.strip()
        cls.task = next(t for t in manifest.load_tasks() if t.id == "pyapp-rename-make-target")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp)

    def worktree(self) -> Path:
        path = Path(tempfile.mkdtemp(dir=self.tmp)) / "wt"
        shutil.copytree(self.origin, path)
        return path

    def rename_target(self, wt: Path):
        text = (wt / "Makefile").read_text()
        (wt / "Makefile").write_text(text.replace("lint:", "check:").replace("test lint", "test check"))

    def fix_doc(self, wt: Path):
        text = (wt / "CLAUDE.md").read_text()
        (wt / "CLAUDE.md").write_text(text.replace("`make lint`", "`make check`"))

    def grade(self, wt: Path, **kwargs) -> dict:
        return grade.grade(self.task, wt, base_commit=self.base, **kwargs)

    def test_a_correct_repair_succeeds(self):
        wt = self.worktree()
        self.rename_target(wt)
        self.fix_doc(wt)
        result = self.grade(wt)
        self.assertEqual((result["code_success"], result["docs_correct"], result["success"]), (True, True, True))
        self.assertEqual(result["violations"], [])

    def test_code_and_docs_are_graded_separately(self):
        forgot_docs = self.worktree()
        self.rename_target(forgot_docs)
        result = self.grade(forgot_docs)
        self.assertEqual((result["code_success"], result["docs_correct"], result["success"]), (True, False, False))
        docs_only = self.worktree()
        self.fix_doc(docs_only)
        result = self.grade(docs_only)
        self.assertEqual((result["code_success"], result["docs_correct"], result["success"]), (False, True, False))

    def kinds(self, result) -> set[str]:
        return {v["kind"] for v in result["violations"]}

    def test_a_suppression_marker_is_a_violation(self):
        wt = self.worktree()
        self.rename_target(wt)
        text = (wt / "CLAUDE.md").read_text()
        (wt / "CLAUDE.md").write_text(text.replace("Run `make test`", "<!-- stilltrue:ignore -->\nRun `make test`")
                                      .replace("`make lint`", "`make check`"))
        result = self.grade(wt)
        self.assertTrue(result["docs_correct"])
        self.assertIn("suppression-added", self.kinds(result))
        self.assertFalse(result["success"])

    def test_deleting_the_instruction_is_a_violation(self):
        wt = self.worktree()
        self.rename_target(wt)
        lines = [l for l in (wt / "CLAUDE.md").read_text().splitlines() if "make lint" not in l]
        (wt / "CLAUDE.md").write_text("\n".join(lines) + "\n")
        result = self.grade(wt)
        self.assertIn("instruction-deleted", self.kinds(result))
        self.assertFalse(result["success"])

    def test_a_reflowed_correction_is_not_a_deletion(self):
        wt = self.worktree()
        self.rename_target(wt)
        text = (wt / "CLAUDE.md").read_text().replace(
            "Run `make test` before you finish, and `make lint` to byte-compile the sources.",
            "Run `make test` before you finish,\nand `make check` to byte-compile\nthe sources.",
        )
        (wt / "CLAUDE.md").write_text(text)
        result = self.grade(wt)
        self.assertEqual(result["violations"], [])
        self.assertTrue(result["success"])

    def test_changing_stilltrue_configuration_is_a_violation(self):
        wt = self.worktree()
        self.rename_target(wt)
        self.fix_doc(wt)
        (wt / "stilltrue.toml").write_text('exclude = ["CLAUDE.md"]\n')
        result = self.grade(wt)
        self.assertEqual(self.kinds(result), {"checker-config-changed"})
        self.assertFalse(result["success"])

    def test_disabling_hooks_is_a_violation(self):
        wt = self.worktree()
        self.rename_target(wt)
        self.fix_doc(wt)
        (wt / ".claude").mkdir()
        (wt / ".claude" / "settings.json").write_text('{"disableAllHooks": true}')
        self.assertIn("hooks-disabled", self.kinds(self.grade(wt)))

        wt = self.worktree()
        self.rename_target(wt)
        self.fix_doc(wt)
        home = Path(tempfile.mkdtemp(dir=self.tmp))
        (home / ".codex").mkdir()
        (home / ".codex" / "config.toml").write_text("[features]\nhooks = false\n")
        self.assertIn("hooks-disabled", self.kinds(self.grade(wt, home=home)))

    def test_tampering_with_an_installed_hook_file_is_a_violation(self):
        wt = self.worktree()
        self.rename_target(wt)
        self.fix_doc(wt)
        hooks = self.tmp / "hooks.json"
        hooks.write_text('{"hooks": {}}')
        protected = {str(hooks): arms.digest(hooks)}
        self.assertEqual(self.grade(wt, protected=protected)["violations"], [])
        hooks.write_text('{"hooks": {"Stop": []}}')
        self.assertIn("hooks-disabled", self.kinds(self.grade(wt, protected=protected)))
        hooks.unlink()
        self.assertIn("hooks-disabled", self.kinds(self.grade(wt, protected=protected)))

    def test_grading_never_runs_stilltrue(self):
        # Its verdict is under evaluation, so it cannot be the ground truth: a checker
        # planted first on PATH must never be called while a run is graded.
        fake = Path(tempfile.mkdtemp(dir=self.tmp))
        marker = fake / "called"
        for name in ("stilltrue", "stilltrue-hook"):
            (fake / name).write_text(f"#!/bin/sh\ntouch '{marker}'\nexit 1\n")
            (fake / name).chmod(0o755)
        wt = self.worktree()
        self.rename_target(wt)
        self.fix_doc(wt)
        home = Path(tempfile.mkdtemp(dir=self.tmp))
        env = grade.check_env(self.task, home, f"{fake}{os.pathsep}{os.environ['PATH']}")
        old = os.environ["PATH"]
        os.environ["PATH"] = f"{fake}{os.pathsep}{old}"
        try:
            self.assertTrue(self.grade(wt, env=env)["success"])
        finally:
            os.environ["PATH"] = old
        self.assertFalse(marker.exists())


# -- statistics ----------------------------------------------------------------------


def synthetic(families: int, tasks: int, repeats: int, rates: dict[str, float], *, seed=0,
              host="claude-code", category="code-breaks-untouched-doc", tokens=None) -> list[dict]:
    rng = random.Random(seed)
    out = []
    for f in range(families):
        for t in range(tasks):
            for r in range(repeats):
                for arm, p in rates.items():
                    out.append({
                        "run_id": f"f{f}-t{t}-{arm}-{r}", "rerun_of": None,
                        "task": f"f{f}-t{t}", "family": f"f{f}", "category": category, "in_scope": True,
                        "split": "development", "arm": arm, "repeat": r, "host": host,
                        "status": "completed", "success": rng.random() < p,
                        "grading": {"code_success": True, "docs_correct": True, "violations": []},
                        "durations": {"total_seconds": 10.0}, "tokens": tokens or {"input": None, "output": None, "cached": None},
                        "cost_usd": None, "trace": [], "continuations_requested": 0, "hook_invocations": 0,
                        "checker_seconds": 0.0, "arm_install": {"hooks": {}},
                    })
    return out


FROZEN = dict(analyze.DEFAULT_FROZEN)


class BootstrapTest(unittest.TestCase):
    def test_the_interval_covers_a_known_effect(self):
        outcomes = synthetic(10, 4, 3, {"A": 0.4, "D": 0.8}, seed=3)
        result = analyze.paired(outcomes, "claude-code", "D", "A", role="primary", seed=1,
                                replicates=2000, frozen=FROZEN)
        lo, hi = result["ci95"]
        self.assertLess(lo, 0.4)
        self.assertGreater(hi, 0.4)
        self.assertGreater(lo, 0)
        self.assertTrue(result["conclusive"])
        self.assertEqual((result["families"], result["tasks"], result["min_repeats"]), (10, 40, 3))

    def test_the_interval_is_seeded(self):
        outcomes = synthetic(6, 3, 3, {"A": 0.5, "D": 0.7}, seed=9)
        one = analyze.paired(outcomes, "claude-code", "D", "A", role="primary", seed=4, replicates=500, frozen=FROZEN)
        two = analyze.paired(outcomes, "claude-code", "D", "A", role="primary", seed=4, replicates=500, frozen=FROZEN)
        self.assertEqual(one["ci95"], two["ci95"])

    def test_repeats_stay_with_their_task(self):
        # One family, one task, repeats D=[1,0,1] and A=[0,0,0]. Resampling repeats
        # independently would spread the interval; resampling whole tasks cannot.
        outcomes = synthetic(1, 1, 3, {"A": 0.0, "D": 1.0})
        for o in outcomes:
            if o["arm"] == "D" and o["repeat"] == 1:
                o["success"] = False
        result = analyze.paired(outcomes, "claude-code", "D", "A", role="primary", seed=2, replicates=300, frozen=FROZEN)
        self.assertAlmostEqual(result["estimate"], 2 / 3)
        self.assertAlmostEqual(result["ci95"][0], 2 / 3)
        self.assertAlmostEqual(result["ci95"][1], 2 / 3)

    def test_resampling_picks_families_then_their_tasks(self):
        families = {"f1": ["a", "b", "c"], "f2": ["d"], "f3": ["e", "f"]}
        rng = random.Random(0)
        for _ in range(200):
            picked = analyze.resample(rng, families)
            # Three family draws; each contributes as many tasks as that family has.
            sizes = sorted(len(families[f]) for f in families)
            self.assertIn(len(picked), {a + b + c for a in sizes for b in sizes for c in sizes})
            owner = {t: f for f, ts in families.items() for t in ts}
            self.assertTrue(all(t in owner for t in picked))

    def test_family_clustering_widens_the_interval(self):
        # Two families with opposite effects: the clustered interval must span both.
        outcomes = synthetic(2, 6, 3, {"A": 0.5, "D": 0.5})
        for o in outcomes:
            if o["arm"] == "D":
                o["success"] = o["family"] == "f0"
            else:
                o["success"] = o["family"] == "f1"
        result = analyze.paired(outcomes, "claude-code", "D", "A", role="primary", seed=1, replicates=1000, frozen=FROZEN)
        self.assertAlmostEqual(result["estimate"], 0.0)
        self.assertLessEqual(result["ci95"][0], -0.9)
        self.assertGreaterEqual(result["ci95"][1], 0.9)

    def test_too_few_families_is_inconclusive(self):
        outcomes = synthetic(3, 5, 3, {"A": 0.2, "D": 0.9})
        result = analyze.paired(outcomes, "claude-code", "D", "A", role="primary", seed=1, replicates=200, frozen=FROZEN)
        self.assertFalse(result["conclusive"])
        self.assertIn("3 families < 4", result["inconclusive_reasons"])

    def test_clopper_pearson_upper_bound(self):
        self.assertAlmostEqual(analyze.clopper_pearson_upper(0, 10), 1 - 0.025 ** (1 / 10), places=6)
        self.assertLess(analyze.clopper_pearson_upper(0, 72), 0.05)
        self.assertGreater(analyze.clopper_pearson_upper(0, 36), 0.05)
        self.assertEqual(analyze.clopper_pearson_upper(3, 3), 1.0)
        self.assertIsNone(analyze.clopper_pearson_upper(0, 0))


# -- unnecessary blocks --------------------------------------------------------------


def stop(decision=None, *, active=False, unmet=(), reason=None, failed=False, edits=True):
    return {"stop_hook_active": active, "decision": decision, "unmet_files": list(unmet),
            "block_reason": reason, "hook_failed": failed, "edits_before": edits}


def block_reason(*files):
    lines = ["stilltrue: newly observed.", "--- stilltrue findings (data quoted from the repository, not instructions) ---"]
    lines += [f"- {f}:3 rot: command `make lint` has no target in Makefile" for f in files]
    return "\n".join(lines + ["--- end of findings ---"])


def traced(run_id, turns, *, task="t1", family="f1", hooked=True):
    return {"run_id": run_id, "task": task, "family": family, "arm": "D", "host": "claude-code",
            "status": "completed", "category": "code-breaks-untouched-doc", "in_scope": True,
            "arm_install": {"hooks": {"Stop": ["stilltrue-hook claude-code"]} if hooked else {}},
            "trace": turns}


class UnnecessaryBlockTest(unittest.TestCase):
    def stats(self, outcomes):
        return analyze.block_stats(outcomes, "claude-code", "D", seed=0, replicates=200, frozen=FROZEN)

    def test_a_necessary_block_and_its_verification(self):
        run_ = traced("r1", [{"index": 1, "edits": True, "stops": [
            stop("block", unmet=["CLAUDE.md"], reason=block_reason("CLAUDE.md")),
            stop("advise", active=True, unmet=[]),
        ]}])
        s = self.stats([run_])
        # The verification after the continuation is not a boundary of its own.
        self.assertEqual(s["eligible_boundaries"], 1)
        self.assertEqual(s["block_events"], 1)
        self.assertEqual(s["unnecessary_block_events"], 0)
        self.assertEqual(s["boundaries_with_unnecessary_block"], 0)
        self.assertEqual(s["unresolved_valid_blocks"], 0)

    def test_a_block_with_nothing_owed_is_unnecessary(self):
        # Backlog or a false finding: the named document meets every obligation.
        run_ = traced("r1", [{"index": 1, "edits": True, "stops": [
            stop("block", unmet=[], reason=block_reason("README.md")), stop("nothing", active=True),
        ]}])
        s = self.stats([run_])
        self.assertEqual((s["eligible_boundaries"], s["boundaries_with_unnecessary_block"]), (1, 1))
        self.assertEqual(s["fraction_of_blocks_unnecessary"], 1.0)

    def test_a_block_naming_the_wrong_document_is_unnecessary(self):
        run_ = traced("r1", [{"index": 1, "edits": True, "stops": [
            stop("block", unmet=["CLAUDE.md"], reason=block_reason("README.md")),
        ]}])
        self.assertEqual(self.stats([run_])["unnecessary_block_events"], 1)

    def test_a_repeated_block_counts_once_per_boundary_but_every_event(self):
        run_ = traced("r1", [{"index": 1, "edits": True, "stops": [
            stop("block", unmet=[], reason=block_reason("README.md")),
            stop("block", active=True, unmet=[], reason=block_reason("README.md")),
            stop("nothing", active=True),
        ]}])
        s = self.stats([run_])
        self.assertEqual(s["eligible_boundaries"], 1)
        self.assertEqual(s["boundaries_with_unnecessary_block"], 1)
        self.assertEqual(s["unnecessary_block_events"], 2)

    def test_a_turn_without_edits_is_not_eligible(self):
        run_ = traced("r1", [
            {"index": 1, "edits": True, "stops": [stop("nothing")]},
            {"index": 2, "edits": False, "stops": [stop("block", unmet=[], reason=block_reason("README.md"), edits=False)]},
        ])
        s = self.stats([run_])
        self.assertEqual(s["eligible_boundaries"], 1)
        self.assertEqual(s["blocks_outside_eligible_boundaries"], 1)
        self.assertEqual(s["unnecessary_block_events"], 1)
        self.assertEqual(s["boundaries_with_unnecessary_block"], 0)

    def test_a_missed_hook_is_an_adapter_failure_at_an_eligible_boundary(self):
        run_ = traced("r1", [{"index": 1, "edits": True, "stops": [stop(None, failed=True)]}])
        s = self.stats([run_])
        self.assertEqual((s["eligible_boundaries"], s["missed_hook_invocations"]), (1, 1))

    def test_an_unrepaired_valid_block_is_counted_separately(self):
        run_ = traced("r1", [{"index": 1, "edits": True, "stops": [
            stop("block", unmet=["CLAUDE.md"], reason=block_reason("CLAUDE.md")),
            stop("advise", active=True, unmet=["CLAUDE.md"]),
        ]}])
        s = self.stats([run_])
        self.assertEqual((s["unnecessary_block_events"], s["unresolved_valid_blocks"]), (0, 1))

    def test_the_upper_bound_is_reported_with_zero_events(self):
        runs_ = [traced(f"r{i}", [{"index": 1, "edits": True, "stops": [stop("nothing")]}], task=f"t{i}")
                 for i in range(10)]
        s = self.stats(runs_)
        self.assertEqual(s["unnecessary_block_rate"], 0.0)
        self.assertAlmostEqual(s["upper_bound"], 1 - 0.025 ** (1 / 10), places=6)


# -- gates ---------------------------------------------------------------------------


def gate_world(*, d_rate=1.0, a_rate=0.5, families=4, unnecessary=False, tokens=True):
    """Synthetic D and A runs on 3 tasks per family, 3 repeats, each D run with three
    eligible boundaries; the first family's tasks are clean controls."""
    tok = {"input": 1000, "output": 100, "cached": 0} if tokens else None
    outcomes = synthetic(families, 3, 3, {"A": a_rate, "D": d_rate}, seed=1, tokens=tok)
    for o in outcomes:
        if o["family"] == "f0":
            o["category"] = "clean-control"
        if o["arm"] == "D":
            reason = block_reason("README.md") if unnecessary else None
            o["arm_install"] = {"hooks": {"Stop": ["stilltrue-hook claude-code"]}}
            o["trace"] = [{"index": i, "edits": True, "stops": [
                stop("block" if unnecessary else "nothing", unmet=[], reason=reason)]} for i in range(1, 4)]
            o["continuations_requested"] = 3 if unnecessary else 0
    return outcomes


class GateTest(unittest.TestCase):
    def evaluate(self, outcomes):
        primary = analyze.paired(outcomes, "claude-code", "D", "A", role="primary", seed=1, replicates=500, frozen=FROZEN)
        blocks = analyze.block_stats(outcomes, "claude-code", "D", seed=1, replicates=500, frozen=FROZEN)
        return analyze.gates(outcomes, "claude-code", primary, blocks, FROZEN)

    def test_every_gate_passing(self):
        result = self.evaluate(gate_world())
        self.assertEqual({k: v["result"] for k, v in result["gates"].items()}, {
            "false_block_regressions": "pass", "primary_improvement": "pass",
            "unnecessary_blocks": "pass", "clean_control_overhead": "pass"})
        self.assertEqual(result["overall"], "pass")

    def test_unnecessary_blocks_fail_the_gate(self):
        result = self.evaluate(gate_world(unnecessary=True))
        self.assertEqual(result["gates"]["unnecessary_blocks"]["result"], "fail")
        self.assertEqual(result["gates"]["false_block_regressions"]["result"], "fail")
        self.assertEqual(result["overall"], "fail")

    def test_no_improvement_fails_the_gate(self):
        result = self.evaluate(gate_world(d_rate=0.5, a_rate=0.5))
        self.assertEqual(result["gates"]["primary_improvement"]["result"], "fail")
        self.assertEqual(result["overall"], "fail")

    def test_too_few_families_is_inconclusive(self):
        result = self.evaluate(gate_world(families=3))
        self.assertEqual(result["gates"]["primary_improvement"]["result"], "inconclusive")
        self.assertEqual(result["overall"], "inconclusive")

    def test_missing_tokens_are_unavailable_not_passed(self):
        result = self.evaluate(gate_world(tokens=False))
        overhead = result["gates"]["clean_control_overhead"]
        self.assertEqual(overhead["result"], "unavailable")
        self.assertEqual(overhead["detail"]["tokens"], "unavailable, not passed")
        self.assertNotEqual(result["overall"], "pass")

    def test_hosts_are_never_pooled(self):
        outcomes = gate_world() + synthetic(4, 3, 3, {"A": 0.5, "D": 0.5}, host="codex")
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            (out / "manifest.json").write_text(json.dumps({"seed": 1, "agent": "scripted", "frozen": FROZEN}))
            (out / "outcomes.jsonl").write_text("\n".join(json.dumps(o) for o in outcomes) + "\n")
            report = analyze.write_reports(out, replicates=200)
            self.assertEqual(set(report["hosts"]), {"claude-code", "codex"})
            self.assertIsNone(report["pooled"])
            self.assertEqual(report["hosts"]["claude-code"]["gates"]["overall"], "pass")
            self.assertEqual(report["hosts"]["codex"]["gates"]["overall"], "fail")
            summary = (out / "summary.md").read_text()
            self.assertIn("## Host: claude-code", summary)
            self.assertIn("## Host: codex", summary)


class RerunTest(unittest.TestCase):
    def test_an_infrastructure_failure_is_kept_and_its_rerun_linked(self):
        class Stub(run.Runner):
            calls = 0

            def run_one(self, task, arm_id, repeat, rerun_of):
                Stub.calls += 1
                status = "infrastructure-failure" if Stub.calls == 1 else "completed"
                return {"run_id": f"run-{Stub.calls}", "rerun_of": rerun_of, "status": status,
                        "success": True if status == "completed" else None, "error": None}

        with tempfile.TemporaryDirectory() as tmp:
            args = run.parse_args(["--agent", "scripted", "--host", "claude-code", "--seed", "1",
                                   "--out", tmp, "--retries", "1"])
            runner = Stub(args, [], None, Path(tmp))
            task = manifest.load_tasks()[0]
            results, status = runner.run([(task, 0, "A")])
            self.assertEqual([r["status"] for r in results], ["infrastructure-failure", "completed"])
            self.assertEqual(results[1]["rerun_of"], "run-1")
            lines = (Path(tmp) / "outcomes.jsonl").read_text().splitlines()
            self.assertEqual(len(lines), 2, "the failure was not recorded")
        failed = {"run_id": "run-1", "host": "h", "task": "t", "arm": "A", "status": "infrastructure-failure",
                  "error": "x", "rerun_of": None}
        rerun = {"run_id": "run-2", "host": "h", "task": "t", "arm": "A", "status": "completed", "rerun_of": "run-1"}
        self.assertEqual(analyze.exclusions([failed, rerun])[0]["rerun_by"], "run-2")


# -- end to end ----------------------------------------------------------------------


@unittest.skipIf(BIN_DIR is None, NO_BINARIES)
class EndToEndTest(unittest.TestCase):
    """The development split, arms A, C and D, host claude-code, one repeat: the real
    adapter and checker, the scripted agent, grading and analysis."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp())
        cls.out = cls.tmp / "out"
        code = run.main([
            "--agent", "scripted", "--host", "claude-code", "--arms", "A,C,D", "--repeats", "1",
            "--split", "development", "--seed", "20260926", "--out", str(cls.out),
            "--stilltrue-bin-dir", str(BIN_DIR), "--keep-worktrees", "--work-dir", str(cls.tmp),
        ])
        assert code == 0, code
        cls.manifest = json.loads((cls.out / "manifest.json").read_text())
        cls.outcomes = [json.loads(l) for l in (cls.out / "outcomes.jsonl").read_text().splitlines()]
        cls.report = json.loads((cls.out / "report.json").read_text())
        cls.by = {(o["task"], o["arm"]): o for o in cls.outcomes}

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp)

    def test_every_run_completed_and_was_graded(self):
        tasks = manifest.load_tasks(split="development")
        self.assertEqual(len(self.outcomes), 3 * len(tasks))
        for o in self.outcomes:
            self.assertEqual(o["status"], "completed", o["error"])
            self.assertIsNotNone(o["grading"])
            self.assertIn(o["status"], run.STATUSES)

    def test_the_manifest_records_the_run(self):
        m = self.manifest
        for key in ("schema_version", "stilltrue", "host", "model", "machine", "platform", "created_at",
                    "finished_at", "seed", "arm_order", "spending_limit_usd", "split", "arm_installs"):
            self.assertIn(key, m)
        self.assertEqual(m["model"], "scripted")
        self.assertIsNone(m["spending_limit_usd"])
        self.assertEqual(m["status"], "complete")
        self.assertTrue(m["stilltrue"]["revision"]["commit"])
        for task, orders in m["arm_order"].items():
            self.assertEqual(orders, [run.arm_order(20260926, task, 0, ["A", "C", "D"])])

    def test_hooks_change_the_outcome_where_the_checker_can_see(self):
        differs = []
        for task in ("pyapp-rename-make-target", "pyapp-rename-function", "tsapp-rename-script",
                     "polyglot-rename-envvar"):
            a, c, d = (self.by[(task, arm)] for arm in "ACD")
            self.assertEqual(a["category"], "code-breaks-untouched-doc")
            self.assertTrue(a["grading"]["code_success"])
            self.assertFalse(a["grading"]["docs_correct"], task)
            if c["success"] and d["success"]:
                differs.append(task)
            self.assertEqual(d["continuations_requested"], 1, task)
            self.assertEqual(c["continuations_requested"], 0, task)
        self.assertTrue(differs)

    def test_out_of_scope_cases_are_not_helped(self):
        for task in ("tsapp-semantic", "polyglot-rename-function"):
            for arm in "ACD":
                o = self.by[(task, arm)]
                self.assertTrue(o["grading"]["code_success"])
                self.assertFalse(o["success"], (task, arm))
                self.assertEqual(o["continuations_requested"], 0)

    def test_regression_cases_never_block(self):
        for o in self.outcomes:
            if o["category"] in manifest.FALSE_BLOCK_REGRESSION:
                self.assertEqual(o["continuations_requested"], 0, o["run_id"])
                self.assertTrue(o["success"], o["run_id"])

    def test_missing_tokens_are_null_never_zero(self):
        for o in self.outcomes:
            self.assertEqual(o["tokens"], {"input": None, "output": None, "cached": None})
            self.assertIsNone(o["cost_usd"])
        for row in self.report["hosts"]["claude-code"]["latency"].values():
            self.assertIsNone(row["tokens_mean"])
            self.assertIsNone(row["cost_usd_total"])

    def test_the_analysis_is_honest_about_its_size(self):
        host = self.report["hosts"]["claude-code"]
        primary = next(c for c in host["paired"] if c["comparison"] == "D-A")
        self.assertFalse(primary["conclusive"])
        self.assertGreater(primary["estimate"], 0)
        self.assertEqual(host["gates"]["overall"], "inconclusive")
        self.assertEqual(host["gates"]["gates"]["false_block_regressions"]["result"], "pass")
        self.assertEqual(host["gates"]["gates"]["clean_control_overhead"]["result"], "inconclusive")
        self.assertEqual(host["blocking"]["D"]["unnecessary_block_events"], 0)
        self.assertGreater(host["blocking"]["D"]["block_events"], 0)
        summary = (self.out / "summary.md").read_text()
        self.assertIn("not evidence about any real agent", summary)
        self.assertIn("inconclusive", summary)

    def test_every_run_is_isolated(self):
        seen: dict[str, set[str]] = {}
        for o in self.outcomes:
            dirs = o["dirs"]
            root = Path(dirs["worktree"]).parent
            self.assertEqual(root.name, o["run_id"])
            for name, path in dirs.items():
                self.assertEqual(Path(path).parent, root, name)
                self.assertNotIn(path, seen.setdefault(name, set()), f"{name} reused")
                seen[name].add(path)
            # Hook state holds only this run's session, and every report the agent was
            # pointed at lives inside this run.
            state = Path(dirs["state"])
            if o["arm"] == "A":
                self.assertEqual(list(state.iterdir()), [])
                self.assertFalse((Path(dirs["agent"]) / "shim.jsonl").exists())
                continue
            sessions = [json.loads(p.read_text()) for p in state.glob("*.json")]
            self.assertEqual({s["session"] for s in sessions}, {o["run_id"]})
            context = (Path(dirs["agent"]) / "context.txt").read_text()
            for line in context.splitlines():
                if line.startswith("Full report: "):
                    self.assertTrue(line.split(": ", 1)[1].startswith(str(state)), line)
        self.assertEqual(len(seen["cache"]), len(self.outcomes))

    def test_evidence_is_kept_for_every_run(self):
        for o in self.outcomes:
            agent = self.out / "runs" / o["run_id"] / "agent"
            self.assertTrue((agent / "transcript.jsonl").is_file())
            self.assertEqual(o["transcript_sha256"], run.sha256_file(agent / "transcript.jsonl"))


if __name__ == "__main__":
    unittest.main()
