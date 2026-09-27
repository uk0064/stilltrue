#!/usr/bin/env bash
# Run stilltrue against real third-party repositories.
#
# Every precision number in the README and CLAUDE.md comes from this. It is not a test:
# it clones ~1GB and takes a few minutes, and the repositories move under it, so the
# counts drift. It is here so the claims can be checked rather than believed.
#
#   tests/corpus/run.sh            # clone (once) and lint, default Tier A
#   tests/corpus/run.sh --strict   # also Tier B
#
# Clones are full: Tier A needs history, so --depth is not an option.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
work="${STILLTRUE_CORPUS:-${TMPDIR:-/tmp}/stilltrue-corpus}"
binary="$root/target/release/stilltrue"
[ -x "$binary" ] || { echo "build it first: cargo build --release" >&2; exit 2; }

mkdir -p "$work"
while read -r repo; do
  [ -n "$repo" ] || continue
  name="${repo#*/}"
  [ -d "$work/$name" ] && continue
  echo "cloning $repo" >&2
  git clone --quiet "https://github.com/$repo.git" "$work/$name" || echo "FAILED $repo" >&2
done < "$(dirname "$0")/repositories.txt"

total=0
quiet=0
degraded=0
failed=0
err="$work/.stderr"
for dir in "$work"/*/; do
  [ -d "$dir/.git" ] || continue
  name="$(basename "$dir")"
  # Both halves of this matter. Discarding stderr hides the warnings that say a scan
  # was degraded, and discarding the status hides a crash — and a repository that
  # crashed prints no findings, so it used to be counted as a quiet one. A silence
  # that cannot be told apart from a failure is not evidence of precision.
  set +e
  out="$(cd "$dir" && "$binary" "$@" 2>"$err")"
  status=$?
  set -e
  count="$(printf '%s\n' "$out" | grep -cE '  (rot|lie)  ' || true)"
  total=$((total + count))

  note=""
  if [ "$status" -ge 2 ]; then
    note="  FAILED exit $status"
    failed=$((failed + 1))
  elif [ -s "$err" ]; then
    # stilltrue warns and continues: an unreadable document, a missing manifest, a
    # shallow clone. The run still answered, but not about everything.
    note="  degraded: $(head -1 "$err" | cut -c1-60)"
    degraded=$((degraded + 1))
  elif [ "$count" -eq 0 ]; then
    quiet=$((quiet + 1))
  fi

  printf '%-12s %3s%s\n' "$name" "$count" "$note"
  [ "$count" -gt 0 ] && printf '%s\n' "$out" | grep -E '  (rot|lie)  ' | sed 's/^/    /'
done
rm -f "$err"

echo
echo "$total findings; $quiet repositories clean and quiet"
echo "$degraded degraded (answered, but not about everything); $failed failed"
echo "A quiet run is only evidence of precision when it is neither of the last two."
