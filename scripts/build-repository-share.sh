#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 5 ]; then
  echo "usage: $0 <quipu-bin> <bobbin-bin> <source-repo> <output-dir> <repository-sha>" >&2
  exit 2
fi

QUIPU_BIN=$(readlink -f "$1")
BOBBIN_BIN=$(readlink -f "$2")
SOURCE=$(readlink -f "$3")
OUTPUT=$4
REPOSITORY_SHA=$5
QUERY="$SOURCE/queries/repository-share-quipu.rq"

test -x "$QUIPU_BIN"
test -x "$BOBBIN_BIN"
test -f "$QUERY"
test ! -e "$OUTPUT" || { echo "output already exists: $OUTPUT" >&2; exit 2; }

mkdir -p "$SOURCE/.bobbin"
python3 - "$SOURCE/.bobbin/config.toml" <<'PY'
from pathlib import Path
import sys

Path(sys.argv[1]).write_text('''quipu_push_chunks = true

[index]
include = ["**/*.rs", "**/*.md", "**/*.toml", "**/*.yml", "**/*.yaml", "**/*.py", "**/*.sh"]
exclude = ["**/target/**", "**/.git/**", "**/book/book/**", "**/vendor/**"]
use_gitignore = true
entities = false

[embedding]
model = "all-MiniLM-L6-v2"
batch_size = 64
''')
PY

env -u BOBBIN_SERVER -u BOBBIN_QUIPU_REMOTE \
  "$BOBBIN_BIN" index "$SOURCE" --source "$SOURCE" --repo quipu \
  --force --skip-calibrate --json

DB="$SOURCE/.bobbin/quipu/quipu.db"
test -s "$DB"

for shape in "$SOURCE"/shapes/*.ttl; do
  name=$(basename "$shape" .ttl)
  "$QUIPU_BIN" shapes load "$name" "$shape" --db "$DB"
done

CONTEXT=$(mktemp)
trap 'rm -f "$CONTEXT"' EXIT
python3 - "$CONTEXT" "$REPOSITORY_SHA" <<'PY'
from pathlib import Path
import sys

path, sha = sys.argv[1:]
Path(path).write_text(f'''@prefix aegis: <http://aegis.gastown.local/ontology/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

aegis:repo_quipu a aegis:GitRepo ;
  rdfs:label "repo_quipu" ;
  aegis:repositorySha "{sha}" .

aegis:repo-embedded-knowledge-databases-directive-2026-09-03 a aegis:Directive ;
  rdfs:label "Repository-embedded knowledge databases" ;
  aegis:issuedBy "Stiwi" ;
  aegis:applies_to aegis:repo_quipu .

aegis:aegis-otg3xz.4 a aegis:WorkItem ;
  rdfs:label "Repository share content" ;
  aegis:touchesRepo aegis:repo_quipu .
''')
PY
"$QUIPU_BIN" knot "$CONTEXT" --db "$DB"

"$QUIPU_BIN" share --construct "$(cat "$QUERY")" --turtle \
  --output "$OUTPUT" --db "$DB"

test "$(wc -c < "$OUTPUT/shapes.ttl")" -gt 0
test "$(wc -l < "$OUTPUT/export.nt")" -gt 10000
grep -q 'src%2Fshare_transport\.rs' "$OUTPUT/export.nt"
"$QUIPU_BIN" import "$OUTPUT"

wc -c "$OUTPUT"/*
wc -l "$OUTPUT/export.nt"
