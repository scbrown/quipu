#!/usr/bin/env bash
# Release publication precedes asset upload. A young release gets a bounded
# grace period; old/missing assets and staged-but-unpublished bytes still fail.
set -euo pipefail
SOURCE=${1:-docs/book/src/explore}
OUTPUT=${2:-docs/book/book/explore}
for f in index.html explorer.js worker.js constellation.js constellation.css; do
  test -s "$OUTPUT/$f" || { echo "missing from book output: explore/$f"; exit 1; }
done
pending=false
if [ -n "${EXPLORER_RELEASE_PUBLISHED_AT:-}" ]; then
  published=$(date -u -d "$EXPLORER_RELEASE_PUBLISHED_AT" +%s)
  age=$(( $(date -u +%s) - published ))
  if [ "$age" -ge 0 ] && [ "$age" -lt 1800 ]; then pending=true; fi
fi
for f in pkg/quipu_wasm_explorer.js pkg/quipu_wasm_explorer_bg.wasm repository.qpack.tar.gz; do
  if test -s "$SOURCE/$f"; then
    test -s "$OUTPUT/$f" || { echo "staged but not published: explore/$f"; exit 1; }
    cmp -s "$SOURCE/$f" "$OUTPUT/$f" || { echo "published bytes differ: explore/$f"; exit 1; }
  elif "$pending"; then
    echo "::warning::release is under 30 minutes old; explorer asset still uploading: $f"
  elif [ -z "${EXPLORER_RELEASE_PUBLISHED_AT:-}" ] && [ ! -d "$SOURCE/pkg" ]; then
    echo "::warning::no release bundle staged: $f"
  else
    echo "missing release asset beyond publication grace: explore/$f"; exit 1
  fi
done
