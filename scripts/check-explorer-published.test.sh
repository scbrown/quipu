#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CHECK="$ROOT/scripts/check-explorer-published.sh"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/src/pkg" "$T/out/pkg"
for f in index.html explorer.js worker.js constellation.js constellation.css; do echo source > "$T/out/$f"; done
for f in quipu_wasm_explorer.js quipu_wasm_explorer_bg.wasm; do
  echo wasm > "$T/src/pkg/$f"; cp "$T/src/pkg/$f" "$T/out/pkg/$f"
done
expect_fail() {
  if "$@" > "$T/error" 2>&1; then echo "expected failure" >&2; exit 1; fi
}
export EXPLORER_RELEASE_PUBLISHED_AT
EXPLORER_RELEASE_PUBLISHED_AT=$(date -u +%FT%TZ)
"$CHECK" "$T/src" "$T/out"
# Same missing pack, older than the grace: cannot silently pass.
EXPLORER_RELEASE_PUBLISHED_AT=2020-01-01T00:00:00Z
expect_fail "$CHECK" "$T/src" "$T/out"
grep -q 'beyond publication grace' "$T/error"
EXPLORER_RELEASE_PUBLISHED_AT=$(date -u +%FT%TZ)
echo pack > "$T/src/repository.qpack.tar.gz"
expect_fail "$CHECK" "$T/src" "$T/out"
grep -q 'staged but not published' "$T/error"
cp "$T/src/repository.qpack.tar.gz" "$T/out/"
"$CHECK" "$T/src" "$T/out"
echo changed > "$T/out/repository.qpack.tar.gz"
expect_fail "$CHECK" "$T/src" "$T/out"
grep -q 'published bytes differ' "$T/error"
rm "$T/out/constellation.js"
expect_fail "$CHECK" "$T/src" "$T/out"
grep -q 'missing from book output' "$T/error"
echo '6 explorer publication scenarios passed'
