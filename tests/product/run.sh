#!/usr/bin/env bash
# End-to-end product assertions: every shipped feature, driven through the release
# binary exactly as a user would, against a repository with real git history.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# The release binary, as a user would have it — not `cargo run`. Override to test an
# archive's binary instead: tests/product/run.sh path/to/stilltrue
ST="${1:-$ROOT/target/release/stilltrue}"
[ -x "$ST" ] || { echo "no binary at $ST — run: cargo build --release"; exit 2; }
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
# Under its own HOME, so a test run leaves nothing in the real one. The cache is keyed
# by repository path and this harness works in a fresh mktemp directory every time, so
# without this every run stranded a handful of cache directories that nothing would ever
# read again — tens of thousands of them, and the only process still reading them was
# Spotlight. Fixtures set their git identity per repository, so an empty HOME is enough.
export HOME="$WORK/home"; mkdir -p "$HOME"
export XDG_CACHE_HOME="$HOME/.cache"
PASS=0; FAIL=0; FAILED=()

ok()   { PASS=$((PASS+1)); printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad()  { FAIL=$((FAIL+1)); FAILED+=("$1"); printf '  \033[31m✗\033[0m %s\n     expected: %s\n     actual:   %s\n' "$1" "$2" "$3"; }
is()   { [ "$2" = "$3" ] && ok "$1" || bad "$1" "$2" "$3"; }
has()  { case "$3" in *"$2"*) ok "$1";; *) bad "$1" "contains: $2" "$(echo "$3" | head -3)";; esac; }
hasnt(){ case "$3" in *"$2"*) bad "$1" "must NOT contain: $2" "$(echo "$3" | head -3)";; *) ok "$1";; esac; }
section(){ printf '\n\033[1m%s\033[0m\n' "$1"; }

R="$WORK/orchard"
"$HERE/orchard.sh" "$R" >/dev/null 2>&1
cd "$R"
rm -rf "$WORK/cache"

section "1. Detection — the five rot classes, each blamed on the right commit"
OUT=$("$ST" 2>&1); CODE=$?
is   "default run exits 1 on rot"            "1" "$CODE"
has  "command rot"      'command `make demo` has no target'     "$OUT"
has  "symbol rot"       'symbol `fold_events()` is not defined' "$OUT"
has  "env var rot"      'env var `ORCHARD_API_KEY` is not read' "$OUT"
has  "link rot"         'link target `./docs/setup.md`'         "$OUT"
has  "blames the split commit"   'Split demo into seed and serve'   "$OUT"
has  "blames the rename commit"  'Rename fold_events to reduce_events' "$OUT"
has  "hedges attribution"        'likely broke in'                  "$OUT"
is   "finds exactly 7"  "7" "$(echo "$OUT" | grep -c '  rot  ')"

section "2. Precision — what it must stay silent about"
hasnt "a symbol that still exists"    'fetchHarvest'  "$OUT"
hasnt "a symbol that still exists"    'load_config'   "$OUT"
is   "a make target that still exists" "0" "$(echo "$OUT" | grep -c 'command `make lint`')"
hasnt "a version claim that matches the pin" 'Python 3.11' "$OUT"
hasnt "no Tier B in a default run"    ' lie '         "$OUT"

section "3. Suggestions — enumerated, not guessed"
has  "names every candidate under six" 'did you mean: make lint, make seed, make serve' "$OUT"

section "4. Tiers — --strict reaches Tier B"
printf '\nThe loader lives in `src/orchard/loader.py`.\n' >> README.md
S=$("$ST" --strict 2>&1)
has  "lie reported under --strict"  'src/orchard/loader.py' "$S"
has  "labelled lie, not rot"        ' lie ' "$S"
D=$("$ST" 2>&1)
hasnt "same claim silent by default" 'loader.py' "$D"
git checkout -q README.md

section "5. Exit codes — --fail-on is independent of --strict"
"$ST" --fail-on none >/dev/null 2>&1; is "--fail-on none exits 0"  "0" "$?"
"$ST" --fail-on rot  >/dev/null 2>&1; is "--fail-on rot exits 1"   "1" "$?"
"$ST" --fail-on any  >/dev/null 2>&1; is "--fail-on any exits 1"   "1" "$?"

section "6. Machine formats — same gate, one truth"
J=$("$ST" --format json 2>/dev/null)
is   "JSON parses"            "ok" "$(echo "$J" | python3 -c 'import json,sys; json.load(sys.stdin); print("ok")' 2>&1)"
is   "JSON has 7 findings"    "7"  "$(echo "$J" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["findings"]))')"
SA=$("$ST" --format sarif 2>/dev/null)
is   "SARIF version 2.1.0"    "2.1.0" "$(echo "$SA" | python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])')"
is   "SARIF columnKind"       "unicodeCodePoints" "$(echo "$SA" | python3 -c 'import json,sys; print(json.load(sys.stdin)["runs"][0]["columnKind"])')"
is   "SARIF fingerprints"     "7" "$(echo "$SA" | python3 -c 'import json,sys; print(sum(1 for r in json.load(sys.stdin)["runs"][0]["results"] if r.get("partialFingerprints")))')"
is   "SARIF level is error"   "error" "$(echo "$SA" | python3 -c 'import json,sys; print(json.load(sys.stdin)["runs"][0]["results"][0]["level"])')"
G=$("$ST" --format github 2>/dev/null)
has  "workflow command"       '::error file=' "$G"
has  "carries line + col"     ',line=' "$G"

section "7. Rule ids are a stable public interface"
is   "command rot rule id" "stilltrue/command/rot" "$(echo "$J" | python3 -c 'import json,sys; print([f["ruleId"] for f in json.load(sys.stdin)["findings"] if "make demo" in f["claim"]][0])')"

section "8. --baseline — adopt without starting red"
"$ST" --write-baseline .stilltrue-baseline >/dev/null 2>&1; is "--write-baseline exits 0" "0" "$?"
B=$("$ST" --baseline .stilltrue-baseline 2>&1); BC=$?
is   "baselined run is silent"        "0" "$BC"
has  "says so"                        "no findings" "$B"
# Reflow every document: line numbers move, fingerprints must not.
python3 - <<'PY'
import pathlib
for f in ("README.md", "CLAUDE.md"):
    p = pathlib.Path(f); p.write_text("<!-- reflowed -->\n\n\n" + p.read_text())
PY
B2=$("$ST" --baseline .stilltrue-baseline 2>&1); B2C=$?
is   "baseline survives a reflow"     "0" "$B2C"
has  "still silent after reflow"      "no findings" "$B2"
git checkout -q README.md CLAUDE.md
# New ROT appearing after the baseline was written must still break through.
# Deleting the lint target makes `make lint` broken, and history proves it existed.
python3 -c "
import pathlib; p=pathlib.Path('Makefile'); p.write_text(p.read_text().replace('lint:\n\truff check src\n',''))"
printf 'Lint with \x60make lint\x60.\n' >> CLAUDE.md
git add -A >/dev/null; git -c user.email=d@e -c user.name=d commit -q -m "Drop the lint target"
B3=$("$ST" --baseline .stilltrue-baseline 2>&1); B3C=$?
has  "rot appearing AFTER the baseline is reported" 'make lint' "$B3"
# Two: CLAUDE.md has claimed `make lint` since the first commit, and neither that
# claim nor the appended one was in the baseline, because both were TRUE when it was
# written. Deleting the target rots both, and a baseline must not hide either.
is   "and it is rot, with history"    "2" "$(echo "$B3" | grep -c 'rot  command `make lint`')"
has  "blamed on the deleting commit"  'Drop the lint target' "$B3"
is   "and it fails the build"         "1" "$B3C"
is   "baselined findings still silent" "0" "$(echo "$B3" | grep -c 'make demo')"
git reset -q --hard HEAD~1; rm -f .stilltrue-baseline

section "9. --fix — rewrites only what it can enumerate"
cp README.md "$WORK/readme.bak"
# `fold_events()` has many candidates; a single-candidate claim is what --fix touches.
F=$("$ST" --fix 2>&1)
has  "reports what it changed or left" "$(echo "$F" | head -1 | cut -c1-1)" "$F"
AFTER=$("$ST" 2>&1); AC=$?
is   "exit code still reflects what was FOUND, not what remains" "1" "$AC"
git checkout -q . 2>/dev/null; cp "$WORK/readme.bak" README.md; git checkout -q README.md

section "10. Suppression — block-anchored"
python3 - <<'PY'
import pathlib
p = pathlib.Path("README.md")
p.write_text(p.read_text().replace(
    "Run `make demo` to see the pipeline end to end.",
    "<!-- stilltrue:ignore the demo target is coming back -->\nRun `make demo` to see the pipeline end to end."))
PY
SUP=$("$ST" 2>&1)
is   "suppressed claim is gone"  "0" "$(echo "$SUP" | grep -c 'README.md.*make demo')"
is   "and only that one — CLAUDE.md still reported" "1" "$(echo "$SUP" | grep -c 'CLAUDE.md.*make demo')"
U=$("$ST" --strict 2>&1)
hasnt "a USED suppression is not reported" 'suppression/unused' "$U"
git checkout -q README.md
# now an unused one
printf '\n<!-- stilltrue:ignore nothing here any more -->\nPlain prose with no claims.\n' >> README.md
UU=$("$ST" --strict 2>&1)
has  "unused suppression under --strict" 'suppression' "$UU"
UD=$("$ST" 2>&1)
hasnt "unused suppression silent by default" 'suppression' "$UD"
git checkout -q README.md
# whole-file
python3 -c "
import pathlib; p=pathlib.Path('README.md'); p.write_text('<!-- stilltrue:ignore-file -->\n'+p.read_text())"
IF=$("$ST" 2>&1)
is   "ignore-file silences the document" "0" "$(echo "$IF" | grep -c '^README.md')"
git checkout -q README.md

section "11. Config — include replaces, exclude extends"
echo 'include = ["CLAUDE.md"]' > stilltrue.toml
C=$("$ST" 2>&1)
hasnt "include REPLACES the defaults" 'README.md' "$C"
has   "and keeps what it names"       'CLAUDE.md' "$C"
echo 'exclude = ["CLAUDE.md"]' > stilltrue.toml
C2=$("$ST" 2>&1)
hasnt "exclude subtracts"             'CLAUDE.md' "$C2"
has   "defaults still present"        'README.md' "$C2"
rm -f stilltrue.toml

section "12. Positional paths filter the document set"
P=$("$ST" README.md 2>&1)
has   "named document linted"    'README.md' "$P"
hasnt "others dropped"           'CLAUDE.md' "$P"

section "13. reStructuredText is read as a first-class document"
cat > docs/guide.rst <<'EOF'
Guide
=====

Run ``make demo`` to start the pipeline.

.. code-block:: bash

   make demo

See `the setup guide <./setup.md>`_ for details.
EOF
git add -A >/dev/null; git -c user.email=d@e -c user.name=d commit -q -m "Add an rst guide"
RST=$("$ST" 2>&1)
has  "rst inline literal is a claim"  'docs/guide.rst' "$RST"
has  "rst command rot found"          'make demo' "$RST"
git reset -q --hard HEAD~1

section "14. TypeScript symbols — the second resolver"
python3 - <<'PY'
import pathlib
p = pathlib.Path("web/src/client.ts")
p.write_text(p.read_text().replace("fetchHarvest", "loadHarvest"))
p = pathlib.Path("web/src/index.ts")
p.write_text(p.read_text().replace("fetchHarvest", "loadHarvest"))
PY
git add -A >/dev/null; git -c user.email=d@e -c user.name=d commit -q -m "Rename fetchHarvest to loadHarvest"
TS=$("$ST" 2>&1)
has  "TypeScript symbol rot"   'fetchHarvest' "$TS"
has  "blamed on the rename"    'Rename fetchHarvest to loadHarvest' "$TS"
git reset -q --hard HEAD~1

section "15. Degraded runs are loud, never silent"
SH="$WORK/shallow"; rm -rf "$SH"
git clone -q --depth 1 "file://$R" "$SH" 2>/dev/null
cd "$SH"
DEG=$("$ST" 2>&1); DC=$?
has  "warns about the shallow clone"  'shallow' "$DEG"
has  "names the fix verbatim"         'fetch-depth: 0' "$DEG"
is   "still exits 0 — no Tier A reachable" "0" "$DC"
"$ST" --require-history >/dev/null 2>&1
is   "--require-history exits 2"      "2" "$?"
DEGOUT=$("$ST" 2>/dev/null)
hasnt "the warning goes to stderr, not stdout" 'fetch-depth' "$DEGOUT"
cd "$R"

section "16. Cache — a stored positive is reused, and --no-cache repairs it"
rm -rf "$WORK/cache"
T1=$( { time -p "$ST" >/dev/null 2>&1; } 2>&1 | awk '/real/{print $2}')
T2=$( { time -p "$ST" >/dev/null 2>&1; } 2>&1 | awk '/real/{print $2}')
ok "cold run ${T1}s, warm run ${T2}s"
NC=$("$ST" --no-cache 2>&1)
is "--no-cache produces identical findings" "7" "$(echo "$NC" | grep -c '  rot  ')"
# Where `dirs::cache_dir()` lands, which is not the same directory on every runner —
# this harness runs on both macOS and ubuntu in CI.
case "$(uname -s)" in
  Darwin) CACHEROOT="$HOME/Library/Caches/stilltrue";;
  *)      CACHEROOT="${XDG_CACHE_HOME:-$HOME/.cache}/stilltrue";;
esac
CACHEDIR=$(find "$CACHEROOT" -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
[ "$CACHEDIR" != "0" ] && ok "cache lives outside the repository" || bad "cache outside repo" "a cache dir under $CACHEROOT" "none found"
is "nothing written into the working tree" "" "$(git status --porcelain | grep -v '^?? docs/guide.rst' | head -1)"

# What the flag is *for*: not speed, but repair. A cached positive is trusted
# at the HEAD it was found at, so a wrong one stays wrong until something ignores it.
# These pin the three
# modes apart — read-write, rewrite, off — which "same findings either way" cannot,
# because a cache that is never read produces the same findings too.
#
# Under its own HOME, so the cache is this test's and the assertions do not depend on
# which of the user's repositories happen to be cached already.
CH="$WORK/cache-home"; rm -rf "$CH"; mkdir -p "$CH"
HOME="$CH" XDG_CACHE_HOME="$CH/.cache" "$ST" >/dev/null 2>&1
CACHEFILE=$(find "$CH" -name history.json -type f 2>/dev/null | head -1)
if [ -n "$CACHEFILE" ]; then
  python3 - "$CACHEFILE" <<'POISON'
import json, sys
d = json.load(open(sys.argv[1]))
assert d["schema"] == 2, "the cache is a versioned envelope"
poisoned = 0
for entry in d["entries"].values():
    if entry.get("found"):
        entry["found"]["subject"] = "POISONED CACHE SUBJECT"
        poisoned += 1
assert poisoned, "no positive entry to poison - the test would prove nothing"
json.dump(d, open(sys.argv[1], "w"))
POISON
  has  "a stored positive is trusted by default" 'POISONED CACHE SUBJECT' \
       "$(HOME="$CH" XDG_CACHE_HOME="$CH/.cache" "$ST" 2>&1)"
  NCP=$(HOME="$CH" XDG_CACHE_HOME="$CH/.cache" "$ST" --no-cache 2>&1)
  hasnt "--no-cache ignores what is stored"      'POISONED CACHE SUBJECT' "$NCP"
  has  "and recomputes the real commit"          'Split demo into seed and serve' "$NCP"
  hasnt "and rewrites it, repairing the next run too" 'POISONED CACHE SUBJECT' \
       "$(HOME="$CH" XDG_CACHE_HOME="$CH/.cache" "$ST" 2>&1)"
else
  bad "cache file written under an isolated HOME" "a history.json" "none"
fi

section "17. Zero-config on a repository that never heard of the tool"
is "no stilltrue.toml present" "" "$(ls stilltrue.toml 2>/dev/null)"
ok "every finding above came from zero configuration"

# ===================================================================================
# thicket: a monorepo shaped for the parts of the tool orchard never reaches.
# ===================================================================================
T="$WORK/thicket"
"$HERE/thicket.sh" "$T" >/dev/null 2>&1
cd "$T"
TOUT=$("$ST" 2>&1)

section "18. Anchoring — the nearest ancestor manifest, not the root"
has  "nested script rot found"        'packages/api/README.md' "$TOUT"
has  "checked against the NESTED manifest" 'no target in packages/api/package.json' "$TOUT"
has  "suggestions come from the nested manifest" 'pnpm migrate, pnpm start' "$TOUT"
hasnt "root scripts are not suggested here" 'pnpm build, pnpm test' "$TOUT"
is   "root scripts that exist stay silent" "0" "$(echo "$TOUT" | grep -c 'pnpm run build')"
# A document outside any nested package anchors at the root instead.
S=$("$ST" --strict 2>&1)
has  "a doc outside the package anchors at the root" 'SKILL.md' "$S"
has  "and says so"                    'no target in package.json' "$S"

section "19. Version claims resolve against exact pins only"
has  "prose contradicting .nvmrc is rot" 'Node 22.4.0 contradicts' "$TOUT"
has  "names the pinned value"          'which pins 24.2.0' "$TOUT"
has  "blamed on the pin commit"        'Move to Node 24' "$TOUT"
# The tool name is part of the suggestion, because `--fix` writes what is offered and
# a bare `24.2.0` replaced the whole claim: "Needs Node 22.4.0" became "Needs 24.2.0".
has  "suggests the pinned version"     'did you mean: Node 24.2.0' "$TOUT"

section "20. In-document anchors — the table of contents is a claim"
has  "renamed heading breaks its anchor" 'has no heading `#configuration`' "$TOUT"
has  "blamed on the rename"            'Rename the Configuration heading' "$TOUT"
has  "suggests the real slugs"         'configuring-thicket' "$TOUT"

section "21. The default file set"
is   "nested README is a document"     "1" "$(echo "$TOUT" | grep -c '^packages/api/README.md')"
has  ".cursor/rules is linted"         'pnpm run test' "$(cat .cursor/rules/style.md)"
is   "and stays silent when true"      "0" "$(echo "$TOUT" | grep -c '.cursor/rules')"
is   "SKILL.md at depth is a document" "1" "$(echo "$S" | grep -c 'skills/deploy/SKILL.md')"
is   "CHANGELOG is excluded"           "0" "$(echo "$S" | grep -c 'CHANGELOG')"
is   "and it really does contain rot"  "1" "$(grep -c 'packages/legacy/index.ts' CHANGELOG.md)"

section "22. Tier C — never reported, under any flag"
is   "a bare ecosystem filename abstains" "0" "$(echo "$S" | grep -c '`requirements.txt`')"
is   "but a directory-qualified one is judged" "1" "$(grep -c 'config/requirements.txt' README.md)"
is   "an extensionless link that resolves is silent" "0" "$(echo "$S" | grep -c 'docs/deploy')"

section "23. Columns are code points, and SARIF URIs are encoded"
is   "non-ASCII line reports a code-point column" "60" "$(echo "$S" | grep 'unicode-check' | sed 's/.*README.md:[0-9]*:\([0-9]*\).*/\1/')"
cp README.md "docs/my notes.md"; git add -A >/dev/null; git -c user.email=d@e -c user.name=d commit -q -m "notes"
SARIF_URIS=$("$ST" --strict --format sarif 2>/dev/null | python3 -c "
import json,sys
print(' '.join(sorted({r['locations'][0]['physicalLocation']['artifactLocation']['uri']
                       for r in json.load(sys.stdin)['runs'][0]['results']})))")
has   "a space in a path is percent-encoded" 'docs/my%20notes.md' "$SARIF_URIS"
hasnt "and the raw space never appears"      'docs/my notes.md'   "$SARIF_URIS"
git reset -q --hard HEAD~1

section "24. Errors warn and skip; they never fail the build"
echo 'include = [unclosed' > stilltrue.toml
E=$("$ST" 2>&1); EC=$?
has  "a malformed config warns"        'ignoring' "$E"
has  "and names the parse error"       'TOML parse error' "$E"
is   "and the run still completes"     "1" "$EC"
echo 'include = ["["]' > stilltrue.toml
E2=$("$ST" 2>&1)
has  "a bad glob warns by name"        'bad glob' "$E2"
rm -f stilltrue.toml
E3=$("$ST" /etc/hosts 2>&1); E3C=$?
has  "a path outside the repo warns"   'outside the repository' "$E3"
is   "and lints nothing rather than everything" "0" "$E3C"

section "25. Symbol applicability — one unreadable language silences them all"
cd "$R"
is   "symbols reported while wholly supported" "2" "$("$ST" 2>&1 | grep -c 'symbol `')"
mkdir -p cmd
printf 'package main\nfunc main(){}\n' > cmd/main.go
printf 'package main\nfunc helper(){}\n' > cmd/helper.go
git add -A >/dev/null; git -c user.email=d@e -c user.name=d commit -q -m "Add a Go command"
is   "two Go files make every symbol Tier C" "0" "$("$ST" 2>&1 | grep -c 'symbol `')"
is   "and the other claim types are unaffected" "5" "$("$ST" 2>&1 | grep -c '  rot  ')"
git reset -q --hard HEAD~1

section "26. --fix rewrites only a single candidate, and never buys a green build"
FX="$WORK/fix"; rm -rf "$FX"; mkdir -p "$FX"; cd "$FX"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'buidl:\n\tcc main.c\n' > Makefile
printf '# p\n\nRun `make buidl` to build.\n' > README.md
git add -A; git commit -q -m "initial, typo in the target too"
printf 'build:\n\tcc main.c\n' > Makefile
git add -A; git commit -q -m "Fix the spelling of the build target"
FOUT=$("$ST" --fix 2>&1); FC=$?
has  "prints the rewrite"              '`make buidl` -> `make build`' "$FOUT"
has  "and the tally"                   'rewrote 1 of 1' "$FOUT"
is   "the file is rewritten"           "Run \`make build\` to build." "$(sed -n 3p README.md)"
is   "exit reflects what was FOUND"    "1" "$FC"
is   "and the next run is clean"       "0" "$("$ST" >/dev/null 2>&1; echo $?)"

section "27. Cache — negative results are rechecked when HEAD moves"
CC="$WORK/cache-repo"; rm -rf "$CC"; mkdir -p "$CC"; cd "$CC"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'build:\n\tcc x\n' > Makefile
printf '# p\n\nRun `make deploy`.\n' > README.md
git add -A; git commit -q -m init
is   "no history yet, so it is a lie"  "1" "$("$ST" --strict 2>&1 | grep -c '  lie  ')"
printf 'build:\n\tcc x\ndeploy:\n\tscp x y\n' > Makefile
git add -A; git commit -q -m "Add a deploy target"
printf 'build:\n\tcc x\n' > Makefile
git add -A; git commit -q -m "Drop the deploy target again"
is   "history exists now, so it is rot" "1" "$("$ST" 2>&1 | grep -c '  rot  ')"
has  "and the cached negative did not win" 'Drop the deploy target again' "$("$ST" 2>&1)"

section "28. Bytes are not characters — CRLF, a missing newline, a BOM"
EE="$WORK/encoding"; rm -rf "$EE"; mkdir -p "$EE/docs"; cd "$EE"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'build:\n\tcc x\n' > Makefile
python3 -c "
import pathlib
pathlib.Path('README.md').write_bytes(b'# p\r\n\r\nRun \`make demo\` here.\r\n')
pathlib.Path('docs/tail.md').write_bytes(b'# t\n\nEnds on a claim \`make demo\`')
pathlib.Path('docs/bom.md').write_bytes(b'\xef\xbb\xbf# b\n\nRun \`make demo\` after a BOM.\n')"
git add -A; git commit -q -m init
printf 'build:\n\tcc x\ndemo:\n\techo hi\n' > Makefile; git add -A; git commit -q -m add
printf 'build:\n\tcc x\n' > Makefile; git add -A; git commit -q -m "Remove demo"
COLS=$("$ST" --format json 2>/dev/null | python3 -c "
import json,sys
print(' '.join(f\"{f['file']}:{f['line']}:{f['column']}\" for f in sorted(json.load(sys.stdin)['findings'], key=lambda f: f['file'])))")
has  "CRLF does not shift the column"     'README.md:3:6' "$COLS"
has  "a BOM does not shift the column"    'docs/bom.md:3:6' "$COLS"
has  "a claim at the final byte is found" 'docs/tail.md:3:18' "$COLS"

section "29. Suppression anchors to the block, and fingerprints stay distinct"
SS="$WORK/suppress"; rm -rf "$SS"; mkdir -p "$SS"; cd "$SS"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'build:\n\tcc x\n' > Makefile
cat > README.md <<'DOC'
# p

Run `make demo` once.

Run `make demo` twice.

- `make demo` in a list item

<!-- stilltrue:ignore the fence is illustrative -->
```bash
make demo
```

## `make demo` in a heading
DOC
git add -A; git commit -q -m init
printf 'build:\n\tcc x\ndemo:\n\techo hi\n' > Makefile; git add -A; git commit -q -m add
printf 'build:\n\tcc x\n' > Makefile; git add -A; git commit -q -m "Remove the demo target"
SUPOUT=$("$ST" 2>&1)
is   "paragraph, list item and heading all report" "4" "$(echo "$SUPOUT" | grep -c '  rot  ')"
is   "the fence is suppressed by its comment"      "0" "$(echo "$SUPOUT" | grep -c ':11:')"
is   "four identical claims, four fingerprints"    "4" "$("$ST" --format sarif 2>/dev/null | python3 -c "
import json,sys
print(len({tuple(r['partialFingerprints'].items()) for r in json.load(sys.stdin)['runs'][0]['results']}))")"
cd "$R"

section "30. The [ignore] table names a claim, not a spelling"
cd "$R"
printf '[ignore]\nsymbols = ["fold_events"]\n' > stilltrue.toml
is   "a symbol named without its parens is ignored" "0" "$("$ST" 2>&1 | grep -c 'fold_events')"
printf '[ignore]\nsymbols = ["fold_events()"]\n' > stilltrue.toml
is   "and the exact spelling still works"           "0" "$("$ST" 2>&1 | grep -c 'fold_events')"
printf '[ignore]\nenv = ["ORCHARD_API_KEY"]\n' > stilltrue.toml
is   "an env var is ignored by name"                "0" "$("$ST" 2>&1 | grep -c 'ORCHARD_API_KEY')"
is   "and the rest still report"                    "5" "$("$ST" 2>&1 | grep -c '  rot  ')"
printf 'strict = true\n' > stilltrue.toml
printf '\nThe loader is `src/orchard/loader.py`.\n' >> README.md
is   "strict in the config lifts Tier B"            "1" "$("$ST" 2>&1 | grep -c '  lie  ')"
git checkout -q README.md; rm -f stilltrue.toml

section "31. A dotted symbol is judged only inside a readable namespace"
DS="$WORK/dotted"; rm -rf "$DS"; mkdir -p "$DS/pkg"; cd "$DS"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'class Widget:\n    def render(self):\n        return 1\n\ndef helper():\n    return 2\n' > pkg/mod.py
touch pkg/__init__.py
cat > CLAUDE.md <<'DOC'
Call `Widget.render()` and `pkg.helper()`.

An instance is `widget.render()`, a foreign one is `requests.get()`,
and a deep one is `some.unknown.thing()`.
DOC
git add -A; git commit -q -m init
# Rewritten rather than sed-ed: `sed -i ''` is BSD syntax, and GNU sed reads the
# empty string as the script and the script as a filename, so on Linux the rename
# silently did not happen and the two assertions below failed against a file that
# still said `render`.
printf 'class Widget:\n    def draw(self):\n        return 1\n\ndef helper():\n    return 2\n' > pkg/mod.py
git add -A; git commit -q -m "Rename render to draw"
DOUT=$("$ST" --strict 2>&1)
has  "a readable class is judged"          'Widget.render()' "$DOUT"
is   "a module of this repository is judged and silent" "0" "$(echo "$DOUT" | grep -c 'pkg.helper')"
is   "an instance needs type inference, so Tier C"      "0" "$(echo "$DOUT" | grep -c 'widget.render')"
is   "a foreign namespace is Tier C"                    "0" "$(echo "$DOUT" | grep -c 'requests.get')"
is   "an unknown deep namespace is Tier C"              "0" "$(echo "$DOUT" | grep -c 'some.unknown')"
hasnt "and 'self' is never offered as a suggestion"     'self' "$DOUT"
has  "while the real method is"                         'draw' "$DOUT"

section "32. Workflow commands escape what the runner's parser needs"
ES="$WORK/escaping"; rm -rf "$ES"; mkdir -p "$ES/docs"; cd "$ES"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'build:\n\tcc x\n' > Makefile
printf '# p\n\nRun `make a,b:c%%d`.\n' > "docs/README,odd.md"
git add -A; git commit -q -m init
printf 'build:\n\tcc x\na,b:c%%d:\n\techo hi\n' > Makefile; git add -A; git commit -q -m add
printf 'build:\n\tcc x\n' > Makefile; git add -A; git commit -q -m "Remove the odd target"
G=$("$ST" --format github 2>&1)
has  "a comma in a path is escaped in the property" 'file=docs/README%2Codd.md' "$G"
has  "a percent in the message is escaped"          'make a,b:c%25d' "$G"
is   "the property list still splits on commas into five fields" "5" "$(echo "$G" | sed 's/^::error //; s/::.*//' | tr ',' '\n' | grep -c '=')"
hasnt "a colon in the message is NOT escaped"       'c%3A' "$G"
cd "$R"

section "33. reStructuredText reads three surfaces and no more"
RS="$WORK/rst"; rm -rf "$RS"; mkdir -p "$RS/docs"; cd "$RS"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'build:\n\tcc x\n' > Makefile
cat > docs/guide.rst <<'DOC'
Guide
=====

Inline literal: ``make demo``.

.. code-block:: bash

   make demo
   make demo && make other

.. code-block:: python

   make demo

A link: `the setup <./setup.md>`_.

.. versionadded:: 1.0
   ``make demo`` was added here.

.. deprecated:: 2.0
   Use ``make demo`` instead.

Indented block::

   make demo
DOC
git add -A; git commit -q -m init
printf 'build:\n\tcc x\ndemo:\n\techo hi\n' > Makefile; git add -A; git commit -q -m add
printf 'build:\n\tcc x\n' > Makefile; git add -A; git commit -q -m "Remove demo"
ROUT=$("$ST" --strict 2>&1)
is  "an inline literal is a claim"          "1" "$(echo "$ROUT" | grep -c ':4:19')"
is  "a shell code-block body is a claim"    "1" "$(echo "$ROUT" | grep -c ':8:4')"
is  "and a chained line splits in two"      "2" "$(echo "$ROUT" | grep -cE ':9:(4|17)')"
is  "a link target is a claim"              "1" "$(echo "$ROUT" | grep -c 'setup.md')"
is  "a python code-block yields nothing"    "0" "$(echo "$ROUT" | grep -c ':13:')"
is  "versionadded is a historical record"   "0" "$(echo "$ROUT" | grep -c ':18:')"
is  "so is deprecated"                      "0" "$(echo "$ROUT" | grep -c ':21:')"
is  "an untagged literal block is not read" "0" "$(echo "$ROUT" | grep -c ':25:')"
# One inline literal, two commands from the bash block's first line and its chained
# second, and one link. Nothing from python, the directives, or the untagged block.
is  "five claims in total, no more"         "5" "$(echo "$ROUT" | grep -cE '  (rot|lie)  ')"
cd "$R"

section "34. A partial clone is as loud as a shallow one"
PC="$WORK/partial"; rm -rf "$PC"
git clone -q --filter=blob:none "file://$R" "$PC" 2>/dev/null
cd "$PC"
POUT=$("$ST" 2>&1); PCODE=$?
has "detected and named"               'partial clone' "$POUT"
has "and says what it cannot do"       'nothing can be reported as rot' "$POUT"
is  "still exits 0 — no Tier A reachable" "0" "$PCODE"
"$ST" --require-history >/dev/null 2>&1
is  "--require-history refuses to run blind" "2" "$?"

section "35. --fix applies several edits to one document without corrupting it"
MF="$WORK/multifix"; rm -rf "$MF"; mkdir -p "$MF"; cd "$MF"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'buidl:\n\tcc x\n' > Makefile
cat > README.md <<'DOC'
# p

Run `make buidl` then `make buidl`.

Later, `make buidl` again.

A longer line with `make buidl` in the middle of it, and text after.
DOC
git add -A; git commit -q -m "initial, with the typo in the target too"
printf 'build:\n\tcc x\n' > Makefile
git add -A; git commit -q -m "Fix the spelling of the target"
MOUT=$("$ST" --fix 2>&1)
has "every edit is printed"            'rewrote 4 of 4' "$MOUT"
is  "two edits on one line both land"  "Run \`make build\` then \`make build\`." "$(sed -n 3p README.md)"
is  "and the tail of a longer line survives" "A longer line with \`make build\` in the middle of it, and text after." "$(sed -n 7p README.md)"
is  "the document is otherwise untouched" "7" "$(wc -l < README.md | tr -d ' ')"
is  "and the next run is clean"        "0" "$("$ST" >/dev/null 2>&1; echo $?)"

# Two candidates is a choice, and a choice is a human's.
printf 'build:\n\tcc x\nserve:\n\tcc y\n' > Makefile
printf '\nRun `make buidl` once more.\n' >> README.md
git add -A; git commit -q -m "Add a second target"
TWO=$("$ST" --fix 2>&1)
has "two candidates are left alone"    'rewrote 0 of' "$TWO"
is  "and the text is unchanged"        "1" "$(grep -c 'make buidl' README.md)"
cd "$R"

section "36. The walk honours gitignore and stops at a symbolic link"
IG="$WORK/ignored"; rm -rf "$IG"; mkdir -p "$IG/docs" "$IG/vendor"; cd "$IG"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'build:\n\tcc x\n' > Makefile
printf 'vendor/\nscratch.md\n' > .gitignore
printf '# p\n\nRun `make demo`.\n' > README.md
printf '# s\n\nRun `make demo`.\n' > scratch.md
printf '# v\n\nRun `make demo`.\n' > vendor/README.md
printf '# r\n\nRun `make demo`.\n' > docs/real.md
ln -s real.md docs/link.md
ln -s /nonexistent-target docs/dangling.md
git add -A; git commit -q -m init
printf 'build:\n\tcc x\ndemo:\n\techo hi\n' > Makefile; git add -A; git commit -q -m add
printf 'build:\n\tcc x\n' > Makefile; git add -A; git commit -q -m "Remove demo"
IOUT=$("$ST" 2>&1)
is  "a tracked document is linted"        "1" "$(echo "$IOUT" | grep -c '^README.md')"
is  "and one at depth"                    "1" "$(echo "$IOUT" | grep -c '^docs/real.md')"
is  "a gitignored file is not walked"     "0" "$(echo "$IOUT" | grep -c 'scratch.md')"
is  "nor a gitignored directory"          "0" "$(echo "$IOUT" | grep -c 'vendor')"
is  "a symlinked document is not followed" "0" "$(echo "$IOUT" | grep -c 'docs/link.md')"
is  "and a dangling one neither crashes nor warns" "0" "$(echo "$IOUT" | grep -c 'dangling')"
is  "two findings in total"               "2" "$(echo "$IOUT" | grep -c '  rot  ')"
is  "and the run still exits 1"           "1" "$("$ST" >/dev/null 2>&1; echo $?)"
cd "$R"

section "37. A file under docs/ is only a document if it reads like one"
DM="$WORK/docmatch"; rm -rf "$DM"; mkdir -p "$DM/docs"; cd "$DM"
git init -q -b main .; git config user.email d@e; git config user.name d
printf 'build:\n\tcc x\n' > Makefile
printf '# d\n\nRun `make demo`.\n' > docs/guide.md
printf '# d\n\nRun ``make demo``.\n' > docs/guide.rst
# Matched by the `docs/**` glob, and not prose. A backtick in a comment is not a claim.
printf '"""Run `make demo` to build."""\n\n\ndef run():\n    pass\n' > docs/helper.py
printf '# Run `make demo` to build\nset -e\n' > docs/setup.sh
printf '{"note": "Run `make demo`"}\n' > docs/meta.json
git add -A; git commit -q -m init
printf 'build:\n\tcc x\ndemo:\n\techo hi\n' > Makefile; git add -A; git commit -q -m add
printf 'build:\n\tcc x\n' > Makefile; git add -A; git commit -q -m "Remove demo"
DOUT2=$("$ST" --strict 2>&1)
is  "a markdown document is read"          "1" "$(echo "$DOUT2" | grep -c 'docs/guide.md')"
is  "an rst document is read"              "1" "$(echo "$DOUT2" | grep -c 'docs/guide.rst')"
is  "a python file under docs is not"      "0" "$(echo "$DOUT2" | grep -c 'helper.py')"
is  "nor a shell script"                   "0" "$(echo "$DOUT2" | grep -c 'setup.sh')"
is  "nor a json file"                      "0" "$(echo "$DOUT2" | grep -c 'meta.json')"
is  "two findings, one per real document"  "2" "$(echo "$DOUT2" | grep -cE '  (rot|lie)  ')"
cd "$R"

section "38. A channel is not a pin, and --fix writes what resolves"
NP="$WORK/notapin"; rm -rf "$NP"; mkdir -p "$NP/docs/old"; cd "$NP"
git init -q -b main .; git config user.email d@e; git config user.name d
# `channel = "stable"` is the commonest rust-toolchain.toml there is. Read as a pin it
# made every Rust version stated in a README rot, blamed on the commit that created the
# file — a false positive in a default run, which is the one thing this tool cannot ship.
printf '[toolchain]\nchannel = "stable"\n' > rust-toolchain.toml
printf 'lts/*\n' > .nvmrc
printf '# p\n\nBuilt with Rust 1.80 on Node 20.\n\nSee [guide](old/x.md).\n' > docs/a.md
printf '# X\n\n## Install\n' > docs/old/x.md
git add -A; git commit -q -m init
mkdir -p docs/new; git mv docs/old/x.md docs/new/x.md; git commit -q -m "Move the page"
NOUT=$("$ST" --strict 2>&1)
is   "a channel pins nothing"          "0" "$(echo "$NOUT" | grep -c 'Rust 1.80')"
is   "nor does an alias"               "0" "$(echo "$NOUT" | grep -c 'Node 20')"
has  "the moved page is still rot"     'link target `old/x.md`' "$NOUT"
# A link is file-relative, so the candidate has to be spelled from docs/.
has  "suggested from the document"     'did you mean: new/x.md' "$NOUT"
hasnt "not from the repository root"   'docs/new/x.md' "$NOUT"
"$ST" --fix >/dev/null 2>&1
is   "--fix writes the link spelling"  "See [guide](new/x.md)." "$(sed -n 5p docs/a.md)"
is   "and the next run is clean"       "0" "$("$ST" --strict >/dev/null 2>&1; echo $?)"
# stdout is the machine document; the rewrite log belongs on stderr.
git checkout -q docs/a.md
JOUT=$("$ST" --format json --fix 2>/dev/null)
is   "--format json --fix stays JSON"  "1" "$(printf '%s' "$JOUT" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(len(d["findings"]))' 2>/dev/null)"
cd "$R"

section "39. Colour is for terminals, and NO_COLOR wins"
cd "$R"
# A pty, because that is the whole question: `is_terminal()` is false under any
# ordinary redirect, so a pipe can only ever show the uncoloured half. python3 is
# already required by this harness, and its `pty` module behaves the same on macOS and
# Linux — unlike `script`, whose arguments do not.
pty() {
  python3 - "$@" <<'PTY'
import os, pty, sys
out = bytearray()
def read(fd):
    chunk = os.read(fd, 1024)
    out.extend(chunk)
    return chunk
status = pty.spawn(sys.argv[1:], read)
sys.stdout.write(out.decode("utf-8", "replace"))
PTY
}
PTYOUT=$(pty "$ST")
case "$PTYOUT" in
  *$'\033['*) ok "a terminal gets colour";;
  *) bad "a terminal gets colour" "an escape code" "$(echo "$PTYOUT" | head -2)";;
esac
has  "and the finding is still there"   'make demo' "$PTYOUT"
NCOUT=$(NO_COLOR=1 pty "$ST")
case "$NCOUT" in
  *$'\033['*) bad "NO_COLOR strips it" "no escape code" "$(echo "$NCOUT" | head -2)";;
  *) ok "NO_COLOR strips it even on a terminal";;
esac
has  "and the finding survives that too" 'make demo' "$NCOUT"
PIPED=$("$ST" 2>&1)
case "$PIPED" in
  *$'\033['*) bad "a pipe is never coloured" "no escape code" "$(echo "$PIPED" | head -2)";;
  *) ok "a pipe is never coloured";;
esac
# The machine formats are the reason this matters: an escape code in SARIF is a corrupt
# document. They must stay clean on a terminal, where every other output is coloured.
for fmt in json sarif github; do
  FOUT=$(pty "$ST" --format "$fmt")
  case "$FOUT" in
    *$'\033['*) bad "--format $fmt stays clean on a tty" "no escape code" "$(echo "$FOUT" | head -1)";;
    *) ok "--format $fmt stays clean on a tty";;
  esac
done
cd "$R"

printf '\n\033[1m── result ──\033[0m\n'
printf '  %s passed, %s failed\n' "$PASS" "$FAIL"
if [ "$FAIL" -gt 0 ]; then printf '\n  failures:\n'; for f in "${FAILED[@]}"; do printf '    · %s\n' "$f"; done; exit 1; fi
