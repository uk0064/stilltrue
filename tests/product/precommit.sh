#!/usr/bin/env bash
# The optional pre-commit integration, through pre-commit itself: a code-only change
# that breaks an untouched document fails the hook, and the hook sees the staged
# snapshot while a direct run sees the working tree.
#
#   tests/product/precommit.sh            # the system hook, against target/release
#   tests/product/precommit.sh --rust     # also build the hook with language: rust
#
# pre-commit is used from PATH, or through `uvx`. Without either this prints UNTESTED
# and exits 3, which is a result, not a pass.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ST="$ROOT/target/release/stilltrue"
[ -x "$ST" ] || { echo "no binary at $ST — run: cargo build --release"; exit 2; }
if command -v pre-commit >/dev/null 2>&1; then
  PRECOMMIT=(pre-commit)
elif command -v uvx >/dev/null 2>&1; then
  PRECOMMIT=(uvx pre-commit)
else
  echo "UNTESTED: neither pre-commit nor uvx is installed, so pre-commit could not be run"
  exit 3
fi
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home" XDG_CACHE_HOME="$WORK/home/.cache" PRE_COMMIT_HOME="$WORK/pre-commit"
mkdir -p "$HOME"
export PATH="$(dirname "$ST"):$PATH"
export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.com
export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.com
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); printf '  ok  %s\n' "$1"; }
bad() { FAIL=$((FAIL+1)); printf '  FAIL %s\n       %s\n' "$1" "$2"; }
is()  { [ "$2" = "$3" ] && ok "$1" || bad "$1" "expected $2, got $3"; }

repo="$WORK/consumer"
git init -q -b main "$repo" && cd "$repo"
printf 'demo:\n\techo demo\n' > Makefile
printf 'Run `make demo` to see it.\n' > CLAUDE.md
git add -A && git commit -q -m "add demo"

# try-repo reads the hook manifest from git, so an untracked one is invisible to it.
git -C "$ROOT" ls-files --error-unmatch .pre-commit-hooks.yaml >/dev/null 2>&1 \
  || { echo "UNTESTED: .pre-commit-hooks.yaml is not tracked, so pre-commit cannot see it"; exit 3; }

# The hook's verdict as pre-commit prints it: Passed or Failed, and never a pre-commit
# error. Both an error and a failing hook exit 1, so an exit code alone would let a
# broken harness pass every "must fail" assertion below.
run_hook() {
  "${PRECOMMIT[@]}" try-repo "$ROOT" "$1" >"$WORK/out" 2>&1
  local code=$?
  if grep -qE '^(stilltrue|stilltrue \(installed binary\))\.+(Passed|Failed)' "$WORK/out"; then
    grep -oE '(Passed|Failed)$' "$WORK/out" | head -1
  else
    echo "ERROR($code)"
  fi
}

echo "pre-commit: stilltrue-system"
code=$(run_hook stilltrue-system)
is "a clean repository passes" Passed "$code"

# A code-only change: the Makefile loses its target, CLAUDE.md is untouched and unstaged.
printf 'seed:\n\techo seed\n' > Makefile
git add Makefile
code=$(run_hook stilltrue-system)
is "a code-only change that breaks an untouched document fails the hook" Failed "$code"
grep -q 'make demo' "$WORK/out" && ok "and names the claim" || bad "and names the claim" "$(tail -5 "$WORK/out")"

# The fix, made in the working tree but not staged.
printf 'Run `make seed` to see it.\n' > CLAUDE.md
"$ST" >/dev/null 2>&1
is "a direct run reads the working tree, where the fix is" 0 "$?"
code=$(run_hook stilltrue-system)
is "the hook reads the staged snapshot, where it is not" Failed "$code"
grep -q 'make seed' CLAUDE.md && ok "and pre-commit restored the unstaged fix afterwards" \
  || bad "unstaged change restored" "CLAUDE.md lost the fix"

git add CLAUDE.md
code=$(run_hook stilltrue-system)
is "staged, the fix passes the hook" Passed "$code"

# A real commit through an installed hook, not just try-repo.
cat > .pre-commit-config.yaml <<CFG
repos:
  - repo: local
    hooks:
      - id: stilltrue
        name: stilltrue
        entry: stilltrue
        language: system
        pass_filenames: false
        always_run: true
CFG
git add .pre-commit-config.yaml
"${PRECOMMIT[@]}" install >/dev/null 2>&1
git commit -q -m "rename demo to seed" >/dev/null 2>&1
is "an installed hook lets a consistent commit through" 0 "$?"
printf 'lint:\n\techo lint\n' > Makefile
git add Makefile
git commit -q -m "drop seed" >/dev/null 2>&1
is "and stops a commit that breaks a document" 1 "$?"
git reset -q --hard HEAD

if [ "${1:-}" = "--rust" ]; then
  echo "pre-commit: stilltrue (language: rust — builds the hook, which takes a while)"
  code=$(PATH="${PATH#"$(dirname "$ST"):"}" run_hook stilltrue)
  is "the rust hook builds and passes a clean repository" Passed "$code"
fi

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
