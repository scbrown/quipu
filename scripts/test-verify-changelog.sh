#!/usr/bin/env bash
# test-verify-changelog.sh — prove verify-changelog.sh fails in BOTH directions.
#
# WHY THIS EXISTS: the guard shipped checking only one direction. It computed
# `missing = expected - actual` and returned OK whenever nothing was missing — so a
# CHANGELOG section holding 221 commits when git-cliff attributed 1 passed clean,
# printing the discrepancy on its own summary line before saying OK. That is the
# shape the bug actually takes here (full-history dump for an empty release: v0.3.5,
# #42/v0.3.7, #59/v0.3.17), so the guard was blind to its own reason for existing
# while looking green.
#
# A guard is not tested until a case that SHOULD fail it does. Each case below
# asserts an exit code and a substring, so "the check ran and found nothing" can
# never be confused with "the check cannot find anything".
#
# Hermetic: builds a throwaway git repo per case: it asserts nothing about quipu's
# own history, so fixing a real CHANGELOG section can never silently defuse it.
#
# Usage: scripts/test-verify-changelog.sh
# Exit:  0 = all cases behaved as specified; 1 = at least one did not.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v git-cliff >/dev/null 2>&1 || { echo "ERROR: git-cliff not installed" >&2; exit 2; }

pass=0; fail=0

# Build a scratch crate: one released commit (v1.0.0), then two releasable commits.
# Echoes "<tmpdir> <hashA> <hashB> <hashC>".
make_repo() {
  local d; d="$(mktemp -d)"
  (
    cd "$d"
    git init -q .
    git config user.email t@example.com
    git config user.name t
    mkdir -p scripts src docs
    cp "$REPO_ROOT/cliff.toml" .
    cp "$REPO_ROOT/scripts/verify-changelog.sh" scripts/
    cp "$REPO_ROOT/scripts/fix-changelog.sh" scripts/
    cp "$REPO_ROOT/scripts/filter-packaged-commits.py" scripts/
    printf '[package]\nname="probe"\nversion="1.0.0"\nedition="2021"\nexclude=["docs/"]\n' > Cargo.toml
    echo a > src/lib.rs; git add -A; git commit -qm "feat: the released thing"
    git tag v1.0.0
    echo b > src/lib.rs; git commit -qam "feat: the second thing"
    echo c > src/lib.rs; git commit -qam "fix: the third thing"
  ) >/dev/null 2>&1
  local a b c
  a="$(git -C "$d" rev-parse --short=7 HEAD~2)"
  b="$(git -C "$d" rev-parse --short=7 HEAD~1)"
  c="$(git -C "$d" rev-parse --short=7 HEAD)"
  echo "$d $a $b $c"
}

# entry <hash> -> one CHANGELOG bullet in the repo's rendered shape
entry() { echo "- Some change([$1](https://example.invalid/commit/$1))"; }

# check <name> <expected-exit> <expected-substring> <tmpdir>
check() {
  local name="$1" want_rc="$2" want_txt="$3" d="$4" out rc
  out="$(cd "$d" && ./scripts/verify-changelog.sh 2>&1)"; rc=$?
  if [[ "$rc" == "$want_rc" ]] && grep -qF -- "$want_txt" <<<"$out"; then
    echo "  PASS  $name (exit $rc)"
    pass=$((pass + 1))
  else
    echo "  FAIL  $name — wanted exit $want_rc containing '$want_txt', got exit $rc:" >&2
    sed 's/^/          /' <<<"$out" >&2
    fail=$((fail + 1))
  fi
  rm -rf "$d"
}

echo "verify-changelog.sh — both-direction guard tests"

# 1. CONTROL. The section matches git-cliff exactly. Must pass, or every release
#    blocks on a defect that is not there.
read -r d a b c <<<"$(make_repo)"
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; entry "$c"; } > "$d/CHANGELOG.md"
check "exact match passes" 0 "none missing, none extra" "$d"

# 2. UNDER-DOCUMENTED (the direction the guard already had). c is omitted.
read -r d a b c <<<"$(make_repo)"
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; } > "$d/CHANGELOG.md"
check "missing commit fails" 1 "is missing commits" "$d"

# 3. OVER-DOCUMENTED — THE REGRESSION THIS FILE EXISTS FOR. Commit a belongs to
#    v1.0.0 and is documented again under 1.1.0. The one-directional guard
#    returned 0 here.
read -r d a b c <<<"$(make_repo)"
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$a"; entry "$b"; entry "$c"; } > "$d/CHANGELOG.md"
check "extra commit fails" 1 "do not belong to this release" "$d"

# 4. THE #59 SHAPE: nothing releasable, and the section dumps all history anyway.
#    Reproduces the empty-delta full-history dump end to end.
read -r d a b c <<<"$(make_repo)"
git -C "$d" tag v1.1.0                     # everything through c is already released
{ echo "# Changelog"; echo; echo "## [1.2.0] - 2026-01-01"; echo;
  entry "$a"; entry "$b"; entry "$c"; } > "$d/CHANGELOG.md"
check "empty-delta full-history dump fails" 1 "nothing to release" "$d"

# 5. git-cliff renders a literal [0000000] for entries it cannot attribute (v0.3.11
#    has one). It is changelog noise, not a misfiled commit — it must not fail a
#    release for a defect that does not exist.
read -r d a b c <<<"$(make_repo)"
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; entry "$c"; entry "0000000"; } > "$d/CHANGELOG.md"
check "unresolvable placeholder hash is ignored" 0 "none missing, none extra" "$d"

# 6. A substantive commit that ALSO edits CHANGELOG.md is still release CONTENT and
#    must still be documented. The exemption used to be "touches CHANGELOG.md",
#    which let exactly this commit stop being required — seen live on #60.
read -r d a b c <<<"$(make_repo)"
(
  cd "$d"
  echo x >> src/lib.rs
  printf '# Changelog\n\nnote\n' > CHANGELOG.md
  git add -A && git commit -qm "fix: real work that also edits the changelog"
) >/dev/null 2>&1
e="$(git -C "$d" rev-parse --short=7 HEAD)"
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; entry "$c"; } > "$d/CHANGELOG.md"     # e deliberately omitted
check "substantive commit touching CHANGELOG.md is still required" 1 "$e" "$d"

# 7. ...but a pure release-mechanics commit (CHANGELOG/Cargo only) is still exempt,
#    or every release would fail on release-plz's own commit.
read -r d a b c <<<"$(make_repo)"
(
  cd "$d"
  printf '# Changelog\n\nnote\n' > CHANGELOG.md
  sed -i 's/version="1.0.0"/version="1.1.0"/' Cargo.toml
  git add -A && git commit -qm "chore: release v1.1.0"
) >/dev/null 2>&1
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; entry "$c"; } > "$d/CHANGELOG.md"
check "pure release-mechanics commit stays exempt" 0 "none missing, none extra" "$d"

# 8. An excluded-path-only commit does not change the packaged crate. It must not
#    be required by the guard when release-plz correctly omits it.
read -r d a b c <<<"$(make_repo)"
(
  cd "$d"
  echo note > docs/ci.md
  git add -A && git commit -qm "docs(ci): excluded documentation"
) >/dev/null 2>&1
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; entry "$c"; } > "$d/CHANGELOG.md"
check "excluded-only commit is not required" 0 "none missing, none extra" "$d"

# 9. The inverse is equally important: documenting that excluded-only commit is
#    over-documentation and must fail, preserving the guard's two-way contract.
read -r d a b c <<<"$(make_repo)"
(
  cd "$d"
  echo note > docs/ci.md
  git add -A && git commit -qm "docs(ci): excluded documentation"
) >/dev/null 2>&1
e="$(git -C "$d" rev-parse --short=7 HEAD)"
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; entry "$c"; entry "$e"; } > "$d/CHANGELOG.md"
check "documented excluded-only commit fails as extra" 1 "do not belong to this release" "$d"

# 10. The generator shares the same boundary: it removes an excluded commit from
#     a generated section while retaining both packaged commits.
read -r d a b c <<<"$(make_repo)"
(
  cd "$d"
  echo note > docs/ci.md
  git add -A && git commit -qm "docs(ci): excluded documentation"
) >/dev/null 2>&1
e="$(git -C "$d" rev-parse --short=7 HEAD)"
{ echo "# Changelog"; echo; echo "## [1.1.0] - 2026-01-01"; echo;
  entry "$b"; entry "$c"; entry "$e"; echo; echo "## [1.0.0] - 2025-12-01"; echo;
  entry "$a"; } > "$d/CHANGELOG.md"
fix_out="$(cd "$d" && ./scripts/fix-changelog.sh 2>&1)"; fix_rc=$?
if [[ "$fix_rc" -eq 0 ]] \
   && grep -qF "[$b]" "$d/CHANGELOG.md" \
   && grep -qF "[$c]" "$d/CHANGELOG.md" \
   && ! grep -qF "[$e]" "$d/CHANGELOG.md"; then
  echo "  PASS  fixer removes excluded-only commit (exit $fix_rc)"
  pass=$((pass + 1))
else
  echo "  FAIL  fixer did not preserve packaged hashes while dropping $e:" >&2
  sed 's/^/          /' <<<"$fix_out" >&2
  fail=$((fail + 1))
fi
rm -rf "$d"

echo
echo "  ${pass} passed, ${fail} failed"
[[ "$fail" -eq 0 ]]
