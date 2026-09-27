"""Task manifests for the agent-task benchmark.

Each case lives in ``bench/tasks/<id>/task.toml`` beside a deterministic scripted
solution, ``scripted.py``. This module validates one manifest, the whole suite, and the
frozen split in ``bench/frozen.toml``. Unknown and missing fields are errors: a typo in
a manifest must never silently drop an obligation from grading.

    python3 bench/manifest.py            # validate every task, print a summary
"""

from __future__ import annotations

import ast
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

BENCH = Path(__file__).resolve().parent
ROOT = BENCH.parent

SPLITS = ("development", "holdout")
CATEGORIES = (
    "stale-at-entry",
    "code-breaks-untouched-doc",
    "clean-control",
    "near-miss",
    "preexisting-rot",
    "unsupported-analysis",
    "semantic-out-of-scope",
)
# Cases whose correct outcome includes "the adapter never blocks". A block in any of
# them is a false block, and the promotion gate requires all of them to pass.
FALSE_BLOCK_REGRESSION = ("clean-control", "near-miss", "preexisting-rot")

REQUIRED = {
    "id": str,
    "family": str,
    "repository": str,
    "split": str,
    "category": str,
    "prompt": str,
    "required_behavior": str,
    "documentation_obligations": list,
    "code_checks": list,
    "known_limitations": list,
    "in_scope": bool,
}
OPTIONAL = {"doc_checks": list}
OBLIGATION_KEYS = {"file", "must_contain", "must_not_contain"}
ID = re.compile(r"^[a-z0-9][a-z0-9-]*$")


class ManifestError(Exception):
    """A manifest, or the suite, is invalid. The message lists every problem."""


@dataclass(frozen=True)
class Obligation:
    file: str
    must_contain: tuple[str, ...] = ()
    must_not_contain: tuple[str, ...] = ()


@dataclass(frozen=True)
class Task:
    id: str
    family: str
    repository: str
    split: str
    category: str
    prompt: str
    required_behavior: str
    documentation_obligations: tuple[Obligation, ...]
    code_checks: tuple[str, ...]
    known_limitations: tuple[str, ...]
    in_scope: bool
    doc_checks: tuple[str, ...] = ()
    dir: Path = field(default=Path("."), compare=False)

    @property
    def build_script(self) -> Path:
        return ROOT / self.repository

    @property
    def scripted(self) -> Path:
        return self.dir / "scripted.py"

    @property
    def false_block_regression(self) -> bool:
        return self.category in FALSE_BLOCK_REGRESSION

    def summary(self) -> dict:
        """The fields every outcome record carries, so analysis needs no manifests."""
        return {
            "task": self.id,
            "family": self.family,
            "split": self.split,
            "category": self.category,
            "in_scope": self.in_scope,
        }


def _relative(path: str) -> bool:
    p = Path(path)
    return bool(path) and not p.is_absolute() and ".." not in p.parts


def _strings(value, name: str, errors: list[str], *, nonempty: bool = False) -> None:
    if not isinstance(value, list) or not all(isinstance(v, str) and v for v in value):
        errors.append(f"`{name}` must be a list of non-empty strings")
    elif nonempty and not value:
        errors.append(f"`{name}` must not be empty")


def validate(data: dict, *, task_dir: Path | None = None, root: Path = ROOT) -> list[str]:
    """Every problem with one manifest, as sentences. Empty means valid."""
    errors: list[str] = []
    for key in sorted(set(data) - set(REQUIRED) - set(OPTIONAL)):
        errors.append(f"unknown field `{key}`")
    for key, kind in REQUIRED.items():
        if key not in data:
            errors.append(f"missing required field `{key}`")
        elif not isinstance(data[key], kind) or (kind is str and not data[key].strip()):
            errors.append(f"`{key}` must be a non-empty {kind.__name__}")
    for key, kind in OPTIONAL.items():
        if key in data and not isinstance(data[key], kind):
            errors.append(f"`{key}` must be a {kind.__name__}")
    if errors:
        return errors

    if not ID.match(data["id"]):
        errors.append(f"`id` {data['id']!r} must be lowercase letters, digits and hyphens")
    if task_dir is not None and task_dir.name != data["id"]:
        errors.append(f"`id` {data['id']!r} does not match its directory {task_dir.name!r}")
    if not ID.match(data["family"]):
        errors.append(f"`family` {data['family']!r} must be lowercase letters, digits and hyphens")
    if data["split"] not in SPLITS:
        errors.append(f"`split` must be one of {', '.join(SPLITS)}, not {data['split']!r}")
    if data["category"] not in CATEGORIES:
        errors.append(f"`category` must be one of {', '.join(CATEGORIES)}, not {data['category']!r}")
    if data["category"] == "semantic-out-of-scope" and data["in_scope"]:
        errors.append("a semantic-out-of-scope case must set `in_scope = false`")

    repo = data["repository"]
    prefix = f"bench/repos/{data['family']}/"
    if not _relative(repo) or not repo.startswith(prefix) or not repo.endswith("/build.sh"):
        errors.append(
            f"`repository` must be a build.sh under {prefix} (its family's repository), not {repo!r}"
        )
    elif not (root / repo).is_file():
        errors.append(f"`repository` {repo!r} does not exist")

    obligations = data["documentation_obligations"]
    for i, ob in enumerate(obligations):
        where = f"documentation_obligations[{i}]"
        if not isinstance(ob, dict):
            errors.append(f"{where} must be a table")
            continue
        for key in sorted(set(ob) - OBLIGATION_KEYS):
            errors.append(f"{where}: unknown field `{key}`")
        if not isinstance(ob.get("file"), str) or not _relative(ob.get("file", "")):
            errors.append(f"{where}: `file` must be a relative path inside the repository")
        for key in ("must_contain", "must_not_contain"):
            if key in ob:
                _strings(ob[key], f"{where}.{key}", errors)
        if not ob.get("must_contain") and not ob.get("must_not_contain"):
            errors.append(f"{where}: needs `must_contain` or `must_not_contain`")
    if not obligations:
        errors.append("`documentation_obligations` must not be empty")
    _strings(data["code_checks"], "code_checks", errors, nonempty=True)
    _strings(data.get("doc_checks", []), "doc_checks", errors)
    _strings(data["known_limitations"], "known_limitations", errors)

    if task_dir is not None:
        scripted = task_dir / "scripted.py"
        if not scripted.is_file():
            errors.append("no scripted.py beside task.toml")
        else:
            tree = ast.parse(scripted.read_text(encoding="utf-8"))
            names = {n.name for n in tree.body if isinstance(n, ast.FunctionDef)}
            if "solve" not in names:
                errors.append("scripted.py does not define solve(worktree, arm, attempt)")
    return errors


def parse(data: dict, task_dir: Path) -> Task:
    return Task(
        id=data["id"],
        family=data["family"],
        repository=data["repository"],
        split=data["split"],
        category=data["category"],
        prompt=data["prompt"].strip(),
        required_behavior=data["required_behavior"].strip(),
        documentation_obligations=tuple(
            Obligation(
                file=ob["file"],
                must_contain=tuple(ob.get("must_contain", [])),
                must_not_contain=tuple(ob.get("must_not_contain", [])),
            )
            for ob in data["documentation_obligations"]
        ),
        code_checks=tuple(data["code_checks"]),
        doc_checks=tuple(data.get("doc_checks", [])),
        known_limitations=tuple(data["known_limitations"]),
        in_scope=data["in_scope"],
        dir=task_dir,
    )


def load_task(task_dir: Path, root: Path = ROOT) -> Task:
    path = task_dir / "task.toml"
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ManifestError(f"{path}: {error}") from error
    errors = validate(data, task_dir=task_dir, root=root)
    if errors:
        raise ManifestError(f"{path}:\n  " + "\n  ".join(errors))
    return parse(data, task_dir)


def load_frozen(root: Path = ROOT) -> dict:
    return tomllib.loads((root / "bench" / "frozen.toml").read_text(encoding="utf-8"))


def validate_suite(tasks: list[Task], frozen: dict) -> list[str]:
    """Problems that only show across tasks: duplicate ids, a family split between
    development and holdout, a split that disagrees with the frozen file."""
    errors: list[str] = []
    seen: set[str] = set()
    families = frozen.get("families", {})
    for task in tasks:
        if task.id in seen:
            errors.append(f"duplicate task id {task.id!r}")
        seen.add(task.id)
        if task.family not in families:
            errors.append(f"{task.id}: family {task.family!r} is not in bench/frozen.toml")
        elif families[task.family] != task.split:
            errors.append(
                f"{task.id}: split {task.split!r} disagrees with the frozen split "
                f"{families[task.family]!r} for family {task.family!r}"
            )
    for family, split in families.items():
        if split not in SPLITS:
            errors.append(f"bench/frozen.toml: family {family!r} has unknown split {split!r}")
    if "holdout" not in families.values():
        errors.append("bench/frozen.toml holds out no family")
    return errors


def load_tasks(
    root: Path = ROOT, *, split: str | None = None, ids: list[str] | None = None
) -> list[Task]:
    """Every task, validated individually and as a suite, then filtered."""
    tasks_dir = root / "bench" / "tasks"
    tasks: list[Task] = []
    problems: list[str] = []
    for task_dir in sorted(p for p in tasks_dir.iterdir() if p.is_dir()):
        try:
            tasks.append(load_task(task_dir, root))
        except ManifestError as error:
            problems.append(str(error))
    if not problems:
        problems = validate_suite(tasks, load_frozen(root))
    if problems:
        raise ManifestError("\n".join(problems))
    if split and split != "all":
        tasks = [t for t in tasks if t.split == split]
    if ids:
        unknown = sorted(set(ids) - {t.id for t in tasks})
        if unknown:
            raise ManifestError(f"no such task in this split: {', '.join(unknown)}")
        tasks = [t for t in tasks if t.id in ids]
    return tasks


def main() -> int:
    try:
        tasks = load_tasks()
    except ManifestError as error:
        print(error, file=sys.stderr)
        return 1
    by_split: dict[str, set[str]] = {}
    for task in tasks:
        by_split.setdefault(task.split, set()).add(task.family)
    categories = sorted({t.category for t in tasks})
    print(f"{len(tasks)} tasks valid")
    for split, families in sorted(by_split.items()):
        print(f"  {split}: {', '.join(sorted(families))}")
    print(f"  categories: {', '.join(categories)}")
    missing = sorted(set(CATEGORIES) - set(categories))
    if missing:
        print(f"  categories with no case: {', '.join(missing)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
