#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
target_dir=$(cd "$repo" && cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
quipu=${QUIPU_BIN:-"$target_dir/debug/quipu"}
fixtures="$repo/examples/sharing-demo"
expected="$fixtures/expected.txt"
work=$(mktemp -d "${TMPDIR:-/tmp}/quipu-sharing-demo.XXXXXX")

cleanup() {
  find "$work" -type f -delete
  find "$work" -depth -type d -empty -delete
}
trap cleanup EXIT

if [[ ! -x "$quipu" ]]; then
  echo "sharing demo: missing executable $quipu (run cargo build --bin quipu)" >&2
  exit 1
fi

mkdir -p "$work/shares"
a_db="$work/store-a.db"
b_db="$work/store-b.db"

"$quipu" shapes load demo "$fixtures/shapes.ttl" --db "$a_db" >/dev/null
"$quipu" knot "$fixtures/store-a-base.ttl" --db "$a_db" >/dev/null
"$quipu" share --output "$work/shares/base" --shapes demo --db "$a_db" >/dev/null

"$quipu" import "$work/shares/base" --actor receiver --db "$b_db" > "$work/quarantined.json"
"$quipu" shapes load demo "$work/shares/base/shapes.ttl" --db "$b_db" >/dev/null
"$quipu" import "$work/shares/base" --actor receiver --db "$b_db" > "$work/staged.json"
share_id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["share_id"])' "$work/staged.json")
"$quipu" import promote "$share_id" --actor receiver --db "$b_db" > "$work/promoted.json"

"$quipu" query 'SELECT ?s WHERE { ?s <https://demo.example/name> ?name } ORDER BY ?s' --db "$b_db" > "$work/composed.txt"
"$quipu" knot "$fixtures/store-b-diverged.ttl" --db "$b_db" >/dev/null
"$quipu" knot "$fixtures/store-a-diverged.ttl" --db "$a_db" >/dev/null
"$quipu" share --output "$work/shares/next" --shapes demo --parent-share "$share_id" --db "$a_db" >/dev/null
"$quipu" status "$work/shares/next" --db "$b_db" > "$work/status.json"
"$quipu" merge "$work/shares/next" --actor receiver --db "$b_db" > "$work/merged.json"
"$quipu" query 'SELECT ?s WHERE { ?s <https://demo.example/name> ?name } ORDER BY ?s' --db "$b_db" > "$work/converged.txt"

python3 - "$work" <<'PY' > "$work/actual.txt"
import json
import pathlib
import sys

work = pathlib.Path(sys.argv[1])
load = lambda name: json.loads((work / name).read_text())
def names(name):
    values = []
    for line in (work / name).read_text().splitlines():
        if "https://demo.example/" not in line:
            continue
        values.append(line.strip().strip('"<>').rsplit("/", 1)[-1])
    return ",".join(sorted(values))

quarantined = load("quarantined.json")
staged = load("staged.json")
promoted = load("promoted.json")
status = load("status.json")
merged = load("merged.json")
manifest = load("shares/base/manifest.json")
facts = sum(bool(line.strip()) for line in (work / "shares/base/export.nt").read_text().splitlines())
shapes = (work / "shares/base/shapes.ttl").read_text().count(" sh:NodeShape")

print("1. SHARE A             scope={} facts={} shapes={}".format(
    manifest["scope"]["kind"], facts, shapes))
print("2. IMPORT B            outcome={} admitted={} quarantined={} blocker={}".format(
    quarantined["outcome"], quarantined["triples"]["accepted"],
    quarantined["triples"]["quarantined"], quarantined["promotion"]["blockers"][0]))
print("3. ADOPT + REIMPORT    outcome={} admitted={} quarantined={} promotion_eligible={}".format(
    staged["outcome"], staged["triples"]["accepted"], staged["triples"]["quarantined"],
    str(staged["promotion"]["eligible"]).lower()))
print("4. PROMOTE B           outcome={} triples={}".format(promoted["outcome"], promoted["triples"]))
print("5. COMPOSE B           root_names={}".format(names("composed.txt")))
print("6. DIVERGE             A_adds=a-only B_adds=b-only")
print("7. STATUS              diverged={} ours_added={} theirs_added={} conflicts={}".format(
    str(status["diverged"]).lower(), status["ours_added"], status["theirs_added"], len(status["conflicts"])))
print("8. RECONNECT           outcome={} asserted={} retracted={} provenance_parents={}".format(
    merged["outcome"], merged["asserted"], merged["retracted"], len(merged["provenance_parents"])))
print("9. CONVERGED B         root_names={}".format(names("converged.txt")))
print("10. BOUNDARY           provider federation unions labelled query results; it does not merge store histories")
PY

if [[ ${1:-} == --check ]]; then
  if ! cmp -s "$expected" "$work/actual.txt"; then
    diff -u "$expected" "$work/actual.txt"
    exit 1
  fi
  echo "sharing demo transcript: OK"
else
  cat "$work/actual.txt"
fi
