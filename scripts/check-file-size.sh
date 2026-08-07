#!/usr/bin/env bash
#
# Source file size ratchet.
#
# ## Why this is a ratchet and not a limit
#
# The limit is 500 lines. Twenty source files already exceed it, and CI was
# green anyway: the old version iterated `git diff --cached`, and CI runs
# `pre-commit run --all-files` where nothing is staged, so the loop body never
# executed (quipu-5le). The check fired only for whoever happened to stage an
# oversized file — invisible until it blocked someone unrelated to the
# violation, which is the opposite of how a guard should behave.
#
# Simply fixing the scan turns main red on twenty files at once. So instead:
#
#   - Files in `.file-size-baseline` are grandfathered AT THEIR RECORDED SIZE.
#     They may shrink freely. Growing by even one line fails.
#   - Any file NOT in the baseline that exceeds the limit fails.
#   - So the set of violations can only shrink, and never silently.
#
# When a baselined file shrinks, the baseline is NOT rewritten automatically —
# a hook that edits tracked files during a commit is how `mermaid.min.js` used
# to turn CI red (see .pre-commit-config.yaml). Run `--update-baseline` and
# commit the result; it refuses to loosen any entry.
#
# Usage:
#   check-file-size.sh                    # scan all tracked .rs files
#   check-file-size.sh --staged           # scan staged .rs files only
#   check-file-size.sh --update-baseline  # tighten the baseline (never loosens)

set -euo pipefail

WARN_LIMIT=400
ERROR_LIMIT=500
BASELINE="$(git rev-parse --show-toplevel)/.file-size-baseline"

mode="all"
case "${1:-}" in
    --staged) mode="staged" ;;
    --update-baseline) mode="update" ;;
    "") ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
esac

# Tests are exempt: a thorough test file is not the debt this guards against.
is_exempt() {
    [[ "$1" =~ tests\.rs$ ]] || [[ "$1" =~ _test\.rs$ ]]
}

list_files() {
    if [ "$mode" = "staged" ]; then
        git diff --cached --name-only --diff-filter=ACM | grep '\.rs$' || true
    else
        git ls-files '*.rs' || true
    fi
}

# Recorded ceiling for a file, or empty when it is not grandfathered.
baseline_for() {
    [ -f "$BASELINE" ] || return 0
    awk -v f="$1" '$1 == f { print $2; exit }' "$BASELINE"
}

if [ "$mode" = "update" ]; then
    tmp=$(mktemp)
    for file in $(git ls-files '*.rs'); do
        is_exempt "$file" && continue
        [ -f "$file" ] || continue
        lines=$(wc -l < "$file")
        [ "$lines" -gt "$ERROR_LIMIT" ] || continue
        recorded=$(baseline_for "$file")
        # Never loosen: keep the smaller of recorded and actual.
        if [ -n "$recorded" ] && [ "$recorded" -lt "$lines" ]; then
            lines="$recorded"
        fi
        echo "$file $lines" >> "$tmp"
    done
    {
        echo "# Files over the ${ERROR_LIMIT}-line limit, grandfathered at these sizes."
        echo "# They may shrink; growing fails the build. Regenerate with:"
        echo "#   scripts/check-file-size.sh --update-baseline"
        echo "# Shrinking this file is the point. See quipu-bu3."
        sort "$tmp" 2>/dev/null || true
    } > "$BASELINE"
    rm -f "$tmp"
    count=$(grep -cv '^#' "$BASELINE" || true)
    echo "Baseline updated: ${count:-0} file(s) over $ERROR_LIMIT lines."
    exit 0
fi

errors=0
warnings=0
shrunk=0

for file in $(list_files); do
    is_exempt "$file" && continue
    [ -f "$file" ] || continue
    lines=$(wc -l < "$file")
    recorded=$(baseline_for "$file")

    if [ "$lines" -gt "$ERROR_LIMIT" ]; then
        if [ -z "$recorded" ]; then
            echo "ERROR: $file has $lines lines (limit: $ERROR_LIMIT)"
            errors=$((errors + 1))
        elif [ "$lines" -gt "$recorded" ]; then
            echo "ERROR: $file grew to $lines lines (baseline: $recorded)"
            errors=$((errors + 1))
        elif [ "$lines" -lt "$recorded" ]; then
            shrunk=$((shrunk + 1))
        fi
    elif [ -n "$recorded" ]; then
        # Dropped under the limit entirely — the best outcome.
        shrunk=$((shrunk + 1))
    elif [ "$lines" -gt "$WARN_LIMIT" ]; then
        # Listed individually only when scanning what you are about to commit.
        # A full scan naming all ~17 files every run is noise that trains people
        # to skip the output, which is how the ERROR lines get missed too.
        if [ "$mode" = "staged" ]; then
            echo "WARNING: $file has $lines lines (warn: $WARN_LIMIT)"
        fi
        warnings=$((warnings + 1))
    fi
done

if [ "$errors" -gt 0 ]; then
    echo
    echo "$errors file(s) exceed $ERROR_LIMIT lines or grew past their baseline."
    echo "Split the file, or if a grandfathered file legitimately shrank elsewhere,"
    echo "run: scripts/check-file-size.sh --update-baseline"
    exit 1
fi
if [ "$shrunk" -gt 0 ]; then
    echo "$shrunk baselined file(s) shrank — run scripts/check-file-size.sh --update-baseline to tighten."
fi
if [ "$warnings" -gt 0 ]; then
    echo "$warnings file(s) approaching the $ERROR_LIMIT-line limit (over $WARN_LIMIT)."
fi
exit 0
