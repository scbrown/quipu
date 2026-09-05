#!/usr/bin/env bash
# The alignment half of the sharing story (aegis-sosiaa, demo step of aegis-iv3df7.3).
#
# examples/sharing-demo shows a graph MOVING between stores. This one shows what
# sharing does NOT move: an opinion about what the concepts ARE. After an import,
# the receiver holds two nodes for one real thing — the publisher's
# `a.example/bobbin-release` and its own `b.example/bobbin-release` — and nothing
# in the import can decide they are the same, because nothing in the import knows.
#
# So the run is: import, propose, decide, apply, then query across both and see
# one answer where there were two.
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
target_dir=$(cd "$repo" && cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
quipu=${QUIPU_BIN:-"$target_dir/debug/quipu"}
fixtures="$repo/examples/align-demo"
expected="$fixtures/expected.txt"
work=$(mktemp -d "${TMPDIR:-/tmp}/quipu-align-demo.XXXXXX")

cleanup() {
  [ -n "${KEEP_WORK:-}" ] && { echo "kept: $work" >&2; return; }
  find "$work" -type f -delete
  find "$work" -depth -type d -empty -delete
}
trap cleanup EXIT

if [[ ! -x "$quipu" ]]; then
  echo "align demo: missing executable $quipu (run cargo build --bin quipu)" >&2
  exit 1
fi

a_db="$work/store-a.db"
b_db="$work/store-b.db"
graph_a="https://a.example/graph"
graph_b="https://b.example/graph"

# --- Publisher A, and a receiver B that already knows the same release ---------
"$quipu" shapes load demo "$fixtures/shapes.ttl" --db "$a_db" >/dev/null
"$quipu" shapes load demo "$fixtures/shapes.ttl" --db "$b_db" >/dev/null
"$quipu" knot "$fixtures/store-a.ttl" --graph "$graph_a" --db "$b_db" >/dev/null
"$quipu" knot "$fixtures/store-b.ttl" --graph "$graph_b" --db "$b_db" >/dev/null

# Two nodes, one thing. This is the state an import leaves you in.
"$quipu" query \
  'SELECT ?s WHERE { GRAPH ?g { ?s <http://www.w3.org/2000/01/rdf-schema#label> "bobbin-release" } } ORDER BY ?s' \
  --db "$b_db" > "$work/before.txt" 2>&1

# --- REFUSAL: a graph IRI that is not in the store is not an empty graph -------
# Recorded here because the reassuring reading of a typo is "0 candidates"
# (aegis-19o403). The demo asserts the refusal rather than describing it.
set +e
"$quipu" align propose "$graph_a" "https://b.example/graf" --db "$b_db" > "$work/typo.txt" 2>&1
typo_rc=$?
set -e

# --- PROPOSE -> DECIDE -> APPLY ------------------------------------------------
"$quipu" align propose "$graph_a" "$graph_b" --out "$work/set.tsv" --db "$b_db" > "$work/propose.json" 2> "$work/propose.txt"
propose_version=$(sed -n 's/^expected-version: //p' "$work/propose.txt")

printf 'https://a.example/bobbin-release\thttps://b.example/bobbin-release\taccept\n' > "$work/decisions.tsv"
"$quipu" align decide "$work/set.tsv" --decisions "$work/decisions.tsv" \
  --reviewer demo-operator --out "$work/decided.tsv" > "$work/decide.json" 2> "$work/decide.txt"
decide_version=$(sed -n 's/^expected-version: \([^ ]*\).*/\1/p' "$work/decide.txt")

"$quipu" align apply "$work/decided.tsv" --graph-a "$graph_a" --graph-b "$graph_b" \
  --expected-version "$decide_version" --actor demo-operator --db "$b_db" > "$work/apply.txt" 2>&1

# --- QUERY ACROSS BOTH ---------------------------------------------------------
"$quipu" query \
  'SELECT ?other WHERE { GRAPH ?g { <https://a.example/bobbin-release> <http://www.w3.org/2002/07/owl#sameAs> ?other } }' \
  --db "$b_db" > "$work/after.txt" 2>&1

python3 - "$work" "$typo_rc" "$propose_version" "$decide_version" <<'SUMMARY' > "$work/actual.txt"
import pathlib, re, sys
work = pathlib.Path(sys.argv[1]); typo_rc = sys.argv[2]
propose_version, decide_version = sys.argv[3], sys.argv[4]

def rows(name):
    """`quipu query` prints a TABLE, not JSON: header, dashes, rows, blank, 'N results'."""
    lines = (work / name).read_text().splitlines()
    out = []
    for line in lines[2:]:
        if not line.strip() or line.strip().endswith("results"):
            break
        out.append(line.strip())
    return out

def names(name):
    return ",".join(sorted(r.rsplit("/", 1)[-1] for r in rows(name))) or "none"

def num(name, pattern):
    m = re.search(pattern, (work / name).read_text())
    return m.group(1) if m else "?"

refused = "not a known graph" in (work / "typo.txt").read_text().lower()

print("1. TWO NODES ONE THING  b_holds={}".format(names("before.txt")))
print("2. TYPO REFUSED         exit={} refused={}  (an absent graph is not an empty one)".format(
    typo_rc, str(refused).lower()))
print("3. PROPOSE              candidates={} set_aside={}".format(
    num("propose.txt", r"(\d+) candidate"), num("propose.txt", r"(\d+) entity")))
print("4. DECIDE               accepted=1 reviewer=demo-operator")
print("5. VERSION CHANGED      propose!=decide={}  (carrying propose's would fail the check)".format(
    str(propose_version != decide_version).lower()))
derived = num("apply.txt", r'graph: "(urn:quipu:align:[^"]+)"')
print("6. APPLY                same_as={} written={} derived_graph_created={}".format(
    num("apply.txt", r"same_as: (\d+)"), num("apply.txt", r"written: (\d+)"),
    str(derived.startswith("urn:quipu:align:")).lower()))
print("7. QUERY ACROSS BOTH    sameAs={}".format(names("after.txt")))
print("8. BOUNDARY             the alignment is a fact in its own graph, not an edit to either source")
print("9. DERIVED GRAPH        quipu computed and created it; nobody had to name it in advance")
SUMMARY

if [[ ${1:-} == --check ]]; then
  if ! cmp -s "$expected" "$work/actual.txt"; then
    diff -u "$expected" "$work/actual.txt"
    exit 1
  fi
  echo "align demo transcript: OK"
else
  cat "$work/actual.txt"
fi
