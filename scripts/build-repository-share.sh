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
FRESH_DB=$(mktemp)
IMPORT_JSON=$(mktemp)
QUERY_JSON=$(mktemp)
trap 'rm -f "$CONTEXT" "$FRESH_DB" "$IMPORT_JSON" "$QUERY_JSON"' EXIT
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

node "$SOURCE/scripts/build-contributor-knowledge.mjs" "$SOURCE" "$CONTEXT"
"$QUIPU_BIN" knot "$CONTEXT" --db "$DB"

"$QUIPU_BIN" share --construct "$(cat "$QUERY")" --turtle \
  --output "$OUTPUT" --db "$DB"

test "$(wc -c < "$OUTPUT/shapes.ttl")" -gt 0
test "$(wc -l < "$OUTPUT/export.nt")" -gt 10000
grep -q 'src%2Fshare_transport\.rs' "$OUTPUT/export.nt"
# Bundled shapes are evidence, never authority: the receiver must explicitly
# adopt them before import can admit their vocabulary into a fresh store.
"$QUIPU_BIN" shapes load repository-share "$OUTPUT/shapes.ttl" --db "$FRESH_DB"
"$QUIPU_BIN" import "$OUTPUT" --db "$FRESH_DB" > "$IMPORT_JSON"
SHARE_ID=$(python3 - "$IMPORT_JSON" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1]))
assert data["outcome"] == "staged", data["outcome"]
assert data["promotion"]["eligible"] is True, data["promotion"]
assert data["triples"]["accepted"] > 10_000, data["triples"]
print(data["share_id"])
PY
)
"$QUIPU_BIN" import promote "$SHARE_ID" --db "$FRESH_DB"
"$QUIPU_BIN" query \
  'PREFIX aegis: <http://aegis.gastown.local/ontology/> SELECT ?module WHERE { ?symbol aegis:definedIn ?module . ?symbol aegis:filePath "src/share_transport.rs" } LIMIT 1' \
  --db "$FRESH_DB" > "$QUERY_JSON"
grep -q 'src%2Fshare_transport\.rs' "$QUERY_JSON"

node "$SOURCE/scripts/verify-contributor-knowledge.mjs" "$QUIPU_BIN" "$FRESH_DB"

wc -c "$OUTPUT"/*
wc -l "$OUTPUT/export.nt"
