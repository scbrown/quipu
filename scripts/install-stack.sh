#!/usr/bin/env bash
# install-stack.sh — install caboodle, drive the two-phase stack install,
# and load knowledge packs into the target store.
#
# Two-phase on purpose: `caboodle plan` writes a reviewable plan file and
# NOTHING installs until that plan has been reviewed (or --yes given
# explicitly). That is caboodle's own doctrine — nothing converges on
# conversation strength alone — and this wrapper preserves it rather than
# papering over it.
#
# Usage:
#   scripts/install-stack.sh [--profile kg] [--yes] [--dry-run]
#                            [--qpack PATH]... [--db PATH] [--plan PATH]
#
#   --profile P   caboodle profile to plan (default: kg)
#   --yes         proceed past the plan to apply + verify + qpack load
#   --qpack PATH  a .qpack.db knowledge pack to verify and unpack
#                 into the target store (repeatable)
#   --db PATH     target quipu store for --qpack (default: quipu's own
#                 default store path)
#   --plan PATH   where caboodle writes/reads the plan
#                 (default: caboodle-plan.toml)
#   --dry-run     print every command that would run; execute nothing
#
# Syntax stays POSIX-parseable (sh -n clean): no arrays — the repeatable
# --qpack list is a newline-separated string walked by `read`.

set -euo pipefail

PROFILE="kg"
PLAN="caboodle-plan.toml"
DB=""
YES=0
DRY_RUN=0
QPACKS=""        # newline-separated; POSIX sh has no arrays
QPACK_COUNT=0

usage() {
    sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --profile) PROFILE="${2:?--profile needs a value}"; shift 2 ;;
        --plan)    PLAN="${2:?--plan needs a value}"; shift 2 ;;
        --db)      DB="${2:?--db needs a value}"; shift 2 ;;
        --qpack)
            QPACKS="${QPACKS}${2:?--qpack needs a path}
"
            QPACK_COUNT=$((QPACK_COUNT + 1))
            shift 2 ;;
        --yes)     YES=1; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "install-stack: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

# Print, then run — or with --dry-run, only print. Every side effect in
# this script goes through here so dry-run cannot drift from the real path.
run() {
    printf '+ %s\n' "$*"
    if [ "$DRY_RUN" -eq 0 ]; then
        "$@"
    fi
}

# ---------------------------------------------------------------- caboodle

if command -v caboodle >/dev/null 2>&1; then
    echo "install-stack: caboodle already installed: $(caboodle --version 2>/dev/null || echo '(version unreadable)')"
else
    echo "install-stack: caboodle not found; installing"
    run cargo install --git https://github.com/scbrown/caboodle --locked
    # Read the version back — a zero exit from cargo is not proof the
    # binary is on PATH and runnable (stack culture: verify by read-back,
    # never trust the installer's exit code alone).
    if [ "$DRY_RUN" -eq 0 ]; then
        if ! command -v caboodle >/dev/null 2>&1; then
            echo "install-stack: cargo install exited 0 but 'caboodle' is not on PATH" >&2
            echo "install-stack: is ~/.cargo/bin on PATH?" >&2
            exit 1
        fi
        VERSION="$(caboodle --version)" || {
            echo "install-stack: installed caboodle failed to report its version" >&2
            exit 1
        }
        echo "install-stack: installed $VERSION"
    else
        printf '+ %s\n' "caboodle --version   # read-back check"
    fi
fi

# ------------------------------------------------------- plan (phase one)

run caboodle plan --profile "$PROFILE" --output "$PLAN"

if [ "$YES" -eq 0 ]; then
    echo ""
    echo "install-stack: plan written to: $PLAN"
    echo "install-stack: nothing has been installed. Review the plan, then re-run"
    echo "install-stack: with --yes to apply, verify, and load packs:"
    echo ""
    QPACK_ARGS=""
    while IFS= read -r pack; do
        [ -n "$pack" ] || continue
        QPACK_ARGS="$QPACK_ARGS --qpack $pack"
    done <<EOF
$QPACKS
EOF
    echo "  $0 --profile $PROFILE --plan $PLAN --yes$QPACK_ARGS"
    exit 0
fi

# ------------------------------------------- apply + verify (phase two)

run caboodle apply --plan "$PLAN"
run caboodle verify --plan "$PLAN"

# ------------------------------------------------------------ qpack load

if [ "$QPACK_COUNT" -gt 0 ]; then
    if [ "$DRY_RUN" -eq 0 ] && ! command -v quipu >/dev/null 2>&1; then
        echo "install-stack: 'quipu' is not on PATH after apply+verify; cannot load packs" >&2
        exit 1
    fi
    # A here-doc, not a pipe: `exit 1` inside a piped while runs in a
    # subshell and would NOT stop the script — a failed verify must.
    while IFS= read -r pack; do
        [ -n "$pack" ] || continue
        if [ "$DRY_RUN" -eq 0 ] && [ ! -f "$pack" ]; then
            echo "install-stack: qpack not found: $pack" >&2
            exit 1
        fi
        # Content-hash verification FIRST, and a failure is a refusal:
        # unpacking an artifact whose hash does not match its manifest
        # would install silently corrupted knowledge.
        if ! run quipu pack --verify "$pack"; then
            echo "install-stack: pack verification FAILED for $pack — refusing to unpack" >&2
            exit 1
        fi
        if [ -n "$DB" ]; then
            run quipu unpack "$pack" --db "$DB"
        else
            run quipu unpack "$pack"
        fi
    done <<EOF
$QPACKS
EOF
fi

echo "install-stack: done (profile=$PROFILE, plan=$PLAN, qpacks=$QPACK_COUNT)"
