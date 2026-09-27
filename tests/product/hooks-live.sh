#!/usr/bin/env bash
# Live host smoke test for the agent adapters. Replay tests in `cargo test` hold the
# adapter to each host's documented contracts; this runs the real host, once, to show
# the contracts are the ones it keeps.
#
#   STILLTRUE_LIVE_BUDGET_USD=0.25 tests/product/hooks-live.sh
#
# It is not part of `cargo test`: it runs a real agent on the operator's own model
# access, which costs money, so every run carries a spending cap. Advisory mode only.
# A host that is not installed is reported UNTESTED — a result, never a pass.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
BIN="$ROOT/target/release"
{ [ -x "$BIN/stilltrue" ] && [ -x "$BIN/stilltrue-hook" ]; } \
  || { echo "no release binaries in $BIN — run: cargo build --release"; exit 2; }
BUDGET="${STILLTRUE_LIVE_BUDGET_USD:-0.25}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
export PATH="$BIN:$PATH"
KEEP="${STILLTRUE_LIVE_KEEP:-}"   # a directory to copy recorded payloads into

PASS=0; FAIL=0; RESULTS=()
ok()  { PASS=$((PASS+1)); printf '  ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL+1)); printf '  FAIL %s\n       %s\n' "$1" "$2"; }
state_field() { python3 -c 'import json,sys,glob; f=glob.glob(sys.argv[1]+"/*.json"); d=json.load(open(f[0])) if f else {}; print(eval(sys.argv[2]))' "$1" "$2" 2>/dev/null; }

echo "Claude Code"
if command -v claude >/dev/null 2>&1; then
  version="$(claude --version 2>/dev/null | head -1)"
  claude plugin validate "$ROOT/integrations/claude-code" >"$WORK/validate.out" 2>&1 \
    && ! grep -q '⚠' "$WORK/validate.out" \
    && ok "the plugin manifest validates without warnings" \
    || bad "the plugin manifest validates without warnings" "$(tail -4 "$WORK/validate.out")"
  repo="$WORK/claude-repo"
  bash "$ROOT/tests/fixtures/readme-demo/build.sh" "$repo" >/dev/null 2>&1
  state="$WORK/claude-state"; recorded="$WORK/claude-recorded"
  prompt='If your context contains a stilltrue finding, reply with only the command it names, in backticks. Otherwise reply with only NONE.'
  (cd "$repo" && STILLTRUE_HOOK_STATE="$state" STILLTRUE_HOOK_RECORD="$recorded" \
    claude -p "$prompt" --plugin-dir "$ROOT/integrations/claude-code" \
      --model haiku --max-budget-usd "$BUDGET" --no-session-persistence \
      >"$WORK/claude.out" 2>"$WORK/claude.err")
  code=$?
  [ "$code" = 0 ] && ok "claude -p exits 0 ($version)" || bad "claude -p exits 0" "exit $code: $(head -3 "$WORK/claude.err")"
  ls "$recorded"/claude-code-SessionStart-*.json >/dev/null 2>&1 \
    && ok "the SessionStart hook ran" || bad "the SessionStart hook ran" "no payload recorded"
  ls "$recorded"/claude-code-Stop-*.json >/dev/null 2>&1 \
    && ok "the Stop hook ran" || bad "the Stop hook ran" "no payload recorded"
  [ "$(state_field "$state" 'd["initial"]["completion"]')" = "complete" ] \
    && ok "the entry scan was complete" || bad "the entry scan was complete" "$(state_field "$state" 'd.get("initial")')"
  [ "$(state_field "$state" 'len(d["initial"]["findings"])')" = "1" ] \
    && ok "and found the documented rot" || bad "and found the documented rot" "$(state_field "$state" 'd.get("initial")')"
  grep -q 'make demo' "$WORK/claude.out" \
    && ok "the entry context reached the agent" \
    || bad "the entry context reached the agent" "the agent replied: $(head -c 200 "$WORK/claude.out")"
  git -C "$repo" status --porcelain | grep -qv '^?? .remember' \
    && bad "nothing was written into the repository" "$(git -C "$repo" status --porcelain | head -3)" \
    || ok "nothing was written into the repository"
  if [ -n "$KEEP" ]; then mkdir -p "$KEEP" && cp "$recorded"/*.json "$KEEP"/; fi
  # Disablement, as docs/agents.md documents it: the plugin still loaded, every hook off.
  disabled="$WORK/claude-disabled"
  (cd "$repo" && STILLTRUE_HOOK_STATE="$WORK/claude-disabled-state" STILLTRUE_HOOK_RECORD="$disabled" \
    claude -p "Reply with only OK." --plugin-dir "$ROOT/integrations/claude-code" \
      --settings '{"disableAllHooks": true}' \
      --model haiku --max-budget-usd "$BUDGET" --no-session-persistence \
      >"$WORK/claude-disabled.out" 2>&1)
  [ -z "$(ls -A "$disabled" 2>/dev/null)" ] \
    && ok "with disableAllHooks, no hook ran" \
    || bad "with disableAllHooks, no hook ran" "$(ls "$disabled")"
  RESULTS+=("claude-code | $version | $(uname -s) $(uname -m) | SessionStart, Stop | live advisory run")
else
  echo "  UNTESTED: claude is not installed"
  RESULTS+=("claude-code | - | $(uname -s) $(uname -m) | - | UNTESTED (not installed)")
fi

echo "Codex"
if command -v codex >/dev/null 2>&1; then
  version="$(codex --version 2>/dev/null | head -1)"
  repo="$WORK/codex-repo"
  bash "$ROOT/tests/fixtures/readme-demo/build.sh" "$repo" >/dev/null 2>&1
  state="$WORK/codex-state"; recorded="$WORK/codex-recorded"
  mkdir -p "$repo/.codex" && cp "$ROOT/integrations/codex/hooks.json" "$repo/.codex/hooks.json"
  echo ".codex/" >> "$repo/.git/info/exclude"
  (cd "$repo" && STILLTRUE_HOOK_STATE="$state" STILLTRUE_HOOK_RECORD="$recorded" \
    codex exec --dangerously-bypass-hook-trust \
      -c "projects.\"$repo\".trust_level=\"trusted\"" \
      "If your context contains a stilltrue finding, reply with only the command it names, in backticks. Otherwise reply with only NONE." \
      >"$WORK/codex.out" 2>"$WORK/codex.err")
  code=$?
  [ "$code" = 0 ] && ok "codex exec exits 0 ($version)" || bad "codex exec exits 0" "exit $code: $(head -3 "$WORK/codex.err")"
  ls "$recorded"/codex-SessionStart-*.json >/dev/null 2>&1 \
    && ok "the SessionStart hook ran" || bad "the SessionStart hook ran" "no payload recorded"
  ls "$recorded"/codex-Stop-*.json >/dev/null 2>&1 \
    && ok "the Stop hook ran" || bad "the Stop hook ran" "no payload recorded"
  grep -q 'make demo' "$WORK/codex.out" \
    && ok "the entry context reached the agent" || bad "the entry context reached the agent" "$(head -c 200 "$WORK/codex.out")"
  if [ -n "$KEEP" ]; then mkdir -p "$KEEP" && cp "$recorded"/*.json "$KEEP"/ 2>/dev/null; fi
  RESULTS+=("codex | $version | $(uname -s) $(uname -m) | SessionStart, Stop | live advisory run")
else
  echo "  UNTESTED: codex is not installed"
  RESULTS+=("codex | - | $(uname -s) $(uname -m) | - | UNTESTED (not installed)")
fi

echo
printf '%s\n' "${RESULTS[@]}"
echo "$PASS passed, $FAIL failed (spending cap: \$$BUDGET per host)"
[ "$FAIL" -eq 0 ] || exit 1
[ "$PASS" -gt 0 ] || exit 3
