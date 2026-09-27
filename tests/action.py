"""Run the composite Action's shell steps locally, offline, against a real archive.

The Action has three steps and only one of them was ever exercised. These run the
literal YAML blocks — extracted from action.yml so they cannot drift from what GitHub
would execute — under the flags GitHub uses for `shell: bash`.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import textwrap


ROOT = Path(__file__).resolve().parents[1]
ACTION = ROOT / "action.yml"
VERSION = re.search(
    r'^version = "([^"]+)"$', (ROOT / "Cargo.toml").read_text(), re.MULTILINE
).group(1)
# GitHub runs `shell: bash` as `bash --noprofile --norc -eo pipefail`.
BASH = ["/bin/bash", "--noprofile", "--norc", "-eo", "pipefail", "-c"]


def block(step_name):
    """The literal `run:` script of one step, read out of action.yml."""
    section = ACTION.read_text().split(f"    - name: {step_name}\n", 1)[1]
    body = section.split("      run: |\n", 1)[1].split("\n    - name:", 1)[0]
    return textwrap.dedent(body)


def run(script, env, cwd=None):
    return subprocess.run(BASH + [script], env=env, cwd=cwd, text=True, capture_output=True)


def check(condition, label, detail=""):
    if not condition:
        raise AssertionError(f"{label}\n{detail}")
    print(f"  ok  {label}")


SERVE = """#!/bin/bash
# Stand in for curl: the last argument is the URL and `-o PATH` says where to put it.
out=""
url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
case "$url" in
  *.sha256) [ -n "$TEST_SUM" ] && cp "$TEST_SUM" "$out" || exit 22 ;;
  *) cp "$TEST_ARCHIVE" "$out" ;;
esac
"""


def checksum_for(archive, root, value=None):
    """The `<hash>  <name>` file cargo-dist publishes beside an archive.

    Hashed in-process rather than by shelling out: the tool is `shasum` on macOS and
    `sha256sum` on Linux, and this test runs on both.
    """
    if value is None:
        value = hashlib.sha256(archive.read_bytes()).hexdigest()
    path = root / "archive.sha256"
    path.write_text(f"{value}  {archive.name}\n")
    return path


def install_env(root, archive, **overrides):
    mock_bin = root / "bin"
    mock_bin.mkdir(exist_ok=True)
    curl = mock_bin / "curl"
    curl.write_text(SERVE)
    curl.chmod(0o755)
    env = dict(
        os.environ,
        PATH=f"{mock_bin}:{os.environ['PATH']}",
        TEST_ARCHIVE=str(archive),
        TEST_SUM=str(checksum_for(archive, root)),
        RUNNER_TEMP=str(root),
        GITHUB_PATH=str(root / "github-path"),
        VERSION=VERSION,
        GITHUB_ACTION_REPOSITORY="uk0064/stilltrue",
    )
    env.update(overrides)
    return env, curl


def test_install(archive):
    print("Install stilltrue:")
    script = block("Install stilltrue")

    with tempfile.TemporaryDirectory(prefix="stilltrue-install-") as d:
        root = Path(d)
        env, curl = install_env(root, archive)
        result = run(script, env)
        check(result.returncode == 0, "a real archive installs", result.stderr)
        installed = Path((root / "github-path").read_text().strip()) / "stilltrue"
        check(installed.is_file(), "the binary lands on PATH", str(installed))
        version = subprocess.run([str(installed), "--version"], text=True, capture_output=True)
        check(
            version.stdout.strip() == f"stilltrue {VERSION}",
            "the installed binary reports its version",
            version.stdout,
        )

        # A failed download must not be reported as a successful installation.
        curl.write_text("#!/bin/bash\nexit 22\n")
        result = run(script, env)
        check(result.returncode == 2, "a failed download exits 2", str(result))
        check("could not download" in result.stderr, "and says so", result.stderr)

    with tempfile.TemporaryDirectory(prefix="stilltrue-tamper-") as d:
        # The binary goes on PATH, so a wrong checksum has to stop the install.
        root = Path(d)
        env, _ = install_env(root, archive)
        env["TEST_SUM"] = str(checksum_for(archive, root, value="0" * 64))
        result = run(block("Install stilltrue"), env)
        check(result.returncode == 2, "a checksum mismatch exits 2", str(result))
        check("checksum mismatch" in result.stderr, "and says so", result.stderr)
        check(
            not (root / "github-path").exists(),
            "and nothing reaches PATH",
            result.stderr,
        )

    with tempfile.TemporaryDirectory(prefix="stilltrue-nosum-") as d:
        root = Path(d)
        env, _ = install_env(root, archive)
        env["TEST_SUM"] = ""
        result = run(block("Install stilltrue"), env)
        check(result.returncode == 2, "a missing checksum exits 2", str(result))
        check("no checksum published" in result.stderr, "and says so", result.stderr)

    with tempfile.TemporaryDirectory(prefix="stilltrue-empty-") as d:
        # tar exits 0 on an empty stream. Without a post-condition the step goes green
        # having installed nothing, and the next step dies with `command not found`.
        root = Path(d)
        empty = root / "empty.tar.gz"
        with tarfile.open(empty, "w:gz"):
            pass
        env, _ = install_env(root, empty)
        result = run(block("Install stilltrue"), env)
        check(result.returncode == 2, "an archive with no binary exits 2", str(result))
        check("produced no binary" in result.stderr, "and says so", result.stderr)

    with tempfile.TemporaryDirectory(prefix="stilltrue-noversion-") as d:
        # `required: true` is not enforced by the runner.
        root = Path(d)
        env, _ = install_env(root, archive, VERSION="")
        result = run(block("Install stilltrue"), env)
        check(result.returncode == 2, "an empty version input exits 2", str(result))
        check("is required" in result.stderr, "and names the input", result.stderr)


def test_run(archive):
    print("Run stilltrue:")
    script = block("Run stilltrue")

    with tempfile.TemporaryDirectory(prefix="stilltrue-run-") as d:
        root = Path(d)
        # Use the binary out of the archive, not out of target/release: it is the one
        # that actually ships, and cargo-dist builds under a profile of its own, so
        # target/release may not exist at all in CI.
        unpacked = root / "unpacked"
        with tarfile.open(archive) as tar:
            tar.extractall(unpacked)
        binary = next(unpacked.rglob("stilltrue"))
        binary.chmod(0o755)
        repo = root / "repo"
        subprocess.run(
            ["bash", str(ROOT / "tests/fixtures/command-rot/build.sh"), str(repo)],
            check=True, capture_output=True,
        )
        # A shim ahead of the real binary counts invocations: the Action must scan once
        # however many outputs it was asked for (ADR-0021).
        shim = root / "shim"
        shim.mkdir()
        calls = root / "calls"
        (shim / "stilltrue").write_text(
            f'#!/bin/bash\necho "$*" >> "{calls}"\nexec "{binary}" "$@"\n'
        )
        (shim / "stilltrue").chmod(0o755)
        summary = root / "step-summary.md"
        outputs = root / "github-output"
        env = dict(
            os.environ,
            PATH=f"{shim}:{binary.parent}:{os.environ['PATH']}",
            RUNNER_TEMP=str(root),
            GITHUB_STEP_SUMMARY=str(summary),
            GITHUB_OUTPUT=str(outputs),
            FAIL_ON="rot", STRICT="false", ARGS="", SARIF_FILE="",
        )

        def invocations():
            count = len(calls.read_text().splitlines()) if calls.exists() else 0
            calls.unlink(missing_ok=True)
            return count

        result = run(script, env, cwd=repo)
        check(invocations() == 1, "one scan without SARIF")
        check(result.returncode == 1, "a rot finding fails the job", str(result))
        check(
            result.stdout.startswith("::error file=CLAUDE.md,line=1,col=6,"),
            "and is annotated at the right place",
            result.stdout,
        )
        check("python" not in script, "the step needs no runtime", script)

        result = run(script, dict(env, FAIL_ON="none"), cwd=repo)
        check(result.returncode == 0, "fail-on none still annotates but passes", str(result))
        check(result.stdout.startswith("::error "), "annotation still emitted", result.stdout)

        result = run(script, dict(env, FAIL_ON="nonsense"), cwd=repo)
        check(result.returncode == 2, "an unknown fail-on value exits 2", str(result))

        invocations()
        summary.unlink(missing_ok=True)
        outputs.unlink(missing_ok=True)
        sarif = root / "out dir" / "out.sarif"
        sarif.parent.mkdir()
        result = run(script, dict(env, SARIF_FILE=str(sarif)), cwd=repo)
        check(invocations() == 1, "one scan with SARIF too, not two")
        check(result.returncode == 1, "sarif-file does not change the exit code", str(result))
        check(sarif.is_file() and sarif.stat().st_size > 0, "and writes SARIF", str(sarif))
        results = json.loads(sarif.read_text())["runs"][0]["results"]
        annotations = [l for l in result.stdout.splitlines() if l.startswith("::error ")]
        check(
            len(results) == len(annotations) == 1,
            "SARIF and annotations carry the same findings",
            f"{len(results)} results, {len(annotations)} annotations",
        )
        report = json.loads((root / "stilltrue-report.json").read_text())
        check(report["version"] == 1, "the run report is written", str(report)[:200])
        check(report["status"] == "complete", "and says the scan was complete")
        check(
            len(report["findings"]) == len(annotations),
            "with the same findings as the annotations",
        )
        check(
            "stilltrue: summary (complete)" in summary.read_text(),
            "the summary reaches the job summary",
            summary.read_text() if summary.exists() else "no step summary",
        )
        check(
            "stilltrue: summary (" in result.stderr,
            "and the step log",
            result.stderr,
        )
        check(
            f"report-file={root / 'stilltrue-report.json'}" in outputs.read_text(),
            "the report path is a step output",
            outputs.read_text() if outputs.exists() else "no outputs",
        )

        # A shallow clone must be visible in the checks UI, not just on stderr.
        shallow = root / "shallow"
        subprocess.run(
            ["git", "clone", "-q", "--depth", "1", f"file://{repo}", str(shallow)],
            check=True, capture_output=True,
        )
        result = run(script, dict(env, FAIL_ON="rot"), cwd=shallow)
        check("::warning::" in result.stdout, "a degraded run annotates too", result.stdout)
        report = json.loads((root / "stilltrue-report.json").read_text())
        check(
            report["status"] == "incomplete",
            "and its report says it was incomplete",
            report["status"],
        )


if __name__ == "__main__":
    # The archive is the thing under test: the Action installs from a release tarball,
    # so there is nothing to check without one. Say which one is missing rather than
    # letting an IndexError stand in for the instruction.
    if len(sys.argv) != 2:
        sys.exit(
            f"usage: {sys.argv[0]} <archive.tar.gz>\n"
            "  build one with `dist build`; it lands in target/distrib/"
        )
    try:
        archive = Path(sys.argv[1]).resolve(strict=True)
    except OSError as err:
        sys.exit(f"{sys.argv[1]}: {err.strerror}")
    test_install(archive)
    test_run(archive)
    print("Action: install and run steps pass")
