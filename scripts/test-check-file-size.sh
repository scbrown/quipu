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
    local dir="$1" lines="$2"
    mkdir -p "$dir/src"
    git -C "$dir" init -q
    git -C "$dir" config user.email t@example.com
    git -C "$dir" config user.name t
    python3 -c "import sys; open(sys.argv[1],'w').write('// x\n'*int(sys.argv[2]))" \
        "$dir/src/big.rs" "$lines"
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
printf '// grow\n' >> "$tmp/new/src/big.rs"
check "growth past baseline fails" 1 "$(run "$tmp/new")"

# Shrinking is always allowed.
python3 -c "open('$tmp/new/src/big.rs','w').write('// x\n'*550)"
check "shrinking passes" 0 "$(run "$tmp/new")"

# --update-baseline must never loosen: re-running after growth keeps the
# tighter number, so a grown file cannot be laundered into the baseline.
setup "$tmp/tight" 600
( cd "$tmp/tight" && "$SCRIPT" --update-baseline >/dev/null 2>&1 )
python3 -c "open('$tmp/tight/src/big.rs','w').write('// x\n'*800)"
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
if [ "$failed" -gt 0 ]; then
    echo "$failed test(s) failed, $passed passed"
    exit 1
fi
echo "all $passed test(s) passed"
