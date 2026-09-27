"""The four experimental arms, and what each installs for each host.

A — the normal harness, without stilltrue: no hooks, no instruction, and no checker on
    the agent's PATH.
B — stilltrue available through an instruction and (on Claude Code) the repair skill,
    with no automatic hooks: the agent runs the checker only if it follows the
    instruction.
C — automatic advisory hooks at session entry and completion.
D — the same hooks with bounded blocking (at most one continuation per user turn).

Every arm keeps the task text, tests, permissions, tools and budget identical; only
what is listed here differs. Installation writes into the run's own directories — a
per-run copy of the plugin, a per-run CODEX_HOME — so an arm cannot see another arm's
configuration, and the grader can check nothing installed was tampered with.

    python3 bench/arms.py            # print what each arm installs, per host
"""

from __future__ import annotations

import hashlib
import json
import re
import shutil
import sys
from dataclasses import dataclass, field
from pathlib import Path

BENCH = Path(__file__).resolve().parent
ROOT = BENCH.parent
HOSTS = ("claude-code", "codex")
ARM_IDS = ("A", "B", "C", "D")

# Arm B's standing instruction. It is given to the session, never written into the
# repository, so the documents under test are identical across arms.
INSTRUCTION = (
    "This repository's documents can be checked with stilltrue, which is on your PATH. "
    "Before you finish, run `stilltrue` in the repository root and repair any document "
    "it reports so that it describes the repository again. Do not delete instructions, "
    "add suppression markers or change stilltrue's configuration to make it pass."
)


@dataclass(frozen=True)
class Arm:
    id: str
    name: str
    checker_on_path: bool
    instruction: bool
    hooks: bool
    block: bool


ARMS = {
    "A": Arm("A", "normal harness without stilltrue", False, False, False, False),
    "B": Arm("B", "stilltrue via instruction/skill, no hooks", True, True, False, False),
    "C": Arm("C", "automatic advisory hooks", True, False, True, False),
    "D": Arm("D", "automatic hooks with bounded blocking", True, False, True, True),
}


@dataclass
class Installation:
    """What one arm put in place for one run."""

    arm: str
    host: str
    # Hook commands by event, exactly as the host would run them.
    hooks: dict[str, list[dict]] = field(default_factory=dict)
    # Extra environment for the host process (and so for its hooks).
    env: dict[str, str] = field(default_factory=dict)
    # Extra arguments for a live host invocation.
    argv: list[str] = field(default_factory=list)
    # What the session is told before the first prompt: standing instructions and the
    # skills it can load. For the scripted host this becomes the start of its context.
    preamble: str = ""
    # Files installed for this run, with their digests: the grader fails a run that
    # changed or removed one (disabling the hooks is a forbidden repair).
    protected: dict[str, str] = field(default_factory=dict)
    # Paths the harness itself wrote into the worktree; excluded from git status.
    worktree_files: list[str] = field(default_factory=list)

    def describe(self) -> dict:
        return {
            "arm": self.arm,
            "host": self.host,
            "hooks": {event: [h["command"] for h in hooks] for event, hooks in self.hooks.items()},
            "env": self.env,
            "argv": self.argv,
            "instruction": bool(self.preamble and INSTRUCTION in self.preamble),
            "protected": sorted(self.protected),
        }


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _protect(installation: Installation, root: Path) -> None:
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        installation.protected[str(path)] = digest(path)


def _skill_listing(skill: Path) -> str:
    """How a host lists an available skill: its name and description."""
    text = skill.read_text(encoding="utf-8")
    name = re.search(r"^name:\s*(.+)$", text, re.M)
    description = re.search(r"^description:\s*(.+)$", text, re.M)
    return (
        f"Available skill `{name.group(1).strip() if name else 'stilltrue'}`: "
        f"{description.group(1).strip() if description else ''}"
    )


def _hooks_from(config: Path) -> dict[str, list[dict]]:
    table = json.loads(config.read_text(encoding="utf-8"))["hooks"]
    return {
        event: [dict(h) for group in groups for h in group["hooks"] if h.get("type") == "command"]
        for event, groups in table.items()
    }


def install(
    host: str,
    arm_id: str,
    *,
    run_dir: Path,
    worktree: Path,
    home: Path,
    root: Path = ROOT,
) -> Installation:
    """Put arm `arm_id` in place for one run on `host`, inside the run's directories."""
    if host not in HOSTS:
        raise ValueError(f"unknown host {host!r}")
    arm = ARMS[arm_id]
    out = Installation(arm=arm_id, host=host)
    if host == "claude-code":
        _install_claude_code(arm, out, run_dir, root)
    else:
        _install_codex(arm, out, run_dir, worktree, home, root)
    return out


def _install_claude_code(arm: Arm, out: Installation, run_dir: Path, root: Path) -> None:
    source = root / "integrations" / "claude-code"
    skill = source / "skills" / "stilltrue" / "SKILL.md"
    if not (arm.hooks or arm.instruction):
        return
    plugin = run_dir / "plugin"
    if arm.hooks:
        # The whole plugin: hooks and skill, exactly as `--plugin-dir` would load it.
        shutil.copytree(source, plugin)
        out.hooks = _hooks_from(plugin / "hooks" / "hooks.json")
    else:
        # Arm B: the same plugin with its hooks removed — the skill only.
        shutil.copytree(source, plugin, ignore=shutil.ignore_patterns("hooks"))
    out.argv += ["--plugin-dir", str(plugin)]
    out.preamble = _skill_listing(skill) + "\n"
    if arm.instruction:
        out.argv += ["--append-system-prompt", INSTRUCTION]
        out.preamble += INSTRUCTION + "\n"
    if arm.block:
        out.env["STILLTRUE_HOOK_MODE"] = "block"
    _protect(out, plugin)


def _install_codex(
    arm: Arm, out: Installation, run_dir: Path, worktree: Path, home: Path, root: Path
) -> None:
    codex_home = home / ".codex"
    codex_home.mkdir(parents=True, exist_ok=True)
    out.env["CODEX_HOME"] = str(codex_home)
    if arm.instruction:
        # Codex reads global guidance from AGENTS.md in CODEX_HOME. Codex has no skill.
        (codex_home / "AGENTS.md").write_text(INSTRUCTION + "\n", encoding="utf-8")
        out.preamble = INSTRUCTION + "\n"
        _protect(out, codex_home)
    if arm.hooks:
        table = json.loads((root / "integrations" / "codex" / "hooks.json").read_text())
        if arm.block:
            for groups in table["hooks"].values():
                for group in groups:
                    for hook in group["hooks"]:
                        hook["command"] += " --block"
        target = worktree / ".codex" / "hooks.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(table, indent=2) + "\n", encoding="utf-8")
        exclude = worktree / ".git" / "info" / "exclude"
        exclude.parent.mkdir(parents=True, exist_ok=True)
        with exclude.open("a", encoding="utf-8") as handle:
            handle.write(".codex/\n")
        out.worktree_files.append(".codex/hooks.json")
        out.hooks = _hooks_from(target)
        out.argv += [
            "--dangerously-bypass-hook-trust",
            "-c",
            f'projects."{worktree}".trust_level="trusted"',
        ]
        out.protected[str(target)] = digest(target)


def describe(host: str, root: Path = ROOT) -> list[dict]:
    """What each arm installs on `host`, computed by installing into a scratch dir."""
    import tempfile

    rows = []
    with tempfile.TemporaryDirectory(prefix="stilltrue-bench-arms-") as tmp:
        for arm_id in ARM_IDS:
            base = Path(tmp) / arm_id
            worktree, home = base / "worktree", base / "home"
            (worktree / ".git" / "info").mkdir(parents=True)
            home.mkdir()
            inst = install(host, arm_id, run_dir=base, worktree=worktree, home=home, root=root)
            row = inst.describe()
            row["name"] = ARMS[arm_id].name
            row["checker_on_path"] = ARMS[arm_id].checker_on_path
            # Scratch paths mean nothing outside this call.
            row["argv"] = [a.replace(str(base), "<run>") for a in row["argv"]]
            row["env"] = {k: v.replace(str(base), "<run>") for k, v in row["env"].items()}
            row["protected"] = [p.replace(str(base), "<run>") for p in row["protected"]]
            rows.append(row)
    return rows


def main() -> int:
    for host in HOSTS:
        print(f"{host}:")
        for row in describe(host):
            print(f"  {row['arm']} {row['name']}")
            print(f"    checker on PATH: {row['checker_on_path']}; instruction: {row['instruction']}")
            for event, commands in row["hooks"].items():
                print(f"    hook {event}: {'; '.join(commands)}")
            if row["env"]:
                print(f"    env: {row['env']}")
            if row["argv"]:
                print(f"    host arguments: {row['argv']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
