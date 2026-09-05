#!/usr/bin/env bash
# resolve-release-tag.sh — which version is this push publishing?
#
# Prints `<tag>\t<version>` on stdout, or refuses.
#
# ── WHY THIS IS NOT `git tag --points-at HEAD` ──────────────────────────────────
#
# It was, and it refused EVERY REAL RELEASE (aegis-fxpkt2 / aegis-xx6xu). The old
# comment stated the assumption in as many words — "release-plz tags the very commit
# being built, so the two agree by construction" — and that holds only when the
# release PR is SQUASHED. Merged with a MERGE COMMIT, release-plz's tag sits on the
# PR's own commit and main's HEAD is the merge commit one above it, so nothing points
# at HEAD and the guard refuses a perfectly good release.
#
# Measured on both refusals:
#
#   quipu-ai-v0.3.36 -> 94edf1eb, merge 96473a88   trees IDENTICAL 00a51f5d
#   quipu-ai-v0.3.37 -> f2304834, merge 4f3b3f8d   trees IDENTICAL 863e0899
#
# ── THE TEST THAT REPLACES IT: ANCESTRY **AND** TREE ────────────────────────────
#
# Two conditions, and dropping either one loses the property the guard exists for:
#
#   ANCESTRY  the tag must be reachable from HEAD (`git tag --merged HEAD`). A tag on
#             some other branch names code this push is not publishing.
#   TREE      the tag's tree must EQUAL HEAD's tree. The merge commit may sit above
#             the tag, but it must introduce NOTHING. This is what makes "the tag
#             names the code we are about to publish" true rather than approximately
#             true, and it is strictly stronger than points-at: a later commit that
#             changed anything is refused even though the tag is still an ancestor.
#
# THE FIX IS NOT "SWITCH TO SQUASH MERGES." That would make the old check pass by
# constraining how humans merge, and it would break again the first time somebody
# merged normally — silently, and only at release time.
#
# ── AND IT MUST NOT BE INVISIBLE UNTIL A RELEASE ────────────────────────────────
#
# The reason this survived two releases is that the publish job is SKIPPED on ordinary
# pushes, so it emits nothing until a release — and its first signal is a red Release
# workflow, which looks exactly like the benign asset-upload race (aegis-9wdzwv) that
# was triaged twice the same day. Two failures wearing one symptom. Hence `--selftest`:
# the logic is exercised on every run of the test suite, not only when it matters.

set -euo pipefail

PREFIX="${TAG_PREFIX:-quipu-ai-v}"

resolve() { # resolve <repo-dir> -> prints "<tag>\t<version>" or fails with a message
  local d="$1" tag head_tree tag_tree
  tag=$(git -C "$d" tag --merged HEAD --sort=-creatordate 2>/dev/null | grep "^${PREFIX}" | head -1 || true)
  if [ -z "$tag" ]; then
    echo "no ${PREFIX}* tag is reachable from HEAD. Refusing to publish a version this run cannot name." >&2
    return 1
  fi
  head_tree=$(git -C "$d" rev-parse 'HEAD^{tree}')
  tag_tree=$(git -C "$d" rev-parse "${tag}^{tree}")
  if [ "$head_tree" != "$tag_tree" ]; then
    echo "tag ${tag} is reachable from HEAD but names DIFFERENT CODE:" >&2
    echo "  tag tree  ${tag_tree}" >&2
    echo "  HEAD tree ${head_tree}" >&2
    echo "Something landed after the tag. Refusing to publish code the tag does not name." >&2
    return 1
  fi
  printf '%s\t%s\n' "$tag" "${tag##*-v}"
}

if [ "${1:-}" = "--selftest" ]; then
  fail=0; echo "resolve-release-tag selftest:"
  t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
  mk() { # mk <dir>
    git -C "$t" init -q "$1" && git -C "$t/$1" config user.email t@t && git -C "$t/$1" config user.name t
    echo base > "$t/$1/f"; git -C "$t/$1" add -A; git -C "$t/$1" commit -qm base
  }
  chk() { [ "$2" = "$3" ] && echo "  ok: $1" || { echo "  FAIL: $1 — expected '$3', got '$2'"; fail=1; }; }

  # 1. THE CASE THAT WAS BROKEN: tag one below HEAD, merge commit above it.
  mk a
  ( cd "$t/a"
    git checkout -qb rel; echo 0.3.37 > version; git add -A; git commit -qm "chore: release v0.3.37"
    git tag quipu-ai-v0.3.37
    git checkout -q master 2>/dev/null || git checkout -q main
    git merge -q --no-ff -m "Merge pull request #158" rel ) >/dev/null 2>&1
  chk "a MERGE COMMIT above the tag resolves (this is what points-at refused)" \
      "$(resolve "$t/a" 2>&1 | cut -f2)" "0.3.37"

  # 2. The squash case must still work — the fix must not trade one for the other.
  mk b
  ( cd "$t/b"; echo 0.3.38 > version; git add -A; git commit -qm rel; git tag quipu-ai-v0.3.38 ) >/dev/null 2>&1
  chk "a tag ON HEAD still resolves" "$(resolve "$t/b" 2>&1 | cut -f2)" "0.3.38"

  # 3. THE PROPERTY THE OLD CHECK HAD AND THIS MUST KEEP: code that landed after the
  #    tag is REFUSED. Ancestry alone would pass this; the tree test is what fails it.
  mk c
  ( cd "$t/c"; echo 0.3.39 > version; git add -A; git commit -qm rel; git tag quipu-ai-v0.3.39
    echo later >> f; git add -A; git commit -qm "something else landed" ) >/dev/null 2>&1
  out=$(resolve "$t/c" 2>&1 || true)
  case "$out" in
    *"names DIFFERENT CODE"*) echo "  ok: a commit AFTER the tag is refused (ancestry alone would pass)" ;;
    *) echo "  FAIL: post-tag commit was not refused: $out"; fail=1 ;;
  esac

  # 4. No tag at all.
  mk d
  out=$(resolve "$t/d" 2>&1 || true)
  case "$out" in
    *"no quipu-ai-v* tag is reachable"*) echo "  ok: no reachable tag is refused, and says so" ;;
    *) echo "  FAIL: missing tag not refused: $out"; fail=1 ;;
  esac

  # 5. A tag on ANOTHER branch must not be picked up.
  mk e
  ( cd "$t/e"
    git checkout -qb other; echo 9.9.9 > version; git add -A; git commit -qm other
    git tag quipu-ai-v9.9.9
    git checkout -q master 2>/dev/null || git checkout -q main ) >/dev/null 2>&1
  out=$(resolve "$t/e" 2>&1 || true)
  case "$out" in
    *"no quipu-ai-v* tag is reachable"*) echo "  ok: a tag on an unmerged branch is NOT reachable" ;;
    *) echo "  FAIL: picked up an unreachable tag: $out"; fail=1 ;;
  esac

  [ "$fail" = 0 ] && echo "  ALL PASS" || echo "  FAILURES ABOVE"
  exit "$fail"
fi

resolve "${1:-.}"
