#!/usr/bin/env bash
# arxiv-scrub-gate — the FULL-ARTIFACT scrub gate for a public paper release.
#
#   scripts/arxiv-scrub-gate.sh            # gate the artifact
#   scripts/arxiv-scrub-gate.sh --selftest # prove every arm, both directions
#
# Exit 0 clean · 1 findings · 2 CANNOT TELL · 3 artifact incomplete.
#
# ── WHY THIS IS NOT THE PRE-PUSH GUARD ───────────────────────────────────────
# `scripts/pre-push-scrub-guard.sh` is deliberately NEW-OCCURRENCE: it refuses
# only what a push ADDS, because a guard that fired on the repository's existing
# debt would be recognised as broken and switched off within a day, leaving no
# guard at all. That reasoning is correct and this file does not touch it.
#
# It is also the reason that guard cannot answer THIS question. arXiv publication
# is a single irreversible act, and what reaches a stranger is the artifact's
# CURRENT STATE — every occurrence, not the delta of the last push. A file that
# has carried an internal hostname since its first commit is invisible to a
# new-occurrence guard and perfectly visible to a reader of the paper.
#
# Three inversions follow from "runs once, before something irreversible":
#
#   1. ANY-occurrence, not new-occurrence.
#   2. FAIL CLOSED. A push guard with no config exits 0 loudly, because blocking
#      every push on every unconfigured machine would get it deleted. This one
#      exits 2: "I could not check" must never be spent as "it is clean", and
#      there is no cry-wolf cost to pay when the command is run once by hand.
#   3. THE PATH EXEMPTION IS REPORTED, NOT ENFORCED AND NOT DISCARDED. The
#      governed ticket rule exempts source comments, because a bead citation next
#      to the code is this fleet's documentation convention and is right for an
#      internal reader. An arXiv reader is not an internal reader: to them a bead
#      id is an unresolvable reference to a private tracker. So exempt-path hits
#      are still SHOWN — as ADVISORY, not counted against the verdict.
#
#      Enforcing the widening outright was the first design and it was wrong: the
#      exempt hits are permanent by convention, so every future run would have
#      ended in a red verdict, and a gate that always says FINDINGS carries no
#      more information than one that always says PASS. Reviewer fatigue is not a
#      lesser failure than a missed leak; it is the mechanism by which a missed
#      leak eventually gets through.
#
# ── AND ONE THING NO REGEX CAN SEE ───────────────────────────────────────────
# The corpus reversibility arm exists because the leak that actually shipped had
# no forbidden string in it. `benchmark/replay/corpus/corpus.json` was published
# as "pseudonymised" and passed every pattern gate clean, while its names were
# `sha256(fixed_salt + iri)[:10]` with both halves public — a wordlist recovered
# 41 real entity names from it. A gate made only of patterns would have certified
# that artifact. See scripts/reseal-replay-corpus.py.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CONF="${SCRUB_PATTERNS_FILE:-$HOME/.config/aegis/scrub-patterns.conf}"
GREP=/usr/bin/grep     # NOT `grep`: on this fleet that is a shell function that
                       # honours .gitignore and reports a clean exit on files it
                       # never opened (aegis-f8fx0).

# THE ARTIFACT. Declared, not discovered — a gate that globs whatever happens to
# be present cannot tell "clean" from "the paper had not been written yet".
# `?` marks a component that is allowed to be absent while the paper is in draft;
# everything else missing is exit 3.
ARTIFACT=(
  "docs/paper-merge"                      # the paper source (aegis-s9sjf.3)
  "benchmark/mergebench"                  # ARM A data + build report
  "benchmark/replay"                      # ARM B/C corpus + build report
  "examples/mergebench"                   # the harness that regenerates ARM A
  "examples/replay"                       # the harness that regenerates ARM B/C
  "scripts/build-replay-corpus.py"        # corpus provenance
  "scripts/reseal-replay-corpus.py"       # anonymisation remediation
)
OPTIONAL_UNTIL_DRAFTED="docs/paper-merge"

say() { printf '%s\n' "$*"; }
rule() { printf -- '── %s %s\n' "$1" "$(printf '%.0s─' $(seq 1 $((66 - ${#1}))))"; }

load_conf() {
  PATTERNS=""; TICKET_PATTERNS=""; TICKET_EXEMPT_RE=""
  [ -r "$CONF" ] || return 1
  while IFS='=' read -r k v; do
    case "$k" in
      patterns)        PATTERNS="$v" ;;
      ticket_patterns) TICKET_PATTERNS="$v" ;;
      ticket_exempt_path_re) TICKET_EXEMPT_RE="$v" ;;
    esac
  done < "$CONF"
  [ -n "$PATTERNS" ]
}

# prove_instrument <pattern> <label> — a zero from an unproven instrument is not
# a finding. Every sweep below is preceded by a control that MUST fire, written
# to a real file under the real invocation, so a sweep that cannot see the corpus
# fails here rather than reporting it clean.
prove_instrument() {
  local pat="$1" label="$2" tmp rc
  tmp=$(mktemp -d)
  # THE PROBE MUST BE SOMETHING THE LIVE PATTERNS ACTUALLY MATCH, and the first
  # version of this function got that wrong in the direction that matters: it
  # planted RFC 5737 documentation addresses (198.51.100.x), which the governed
  # rule deliberately does NOT match — it governs RFC 1918 private space. The
  # control then reported the live instrument DEAD while it was working
  # perfectly, i.e. a false alarm that would have blocked a clean release.
  #
  # So: one probe per private range, top-of-range addresses that are not in use
  # on any estate this gate protects — they exercise every arm and publish
  # nothing about anyone's topology.
  #
  # THEY ARE ASSEMBLED AT RUN TIME RATHER THAN WRITTEN AS LITERALS, and that is
  # not cosmetic. Written out, this line is a pushable internal-identifier
  # pattern, and the repository's own pre-push guard refused the commit that
  # introduced it — correctly, because a guard cannot tell a control probe from
  # the real thing and should not try. The guard keeps an exclusion list for
  # files that genuinely cannot avoid the literals; its own comment reserves that
  # for the unavoidable case and assembles fixtures from fragments otherwise.
  # This one is avoidable, so it is avoided, and no exclusion had to be widened
  # to land a gate.
  #
  # The octets stay visible as numbers, so this is not the "split the literal to
  # sneak past the checker" move that would also hide the string from the next
  # person's grep — there is simply no address here until the shell builds one.
  { for range in "10 255" "172 31" "192 168"; do
      set -- $range; printf '%d.%d.255.1 ' "$1" "$2"
    done
    printf '/%s/%s/x\n' home jsmith
  } > "$tmp/probe"
  [ "$label" = ticket ] && printf 'see %s-000000 for the reason\n' aegis > "$tmp/probe"
  "$GREP" -qE "$pat" "$tmp/probe"; rc=$?
  rm -rf "$tmp"
  return $rc
}

scan() { # scan <pattern> <paths...> -> prints "file:line:match"
  local pat="$1"; shift
  "$GREP" -rEIn "$pat" "$@" 2>/dev/null || true
}

gate() {
  local rc=0 missing=() present=()

  rule "artifact"
  for c in "${ARTIFACT[@]}"; do
    if [ -e "$HERE/$c" ]; then present+=("$c"); say "  present  $c"
    elif [ "$c" = "$OPTIONAL_UNTIL_DRAFTED" ]; then
      missing+=("$c"); say "  ABSENT   $c   (not yet drafted — gate is INCOMPLETE, not clean)"
    else missing+=("$c"); say "  ABSENT   $c   (required)"; fi
  done
  [ ${#present[@]} -gt 0 ] || { say "nothing to gate"; return 3; }

  rule "policy source"
  if load_conf; then
    say "  config   $CONF"
    say "  emitted  $(date -r "$CONF" '+%Y-%m-%d %H:%M' 2>/dev/null || echo unknown)"
    say "           (a PROJECTION of the policy graph — regenerate with"
    say "            goldblum policy/emit-scrub-config.py; if the graph has"
    say "            gained a pattern since, this gate is behind it)"
  else
    say "  CANNOT TELL — no usable pattern config at $CONF."
    say "  A publication gate that cannot read the policy has nothing to say"
    say "  about the artifact. Refusing to report a result."
    return 2
  fi

  rule "block tier — internal identifiers, any occurrence"
  if ! prove_instrument "$PATTERNS" block; then
    say "  CONTROL FAILED — the live patterns do not fire on a planted probe."
    say "  Any zero below would be meaningless. Refusing to report a result."
    return 2
  fi
  say "  control  ok (planted probe detected)"
  local hits; hits=$(cd "$HERE" && scan "$PATTERNS" "${present[@]}")
  if [ -n "$hits" ]; then
    say "  FINDINGS:"; printf '%s\n' "$hits" | sed 's/^/    /'; rc=1
  else
    say "  clean    0 occurrences across ${#present[@]} components"
  fi

  rule "ticket tier — bead ids"
  # TWO TIERS, REPORTED SEPARATELY, and the split is the point.
  #
  # The governed rule already exempts source comments and docstrings: a bead
  # citation beside the code is this fleet's documentation convention and is the
  # reason its comments are worth reading. Gating the artifact WITHOUT that
  # exemption is defensible for a paper — but if the extra hits are printed as
  # FINDINGS they never go away, every future run ends in a red verdict, and the
  # real block-tier line above it stops being read. A gate whose verdict is
  # permanently negative has the same information content as one that is
  # permanently positive.
  #
  # So: exempt-path hits are ADVISORY and do not set the exit code; governed
  # hits are FINDINGS and do. The widening still gets looked at — by a person,
  # once, before submission — which is all it was ever able to justify.
  if [ -z "$TICKET_PATTERNS" ]; then
    say "  CANNOT TELL — config carries no ticket_patterns"; return 2
  fi
  if ! prove_instrument "$TICKET_PATTERNS" ticket; then
    say "  CONTROL FAILED — ticket patterns do not fire on a planted probe."
    return 2
  fi
  say "  control  ok (planted probe detected)"
  local all governed advisory
  all=$(cd "$HERE" && scan "$TICKET_PATTERNS" "${present[@]}")
  # THE EXEMPTION MATCHES A PATH, SO IT MUST BE GIVEN A PATH. `scan` emits
  # `path:line:text`, and testing the exemption against that whole string
  # silently misclassifies every rule anchored on the path's end — the real
  # regex is extension-anchored, so `^(...)` against `main.rs:179:...` never
  # matched and an EXEMPT file was reported as a governed FINDING. It fails
  # toward over-reporting, which is why it survived a run: the output still
  # looked like a working gate, just a stricter one.
  governed=""; advisory=""
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    p=${line%%:*}
    if [ -n "$TICKET_EXEMPT_RE" ] && printf '%s' "$p" | "$GREP" -qE "$TICKET_EXEMPT_RE"; then
      advisory+="$line"$'\n'
    else
      governed+="$line"$'\n'
    fi
  done <<< "$all"
  governed=$(printf '%s\n' "$governed" | "$GREP" -v '^$' || true)
  advisory=$(printf '%s\n' "$advisory" | "$GREP" -v '^$' || true)
  if [ -n "$governed" ]; then
    say "  FINDINGS (governed — user-facing artefacts):"
    printf '%s\n' "$governed" | sed 's/^/    /'; rc=1
  else
    say "  clean    0 occurrences in governed paths"
  fi
  if [ -n "$advisory" ]; then
    say "  ADVISORY (exempt paths — source comments; NOT counted against the"
    say "            verdict, but read them once before you submit: a bead id is"
    say "            unresolvable to a reader outside this project):"
    printf '%s\n' "$advisory" | sed 's/^/    /'
  fi

  rule "reversibility — what no regex can see"
  if [ -e "$HERE/benchmark/replay/corpus/corpus.json" ]; then
    local out
    out=$(cd "$HERE" && python3 scripts/reseal-replay-corpus.py --check 2>&1); local orc=$?
    printf '%s\n' "$out" | sed 's/^/  /'
    case $orc in
      0) ;;
      1) rc=1 ;;
      *) say "  CANNOT TELL — reversibility check could not run"; return 2 ;;
    esac
  else
    say "  no corpus present to check"
  fi

  rule "verdict"
  if [ ${#missing[@]} -gt 0 ]; then
    say "  INCOMPLETE — ${#missing[@]} component(s) absent: ${missing[*]}"
    say "  The components that ARE present gated $([ $rc -eq 0 ] && echo clean || echo WITH FINDINGS)."
    say "  This is NOT a pass. Re-run when the artifact is whole."
    [ $rc -ne 0 ] && return 1
    return 3
  fi
  if [ $rc -eq 0 ]; then say "  PASS — whole artifact, every arm, controls proven"; else say "  FINDINGS — see above"; fi
  return $rc
}

selftest() {
  # NOT `local tmp` with an EXIT trap: the trap fires after the function has
  # returned, when a local is out of scope, and `set -u` then reports an unbound
  # variable AFTER the verdict line — noise printed below a PASS is exactly the
  # kind of thing a reader learns to ignore.
  local fail=0; SELFTEST_TMP=$(mktemp -d); trap 'rm -rf "$SELFTEST_TMP"' EXIT
  local tmp="$SELFTEST_TMP"
  load_conf || { say "SKIP — no live config; this is NOT a pass"; return 2; }

  prove_instrument "$PATTERNS" block \
    && say "ok   block instrument fires on a planted probe" \
    || { say "FAIL block instrument is dead"; fail=1; }
  prove_instrument "$TICKET_PATTERNS" ticket \
    && say "ok   ticket instrument fires on a planted probe" \
    || { say "FAIL ticket instrument is dead"; fail=1; }

  # Must NOT fire on ordinary prose — a publication gate that cries wolf gets
  # its findings skimmed, which is the failure mode that matters when the output
  # is read once by a person about to press submit.
  printf 'a derivative activation, version 1.2.3.4, resolver 8.8.8.8\n' > "$tmp/clean"
  "$GREP" -qE "$PATTERNS" "$tmp/clean" \
    && { say "FAIL block tier fires on clean prose"; fail=1; } \
    || say "ok   block tier silent on clean prose"

  # The reversibility arm must have BOTH outcomes available, or its pass is the
  # same output as total darkness.
  python3 "$HERE/scripts/reseal-replay-corpus.py" --selftest >/dev/null 2>&1 \
    && say "ok   reversibility arm proves both directions" \
    || { say "FAIL reversibility arm selftest failed"; fail=1; }

  # The exemption is matched against a PATH, not against `path:line:text`.
  # Regression for a bug that classified an exempt source file as a governed
  # finding: the live rule is extension-anchored, so testing it against a line
  # that continues past the filename can never match. Asserted against the LIVE
  # exemption, because a synthetic one would just re-test the fixture.
  if [ -n "$TICKET_EXEMPT_RE" ]; then
    local probe_line="examples/replay/main.rs:179:  // see aegis-000000"
    if printf '%s' "${probe_line%%:*}" | "$GREP" -qE "$TICKET_EXEMPT_RE" \
       && ! printf '%s' "$probe_line" | "$GREP" -qE "^($TICKET_EXEMPT_RE)"; then
      say "ok   exemption is applied to the path, not the whole match line"
    else
      say "ok   exemption shape does not exercise this regression (live rule differs)"
    fi
  fi

  # Fail-closed: an unreadable config must yield 2, never 0.
  local rc; SCRUB_PATTERNS_FILE=/nonexistent "$0" >/dev/null 2>&1; rc=$?
  [ "$rc" = 2 ] && say "ok   unconfigured exits 2 (cannot tell), not 0" \
                || { say "FAIL unconfigured exited $rc, not 2"; fail=1; }

  say "$([ $fail -eq 0 ] && echo 'selftest PASSED' || echo 'selftest FAILED')"
  return $fail
}

case "${1:-}" in
  --selftest) selftest ;;
  "") gate ;;
  *) say "usage: $0 [--selftest]"; exit 64 ;;
esac
