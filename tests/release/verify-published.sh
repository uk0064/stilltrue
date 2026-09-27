#!/usr/bin/env bash
# Verify a published release the way a user meets it: download the archive for this
# platform, check the checksum published beside it, and run the product suite and the
# README demonstration against the binary that came out of it.
#
#   tests/release/verify-published.sh 0.1.0
#
# Exits 0 only when every step passed. A release that is not there is "unverified", and
# says so — never a pass. STILLTRUE_RELEASE_BASE overrides where archives are fetched
# from, which is how this script's own failure paths are tested without a network.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
VERSION="${1:-}"
[ -n "$VERSION" ] || { echo "usage: $0 VERSION" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64) target=aarch64-unknown-linux-gnu ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Darwin-arm64)  target=aarch64-apple-darwin ;;
  *) echo "unverified: $(uname -s)-$(uname -m) is not an advertised platform" >&2; exit 1 ;;
esac

base="${STILLTRUE_RELEASE_BASE:-https://github.com/uk0064/stilltrue/releases/download}"
url="$base/v$VERSION/stilltrue-$target.tar.gz"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
fetch() { curl --proto '=https,file' --tlsv1.2 --retry 2 -sSfL -o "$1" "$2"; }

unverified() { echo "unverified: v$VERSION on $target — $1" >&2; exit 1; }

fetch "$WORK/archive.tar.gz" "$url" || unverified "no archive at $url"
fetch "$WORK/archive.tar.gz.sha256" "$url.sha256" || unverified "no checksum at $url.sha256"
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$WORK/archive.tar.gz" | awk '{print $1}')
else
  actual=$(shasum -a 256 "$WORK/archive.tar.gz" | awk '{print $1}')
fi
expected=$(awk '{print $1}' < "$WORK/archive.tar.gz.sha256")
[ "$actual" = "$expected" ] || unverified "checksum mismatch: expected $expected, got $actual"

mkdir -p "$WORK/bin"
tar xz --strip-components=1 -C "$WORK/bin" -f "$WORK/archive.tar.gz" \
  || unverified "the archive did not unpack"
binary="$WORK/bin/stilltrue"
[ -x "$binary" ] || unverified "the archive holds no binary"
reported="$("$binary" --version 2>/dev/null | awk '{print $2}')"
[ "$reported" = "$VERSION" ] || unverified "the binary says it is ${reported:-nothing}, not $VERSION"

# The demonstration: the README's example, then a correction and a clean, complete rerun.
repo="$WORK/demo"
bash "$ROOT/tests/fixtures/readme-demo/build.sh" "$repo" >/dev/null 2>&1 \
  || unverified "could not build the demonstration repository"
out="$(cd "$repo" && "$binary" 2>&1)"; code=$?
[ "$code" = 1 ] || unverified "the demonstration exited $code, not 1"
case "$out" in *'command `make demo` has no target in Makefile'*) ;; *) unverified "the demonstration printed no finding";; esac
sed 's/`make demo`/`make seed`/' "$repo/CLAUDE.md" > "$repo/CLAUDE.md.new" && mv "$repo/CLAUDE.md.new" "$repo/CLAUDE.md"
(cd "$repo" && "$binary" --report-file "$WORK/report.json" >/dev/null 2>&1) \
  || unverified "the corrected demonstration did not run clean"
grep -q '"status": "complete"' "$WORK/report.json" \
  || unverified "the corrected demonstration was not a complete run"

"$ROOT/tests/product/run.sh" "$binary" || unverified "the product suite failed against the published binary"

echo "verified: v$VERSION on $target"
echo "record it: echo $VERSION >> tests/release/verified.txt (once every advertised platform passes)"
