#!/usr/bin/env bash
# Produce the demo's canonical text share with the native Quipu CLI.
# This is an authoring step, never part of the Pages build.
# Usage: export-datalinks.sh <graph.ttl> <shapes.ttl> <identifier-policy.ttl> [out-dir]
set -euo pipefail
if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
    echo "usage: $0 <graph.ttl> <shapes.ttl> <identifier-policy.ttl> [out-dir]" >&2
    exit 2
fi
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${4:-$ROOT/docs/book/src/datalinks/qpack}
QUIPU_BIN=${QUIPU_BIN:-quipu}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
DB="$WORK/demo.db"
# Policy stays in its own graph and is not part of the public demo payload.
"$QUIPU_BIN" shapes load identifier-policy "$ROOT/examples/sharing-demo/policy-shapes.ttl" --db "$DB"
"$QUIPU_BIN" shapes load datalinks "$2" --db "$DB"
"$QUIPU_BIN" knot "$3" --graph urn:datalinks:identifier-policy --db "$DB"
"$QUIPU_BIN" knot "$1" --db "$DB"
"$QUIPU_BIN" share --output "$OUT" --shapes datalinks --db "$DB"
