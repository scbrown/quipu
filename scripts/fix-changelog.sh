#!/usr/bin/env bash
# fix-changelog.sh — regenerate the newest CHANGELOG section with git-cliff.
#
# WHY: release-plz's commit selection is unreliable on this repo and has shipped
# defective sections repeatedly (see the comment block in .github/workflows/release.yml).
# `verify-changelog.sh` catches the defect, and its failure message ends with the
# instruction "regenerate with the tool that is correct here". This is that step,
# mechanized, so the correction happens on every release PR instead of depending on
# somebody noticing a red check and running git-cliff by hand.
#
# It is deliberately the SAME range derivation as verify-changelog.sh. If the two
# ever disagree about which commits belong to a release, the verifier is the
# authority and this script is the bug.
#
# Idempotent: regenerating an already-correct section produces identical bytes, so
# the CI job that runs this converges in one pass and cannot loop.
#
# Usage:
#   scripts/fix-changelog.sh            # rewrite the newest section in place
#   scripts/fix-changelog.sh --check    # exit 1 if it WOULD change something
#
# Exit: 0 = section is correct (or was corrected); 1 = --check found a diff;
#       2 = usage/env error.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

CLIFF_CONFIG="${CLIFF_CONFIG:-cliff.toml}"
CHECK_ONLY=0
case "${1:-}" in
  --check) CHECK_ONLY=1 ;;
  -h|--help) grep '^#' "$0" | sed 's/^# \?//'; exit 0 ;;
  "") ;;
  *) echo "unknown arg: $1" >&2; exit 2 ;;
esac

command -v git-cliff >/dev/null 2>&1 || { echo "ERROR: git-cliff not installed" >&2; exit 2; }
[[ -f CHANGELOG.md ]] || { echo "ERROR: no CHANGELOG.md at $REPO_ROOT" >&2; exit 2; }

# ---------------------------------------------------------------------------
# Scratch space.
#
# Everything this script writes goes in one mktemp'd directory, and the cleanup
# refuses to delete anything that is not demonstrably that directory: the variable
# must be non-empty, must still be a directory, and must match the exact name shape
# mktemp was asked for. A bare `rm -rf "$VAR"` with an empty or surprising VAR is
# how scripts eat things they did not create; this cannot, because the pattern will
# not match. `set -u` covers the unset case before we ever get here.
# ---------------------------------------------------------------------------
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/quipu-changelog-fix.XXXXXX")"
cleanup() {
  case "${SCRATCH:-}" in
    */quipu-changelog-fix.??????)
      [[ -d "$SCRATCH" ]] && rm -rf -- "$SCRATCH"
      ;;
    *)
      echo "refusing to clean unexpected scratch path: '${SCRATCH:-}'" >&2
      ;;
  esac
}
trap cleanup EXIT

GEN_FILE="$SCRATCH/cliff.md"
NEW_FILE="$SCRATCH/CHANGELOG.new"
PACKAGED_HASHES="$SCRATCH/packaged-hashes"
export GEN_FILE PACKAGED_HASHES

# Newest version section = from the first `## [x.y.z]` heading to the next one.
newest_ver="$(grep -m1 -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md \
  | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
[[ -n "$newest_ver" ]] || { echo "ERROR: no versioned section found in CHANGELOG.md" >&2; exit 2; }

# Keep whatever date the section already carries; release-plz sets it and it is not
# ours to move. Only fall back to today when the heading has no date at all.
existing_date="$(grep -m1 -oE "^## \[${newest_ver}\] - [0-9]{4}-[0-9]{2}-[0-9]{2}" CHANGELOG.md \
  | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}' || true)"
section_date="${existing_date:-$(date -u +%Y-%m-%d)}"

# Range = <prev-tag>..<head-of-release>, derived exactly as verify-changelog.sh does.
rel_head="HEAD"
if git rev-parse -q --verify "refs/tags/v${newest_ver}" >/dev/null 2>&1 \
   && git merge-base --is-ancestor "v${newest_ver}" HEAD 2>/dev/null; then
  rel_head="v${newest_ver}"
fi
prev_tag="$(git tag --list 'v[0-9]*' --sort=-v:refname \
  | grep -vx "v${newest_ver}" \
  | while read -r t; do git merge-base --is-ancestor "$t" "$rel_head" 2>/dev/null && { echo "$t"; break; }; done)"
[[ -n "$prev_tag" ]] || { echo "ERROR: no previous v-tag found as ancestor of ${rel_head}" >&2; exit 2; }
range="${prev_tag}..${rel_head}"

echo "changelog-fix: version ${newest_ver}, range ${range}"

git-cliff --config "$CLIFF_CONFIG" "$range" > "$GEN_FILE" 2>/dev/null || true
[[ -s "$GEN_FILE" ]] || { echo "ERROR: git-cliff produced nothing for ${range}" >&2; exit 2; }
cliff_hashes="$(grep -oE '\[[0-9a-f]{7}\]' "$GEN_FILE" | tr -d '[]' | sort -u || true)"
if [[ -n "$cliff_hashes" ]]; then
  printf '%s\n' "$cliff_hashes" | scripts/filter-packaged-commits.py > "$PACKAGED_HASHES"
else
  : > "$PACKAGED_HASHES"
fi

# Splice: everything before the newest section + regenerated body + everything from
# the following section onward.
#
# Release-MECHANICS commits are dropped from the body. git-cliff attributes them to
# the release like any other conventional commit, but they document the release
# rather than being documented BY it -- and keeping them does not terminate: this
# script's own correction commit would become a new entry needing another
# correction, forever. The exemption is the same one verify-changelog.sh applies to
# its "missing" set (touches ONLY CHANGELOG.md/Cargo.toml/Cargo.lock), so the two
# agree by construction.
python3 - "$newest_ver" "$section_date" > "$NEW_FILE" <<'PYEOF'
import io, os, re, subprocess, sys

ver, section_date = sys.argv[1], sys.argv[2]
MECHANICS = {"CHANGELOG.md", "Cargo.toml", "Cargo.lock"}
PACKAGED = set(io.open(os.environ["PACKAGED_HASHES"], encoding="utf-8").read().split())

def is_mechanics(sha):
    out = subprocess.run(["git", "show", "--name-only", "--format=", sha],
                         capture_output=True, text=True).stdout
    files = {l.strip() for l in out.splitlines() if l.strip()}
    return bool(files) and files <= MECHANICS

gen = io.open(os.environ["GEN_FILE"], encoding="utf-8").read()
body = gen[gen.index("## ["):]
body = re.sub(r"^## \[[^\]]*\].*", "## [%s] - %s" % (ver, section_date),
              body, count=1, flags=re.M)

kept, dropped = [], 0
for line in body.splitlines():
    m = re.match(r"^- .*\(\[([0-9a-f]{7})\]", line)
    if m:
        sha = m.group(1)
        if sha not in PACKAGED or is_mechanics(sha):
            dropped += 1
            continue
    kept.append(line)

# Drop headings left with no entries beneath them.
out = []
for idx, line in enumerate(kept):
    if line.startswith("### "):
        nxt = next((kept[j] for j in range(idx + 1, len(kept)) if kept[j].strip()), "")
        if not nxt.startswith("- "):
            continue
    out.append(line)

body = "\n".join(out).rstrip() + "\n"
if dropped:
    sys.stderr.write("changelog-fix: dropped %d release-mechanics commit(s)\n" % dropped)

cl = io.open("CHANGELOG.md", encoding="utf-8").read()
start = cl.index("## [%s]" % ver)
rest = cl[start + 1:]
nxt = rest.find("\n## [")
end = len(cl) if nxt == -1 else start + 1 + nxt + 1
sys.stdout.write(cl[:start] + body + "\n" + cl[end:])
PYEOF

# ---------------------------------------------------------------------------
# Sanity gate before anything overwrites the real file. A regeneration that lost
# the preamble, lost the previous release, or came out suspiciously small is a bug
# in this script, and the failure mode of shipping it is a silently truncated
# changelog -- exactly the class of defect this script exists to prevent.
# ---------------------------------------------------------------------------
[[ -s "$NEW_FILE" ]] || { echo "ERROR: regenerated changelog is empty; refusing to write" >&2; exit 2; }
grep -q "^## \[${newest_ver}\]" "$NEW_FILE" \
  || { echo "ERROR: regenerated changelog lost the ${newest_ver} heading; refusing to write" >&2; exit 2; }
grep -q "^## \[${prev_tag#v}\]" "$NEW_FILE" \
  || { echo "ERROR: regenerated changelog lost the ${prev_tag} section; refusing to write" >&2; exit 2; }
old_lines="$(wc -l < CHANGELOG.md)"
new_lines="$(wc -l < "$NEW_FILE")"
if (( new_lines * 2 < old_lines )); then
  echo "ERROR: regenerated changelog is ${new_lines} lines vs ${old_lines}; refusing to write" >&2
  exit 2
fi

if cmp -s "$NEW_FILE" CHANGELOG.md; then
  echo "OK — the ${newest_ver} section already matches git-cliff; nothing to do."
  exit 0
fi

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  echo "WOULD REWRITE — the ${newest_ver} section does not match git-cliff."
  diff -u CHANGELOG.md "$NEW_FILE" | head -60 || true
  exit 1
fi

cat "$NEW_FILE" > CHANGELOG.md
echo "REWROTE — the ${newest_ver} section now matches git-cliff."
