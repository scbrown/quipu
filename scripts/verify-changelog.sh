#!/usr/bin/env bash
# verify-changelog.sh — guard against release-plz's changelog commit mis-selection.
#
# WHY: release-plz's own commit selection for the CHANGELOG is
# unreliable on this repo — it has picked a wrong base (documenting only the 4
# newest commits of a 17-commit release, v0.3.6) and, separately, dumped
# full-history for an empty release (v0.3.7, and v0.3.5 before it). git-cliff with
# THIS repo's cliff.toml over `<prev-tag>..<head>` produces the correct delta every
# time. This script regenerates that correct delta and fails if the newest section
# of CHANGELOG.md is missing any commit git-cliff attributes to the release —
# mechanizing the manual "diff before merge" workaround so a bad changelog can't be
# merged unnoticed (v0.3.5's under-documented section was).
#
# Usage:
#   scripts/verify-changelog.sh                 # verify HEAD's CHANGELOG newest section
#   scripts/verify-changelog.sh --at <ref>      # verify CHANGELOG.md as of <ref> (for tests)
#   scripts/verify-changelog.sh --range A..B    # override the git-cliff range explicitly
#
# Exit: 0 = every expected commit is documented; 1 = commits missing; 2 = usage/env error.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

CLIFF_CONFIG="${CLIFF_CONFIG:-cliff.toml}"
AT_REF=""
RANGE_OVERRIDE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --at) AT_REF="$2"; shift 2 ;;
    --range) RANGE_OVERRIDE="$2"; shift 2 ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

command -v git-cliff >/dev/null 2>&1 || { echo "ERROR: git-cliff not installed" >&2; exit 2; }

# The CHANGELOG content to check (working tree, or as-of a ref for tests).
if [[ -n "$AT_REF" ]]; then
  changelog_content="$(git show "${AT_REF}:CHANGELOG.md")"
  head_ref="$AT_REF"
else
  changelog_content="$(cat CHANGELOG.md)"
  head_ref="HEAD"
fi

# Newest version section = from the first `## [x]` heading to the next one.
newest_ver="$(printf '%s\n' "$changelog_content" | grep -m1 -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
[[ -n "$newest_ver" ]] || { echo "ERROR: no versioned section found in CHANGELOG.md" >&2; exit 2; }
newest_section="$(printf '%s\n' "$changelog_content" | awk '/^## \[/{n++} n==1' )"

# Range = <prev-tag>..<head-of-release>.
#  - HEAD of range: the tag v<newest_ver> if it already exists (a released section
#    documents v<prev>..v<newest>, NOT ..HEAD — HEAD may be ahead of the release);
#    otherwise head_ref itself (a pending release PR whose tag does not exist yet).
#  - prev tag: the highest v-tag that is an ancestor of the release head and is not
#    the version being released — exactly the base git-cliff would use at tag time.
if [[ -n "$RANGE_OVERRIDE" ]]; then
  range="$RANGE_OVERRIDE"
else
  rel_head="$head_ref"
  if git rev-parse -q --verify "refs/tags/v${newest_ver}" >/dev/null 2>&1 \
     && git merge-base --is-ancestor "v${newest_ver}" "$head_ref" 2>/dev/null; then
    rel_head="v${newest_ver}"
  fi
  prev_tag="$(git tag --list 'v[0-9]*' --sort=-v:refname \
    | grep -vx "v${newest_ver}" \
    | while read -r t; do git merge-base --is-ancestor "$t" "$rel_head" 2>/dev/null && { echo "$t"; break; }; done)"
  [[ -n "$prev_tag" ]] || { echo "ERROR: no previous v-tag found as ancestor of ${rel_head}" >&2; exit 2; }
  range="${prev_tag}..${rel_head}"
fi

# Expected commits = git-cliff over the range with the repo's config (the source of
# truth). Extract the 7-char hashes it renders; drop the release commit itself.
expected="$(git-cliff --config "$CLIFF_CONFIG" "$range" 2>/dev/null \
  | grep -oE '\[[0-9a-f]{7}\]' | tr -d '[]' | sort -u)"
# Actual = hashes present in the newest CHANGELOG section.
actual="$(printf '%s\n' "$newest_section" | grep -oE '\[[0-9a-f]{7}\]' | tr -d '[]' | sort -u)"

missing="$(comm -23 <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") || true)"
# Drop release-MECHANICS commits from "missing": they are not release CONTENT and
# cannot be required to appear in the changelog. A commit that TOUCHES CHANGELOG.md
# is changelog maintenance (release-plz's own "chore: release vX", or a hand splice
# like the manual fb1cfea splice) — it documents the release, it is not documented BY it.
missing="$(printf '%s\n' "$missing" | while read -r h; do
  [[ -z "$h" ]] && continue
  git show --name-only --format= "$h" 2>/dev/null | grep -qx 'CHANGELOG.md' && continue
  echo "$h"
done)"

exp_n="$(grep -c . <<<"$expected" || true)"
act_n="$(grep -c . <<<"$actual" || true)"
mis_n="$(grep -c . <<<"$missing" || true)"

echo "changelog-verify: version ${newest_ver}, range ${range}"
echo "  git-cliff commits: ${exp_n} · documented in CHANGELOG: ${act_n} · missing: ${mis_n}"
if [[ "$mis_n" -gt 0 ]]; then
  echo "FAIL — the newest CHANGELOG section is missing commits git-cliff attributes to this release:" >&2
  while read -r h; do [[ -n "$h" ]] && echo "  - $h $(git log -1 --format='%s' "$h" 2>/dev/null)"; done <<<"$missing" >&2
  echo "" >&2
  echo "Fix: regenerate with the tool that is correct here —" >&2
  echo "  git-cliff --config ${CLIFF_CONFIG} ${range}" >&2
  echo "and splice its output into the ${newest_ver} section" >&2
  exit 1
fi
echo "OK — every git-cliff commit for ${newest_ver} is documented."
