#!/usr/bin/env bash
#
# Tests for the file-size ratchet (quipu-5le).
#
# The guard this replaces was green for the wrong reason for its whole life —
# it scanned `git diff --cached`, and CI runs `pre-commit run --all-files` with
# nothing staged, so it never checked anything. A guard that cannot fail is
# indistinguishable from one that passes, which is precisely why the
# replacement gets tests of its own.
#
# Each case builds a throwaway git repo, so nothing here depends on the state
# of the quipu tree.

set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-file-size.sh"
passed=0
failed=0

# Build a scratch repo containing $2 lines in src/big.rs.
setup() {
    local dir="$1" lines="$2" kind="${3:-code}"
    mkdir -p "$dir/src"
    git -C "$dir" init -q
    git -C "$dir" config user.email t@example.com
    git -C "$dir" config user.name t
    # CODE lines, not comments. The old fixture wrote '// x' — fine for a raw-line
    # gate and meaningless for a code-line one (aegis-gf3j7): a "600-line" file of
    # comments is 0 code lines, so three of the tests below silently stopped
    # testing anything the moment the metric changed. $3 optional: "comment".
    python3 -c "
import sys
kind = sys.argv[3] if len(sys.argv) > 3 else 'code'
line = '// x' if kind == 'comment' else 'pub fn f%d() {}'
n = int(sys.argv[2])
body = '\\n'.join((line % i) if '%d' in line else line for i in range(n))
open(sys.argv[1], 'w').write(body + '\\n')
" "$dir/src/big.rs" "$lines" "${3:-code}"
    git -C "$dir" add -A
    git -C "$dir" commit -qm init
}

check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo "  ok: $name"
        passed=$((passed + 1))
    else
        echo "  FAIL: $name (expected exit $expected, got $actual)"
        failed=$((failed + 1))
    fi
}

run() {
    local dir="$1"; shift
    ( cd "$dir" && "$SCRIPT" "$@" >/dev/null 2>&1 ) && echo 0 || echo $?
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "file-size ratchet:"

# A file under the limit needs no baseline and passes.
setup "$tmp/small" 100
check "under the limit passes" 0 "$(run "$tmp/small")"

# An unbaselined file over the limit fails. This is the case the OLD script
# missed entirely in CI.
setup "$tmp/new" 600
check "new violation fails" 1 "$(run "$tmp/new")"

# Once grandfathered at its size, the same file passes.
( cd "$tmp/new" && "$SCRIPT" --update-baseline >/dev/null 2>&1 )
check "grandfathered violation passes" 0 "$(run "$tmp/new")"

# Growing past the baseline fails, even by one line.
printf 'pub fn grow() {}\n' >> "$tmp/new/src/big.rs"   # CODE, not a comment
check "growth past baseline fails" 1 "$(run "$tmp/new")"

# Shrinking is always allowed.
python3 -c "open('$tmp/new/src/big.rs','w').write(''.join('pub fn s%d() {}\n' % i for i in range(550)))"
check "shrinking passes" 0 "$(run "$tmp/new")"

# --update-baseline must never loosen: re-running after growth keeps the
# tighter number, so a grown file cannot be laundered into the baseline.
setup "$tmp/tight" 600
( cd "$tmp/tight" && "$SCRIPT" --update-baseline >/dev/null 2>&1 )
python3 -c "open('$tmp/tight/src/big.rs','w').write(''.join('pub fn t%d() {}\n' % i for i in range(800)))"
( cd "$tmp/tight" && "$SCRIPT" --update-baseline >/dev/null 2>&1 )
recorded=$(awk '/big.rs/ {print $2}' "$tmp/tight/.file-size-baseline")
if [ "$recorded" = "600" ]; then
    echo "  ok: --update-baseline refuses to loosen"
    passed=$((passed + 1))
else
    echo "  FAIL: --update-baseline loosened 600 -> $recorded"
    failed=$((failed + 1))
fi

# Test files are exempt regardless of size.
setup "$tmp/tests" 100
python3 -c "open('$tmp/tests/src/big_tests.rs','w').write('// x\n'*900)"
git -C "$tmp/tests" add -A && git -C "$tmp/tests" commit -qm t
check "tests.rs is exempt" 0 "$(run "$tmp/tests")"

echo

# ── aegis-gf3j7: the metric is CODE lines. ian's required controls. ──
#
# The metric change is the kind that can silently make a gate vacuous: baselines
# recorded in RAW lines are all larger than any CODE count, so until they were
# regenerated nothing could fail. Each direction is pinned here so it stays true.

# Comment-only growth must NOT fail — that is the entire point of the change.
setup "$tmp/cmt" 450
printf '// a comment\n// another\n// and a third\n' >> "$tmp/cmt/src/big.rs"
check "comment-only growth passes" 0 "$(run "$tmp/cmt")"

# Huge in RAW lines, small in CODE lines, must pass. This is src/config.rs from
# the ruling: 227 code, 207 comment, refused by the old metric for being well
# documented.
setup "$tmp/documented" 700 comment
printf 'pub fn only_code() {}\n' >> "$tmp/documented/src/big.rs"
check "700 comment lines + 1 code line passes" 0 "$(run "$tmp/documented")"

# ...and one code line past the limit must still fail.
setup "$tmp/codebig" 501
check "501 code lines fails" 1 "$(run "$tmp/codebig")"

if [ "$failed" -gt 0 ]; then
    echo "$failed test(s) failed, $passed passed"
    exit 1
fi
