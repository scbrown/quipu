#!/usr/bin/env bash
#
# Build and deploy quipu-server with a FAIL-LOUD gate that makes the
# silent-build-skip impossible instead of merely caught-this-time.
#
# THE BUG THIS EXISTS TO KILL: quipu-server declares
# `required-features = ["shacl", "onnx"]` in Cargo.toml. A plain
# `cargo build --release` builds the lib and the feature-free bins, SILENTLY
# EXCLUDES quipu-server (exit 0, no warning), and leaves whatever server binary
# was there before. A deploy that copies "the built binary" then ships a STALE
# one — once observed swapping a pre-confidence binary live, caught only because
# a downstream test wrote 0 triples. This script builds with the required
# features and then PROVES the binary is this build's output before staging it.
#
# Public-safe: no hostnames/paths beyond generic defaults. Override via env:
#   BUILD_DIR   repo/build checkout            (default: current dir)
#   INSTALL_TARGETS  space-separated dest paths (default: /usr/local/bin/quipu-server)
#   SERVICE     systemd unit to restart        (default: quipu; empty = skip)
#   HEALTH_URL  post-deploy check base         (default: http://localhost:3030)
#   NO_DEPLOY=1 build + gate only, do not install/restart
#   QUIPU_AUTH_TOKEN  bearer for the /shapes gate (that route needs auth to LIST)
#   QUIPU_TOKEN_FILE  file to read the bearer from, if the env var is unset
#   SHAPES_CHECK_ONLY=1  run ONLY the shapes gate against HEALTH_URL and exit;
#                        neither builds nor deploys, so both of its branches are
#                        exercisable without a deploy
#   DEPLOY_ACTOR  WHO is deploying — REQUIRED to install (see below)
#   DEPLOY_LOG    attribution log path         (default: /var/log/quipu-deploy.log)
#
# ── WHY DEPLOY_ACTOR IS REQUIRED, AND WHY IT REFUSES ──
# Every agent on this fleet reaches the deploy target as the SAME root over the
# SAME ssh key from ONE host. The deployer's identity is therefore not in the
# ssh session, the sudo record, or any system log — no log on the box can name
# a deployer, and none ever will unless the caller asserts it.
#
# Measured cost of that gap, once: answering "who deployed?" took a whole-crew
# per-agent transcript sieve plus a content search, excluding 14 of 24 candidate
# windows by mechanism. One line in a log answers it instantly. Worse, that
# forensic route worked BY COINCIDENCE — it depended on every agent happening to
# run on one host with readable transcripts. Move one agent off-host and the
# same question becomes unanswerable, with no warning that the capability was
# ever lost.
#
# So this REFUSES rather than defaulting. "unknown" is the value the field would
# take on every unattributed deploy, so a log that accepts it reproduces the
# exact blind spot while looking like it fixed it. The refusal costs one env
# var; an unattributable deploy costs the next incident.
set -euo pipefail

BIN=quipu-server
FEATURES="shacl,onnx"
BUILD_DIR="${BUILD_DIR:-$PWD}"
INSTALL_TARGETS="${INSTALL_TARGETS:-/usr/local/bin/quipu-server}"
SERVICE="${SERVICE-quipu}"
HEALTH_URL="${HEALTH_URL:-http://localhost:3030}"

# cargo may not be on PATH under a non-login shell (systemd/ssh). Pull it in from
# the standard rustup location before giving up, so the script works whether the
# operator ran a login shell or not.
command -v cargo >/dev/null 2>&1 || { [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"; }
command -v cargo >/dev/null 2>&1 || PATH="$HOME/.cargo/bin:$PATH"

# Building needs the build-dir owner (rustup); installing to system paths and
# restarting the service needs root. When not already root, escalate ONLY those
# two steps via sudo (kept out of the build so cargo runs as the owner).
SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO="sudo"

say() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }
die() { printf '\n\033[1;31mDEPLOY ABORTED: %s\033[0m\n' "$*" >&2; exit 1; }

DEPLOY_LOG="${DEPLOY_LOG:-/var/log/quipu-deploy.log}"
DEPLOY_ACTOR="${DEPLOY_ACTOR:-${SHANTY_AGENT:-${GT_CREW:-}}}"

# The placeholders are REJECTED BY NAME, because each is a value the shared-root
# deploy path hands you for free — and a free value is the one that ends up in
# the log. `whoami` on the target is literally "root" for every agent, so
# accepting it would log a fact that is true, useless, and identical for all 19.
case "$(printf '%s' "${DEPLOY_ACTOR:-}" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')" in
  ''|unknown|none|null|root|nobody|-|n/a|na|tbd|agent|crew|user)
    DEPLOY_ACTOR="" ;;
esac

require_actor() {
  [ -n "$DEPLOY_ACTOR" ] && return 0
  die "DEPLOY_ACTOR is not set (or names nobody in particular).

  This deploy would mutate a shared host that CANNOT tell agents apart: every
  agent arrives as the same root over the same key, so nothing downstream —
  not sshd, not sudo, not the journal — can record who you are. If this
  install lands unattributed, the next 'who deployed?' is answered by sieving
  transcripts, or not at all.

  Say who you are, then re-run:
      DEPLOY_ACTOR=<your-agent-name> $0
  \$SHANTY_AGENT / \$GT_CREW are used automatically when set, so an agent
  session normally needs nothing. Placeholders (unknown, root, none, ...) are
  rejected on purpose: they are what the shared path gives you for free, and
  logging one is the same blind spot with a timestamp on it."
}

# ONE LINE PER EVENT, APPEND-ONLY, WRITTEN AS IT HAPPENS. Fields are k=v so the
# log stays greppable without a parser. Note this is called at INSTALL time and
# again after verification rather than once at the end: the install is the
# moment the shared host is MUTATED, and a deploy that clobbers someone and then
# dies at a later gate is exactly the forensic case — a single line written only
# on success would omit precisely the deploys worth attributing.
deploy_log() {
  local line
  line="$(date -u +%Y-%m-%dT%H:%M:%SZ) actor=$DEPLOY_ACTOR from=$(hostname 2>/dev/null || echo '?') pid=$$ $*"
  if ! printf '%s\n' "$line" | $SUDO tee -a "$DEPLOY_LOG" >/dev/null 2>&1; then
    # NEVER FATAL, ALWAYS LOUD. Refusing the deploy because the audit line
    # could not be written would take a working deploy path down over its
    # seatbelt; staying quiet would leave a silent hole in the one record this
    # whole change exists to create. So: say so, on stderr, and carry on.
    printf '\033[1;33mWARN: could not append to %s — this deploy is UNLOGGED: %s\033[0m\n' \
      "$DEPLOY_LOG" "$line" >&2
  fi
}

check_shapes() {
  local token body code count
  token="${QUIPU_AUTH_TOKEN:-}"
  if [ -z "$token" ] && [ -n "${QUIPU_TOKEN_FILE:-}" ] && [ -r "$QUIPU_TOKEN_FILE" ]; then
    token=$(cat "$QUIPU_TOKEN_FILE")
  fi

  body=$(mktemp)
  # `-w '%{http_code}'` ALREADY prints 000 when the request never completes, so a
  # `|| echo 000` fallback CONCATENATES with it and yields "000000" — which then
  # falls through to the catch-all branch and reports a nonsense status. Observed.
  # Take curl's word, and normalise anything that is not three digits.
  code=$(curl -s -m 8 -o "$body" -w '%{http_code}' \
    -X POST -H 'Content-Type: application/json' \
    ${token:+-H "Authorization: Bearer $token"} \
    -d '{"action":"list"}' "$HEALTH_URL/shapes") || true
  case "$code" in
    [0-9][0-9][0-9]) : ;;
    *) code=000 ;;
  esac

  case "$code" in
    200) : ;;
    401|403)
      rm -f "$body"
      die "shapes gate could not AUTHENTICATE to $HEALTH_URL/shapes (HTTP $code) — /shapes needs a bearer even to list. This says NOTHING about whether shapes are loaded. The binary IS installed and the service IS serving (the gates above passed); only this last check did not run. Set QUIPU_AUTH_TOKEN (or QUIPU_TOKEN_FILE) and re-run to complete it." ;;
    000)
      rm -f "$body"
      die "shapes gate got no response from $HEALTH_URL/shapes (timeout/connection). The service answered /health and /version moments ago, so investigate before assuming shapes are gone." ;;
    *)
      rm -f "$body"
      die "shapes gate got HTTP $code from $HEALTH_URL/shapes — unexpected; not treating it as a shapes count." ;;
  esac

  count=$(grep -o '"count":[0-9]*' "$body" | head -1 | cut -d: -f2)
  rm -f "$body"
  [ -n "$count" ] \
    || die "shapes gate got HTTP 200 but no \"count\" field — the response shape changed; this gate is no longer measuring what it claims."
  [ "$count" -ge 1 ] \
    || die "shacl is compiled in but ZERO shapes are loaded — validation is a no-op. Load shapes and re-run, or the /shapes count>0 invariant is broken."
  echo "shapes loaded: count=$count — SHACL is actually validating."
}

if [ "${SHAPES_CHECK_ONLY:-0}" = 1 ]; then
  say "shapes gate only, against $HEALTH_URL"
  check_shapes
  exit 0
fi

# REFUSE BEFORE THE BUILD, NOT AT THE INSTALL. The build is minutes; failing a
# one-env-var precondition after it is how a gate earns a wrapper that presets
# the variable to something meaningless just to stop the nagging. Only when this
# run will actually install: NO_DEPLOY builds and mutates nothing, so demanding
# an identity for it would be friction with no audit value.
[ "${NO_DEPLOY:-0}" = 1 ] || require_actor

cd "$BUILD_DIR"
ARTIFACT="target/release/$BIN"
command -v cargo >/dev/null 2>&1 \
  || die "cargo not found (looked on PATH and in \$HOME/.cargo). Run as the build-dir owner, or install rustup."

# Prove the required features are still declared — if someone drops them from
# Cargo.toml, a bare build would "succeed" and this script would build the
# server WITHOUT the features rather than failing. Guard the assumption itself.
grep -Eq 'required-features *= *\[.*"shacl".*"onnx"' Cargo.toml \
  || die "Cargo.toml no longer declares required-features=[shacl,onnx] for $BIN — the feature contract this gate depends on has changed; re-verify before deploying."

prior_sha=""
[ -f "$ARTIFACT" ] && prior_sha=$(sha256sum "$ARTIFACT" | cut -d' ' -f1)
built_head=$(git rev-parse HEAD 2>/dev/null || echo "")

say "cargo build --release --bin $BIN --features $FEATURES"
cargo build --release --bin "$BIN" --features "$FEATURES"

# ── FAIL-LOUD GATE 1: the build actually PRODUCED the server binary ──
# The silent-skip signature is a MISSING artifact: a bare `cargo build` excludes
# quipu-server from the build graph entirely (required-features), exits 0, and
# writes nothing. Because this script ALWAYS passes --bin --features, cargo will
# error (caught by set -e) if it cannot build it — so a missing artifact here
# means something is deeply wrong. Freshness-vs-source is proven downstream by
# the feature symbols (GATE 2) and the RUNNING git_sha (GATE 3.5), NOT by mtime:
# cargo legitimately cache-hits an up-to-date binary and leaves its mtime old,
# which an mtime>=build_start gate would false-fail on an unchanged-source deploy.
[ -f "$ARTIFACT" ] \
  || die "$ARTIFACT was NOT produced despite an explicit --bin $BIN --features $FEATURES build. A bare build silently skips $BIN (required-features); something bypassed the feature flags."

new_sha=$(sha256sum "$ARTIFACT" | cut -d' ' -f1)
say "built: $ARTIFACT  sha256=${new_sha:0:12}  ($(stat -c %s "$ARTIFACT") bytes)"
if [ -n "$prior_sha" ] && [ "$prior_sha" = "$new_sha" ]; then
  echo "note: identical to the previous build (source unchanged) — deploying is a no-op swap, allowed."
fi

# ── FAIL-LOUD GATE 2: the compiled binary actually carries the features ──
# Cheap symbol smoke check: onnx + shacl codepaths leave strings in the binary.
# grep -c (not -q): -q closes the pipe on first match, strings then dies with
# SIGPIPE and `set -o pipefail` turns that into a false failure. -c reads the
# whole stream. (Same SIGPIPE-under-pipefail trap as the checksum gate elsewhere.)
[ "$(strings "$ARTIFACT" | grep -ci onnx)"  -gt 0 ] || die "built $BIN carries NO onnx symbols — the feature did not compile in. Do not deploy."
[ "$(strings "$ARTIFACT" | grep -ci shacl)" -gt 0 ] || die "built $BIN carries NO shacl symbols — the feature did not compile in. Do not deploy."
echo "feature smoke: onnx + shacl symbols present."

if [ "${NO_DEPLOY:-0}" = 1 ]; then
  say "NO_DEPLOY=1 — build + gate passed, not installing."; exit 0
fi

# ── deploy: back up each target, then install ──
# The tree's OWN dirty flag is recorded, not just the sha: a build from a dirty
# tree is not reproducible from its sha, and "dirty" was the fact that told
# a past incident that the install had bypassed this script entirely. Recording
# also gives a second, independent reading to set against what /version claims,
# it here also gives a second, independent reading to set against what /version
# claims — a discrepancy currently open against this deployment.
built_dirty=no
[ -n "$(git status --porcelain 2>/dev/null)" ] && built_dirty=yes

for dest in $INSTALL_TARGETS; do
  [ -f "$dest" ] && $SUDO cp -p "$dest" "$dest.bak-$(date +%Y%m%d%H%M%S)"
  $SUDO install -m 755 "$ARTIFACT" "$dest"
  echo "installed -> $dest"
  deploy_log "event=install dest=$dest built_sha=${built_head:-?} dirty=$built_dirty bin_sha256=${new_sha:0:16}"
done

if [ -n "$SERVICE" ]; then
  say "restart $SERVICE"
  $SUDO systemctl restart "$SERVICE"
  sleep 5
  $SUDO systemctl is-active --quiet "$SERVICE" || die "$SERVICE is not active after restart — check journalctl -u $SERVICE."
fi

# ── FAIL-LOUD GATE 3: the RUNNING server proves it, post-deploy ──
say "post-deploy verification against $HEALTH_URL"
code=$(curl -s -o /dev/null -w '%{http_code}' -m 8 "$HEALTH_URL/health" || true)
[ "$code" = 200 ] || die "$HEALTH_URL/health returned $code (want 200)."

ver=$(curl -s -m 8 "$HEALTH_URL/version")
grep -q '"onnx":true'  <<<"$ver" || die "/version does not report onnx enabled — the deployed binary is not the feature build. Running: $ver"
grep -q '"shacl":true' <<<"$ver" || die "/version does not report shacl enabled — the deployed binary is not the feature build. Running: $ver"
sha_running=$(echo "$ver" | grep -o '"git_sha":"[a-f0-9]*"' | cut -d'"' -f4)
dirty_running=$(echo "$ver" | grep -o '"git_dirty":[a-z]*' | cut -d: -f2)
echo "running: sha=${sha_running:0:8} features onnx+shacl confirmed, health 200."

# LOG WHAT IS SERVING — and log it HERE, BEFORE gate 3.5 can abort.
# The shipped sha and the SERVED sha are different facts, and recording only the
# first is worse than useless: a box has been observed whose tree read
# 4aba263+clean while /version reported f44afc9+dirty, so a log saying "deployed
# 4aba263" would have been confidently misleading.
#
# The ordering is the load-bearing part. Gate 3.5 EXITS when the served sha is
# not the built one, and that is precisely the case worth having on disk — a
# deploy that did not take, or someone else's binary already serving. Logging
# after the gate would record only the deploys that went fine.
deploy_log "event=serving sha=${sha_running:-unreadable} dirty=${dirty_running:-?} built_sha=${built_head:-?} health=$code"

# ── FAIL-LOUD GATE 3.5: the RUNNING binary is the code we just built ──
# The definitive anti-stale check and the direct heir of the original bug (a
# stale binary swapped live): if the server reports a git_sha other than the
# HEAD we built, the new binary is NOT serving — the deploy did not take.
if [ -n "$built_head" ] && [ -n "$sha_running" ]; then
  case "$built_head" in
    "$sha_running"*) : ;;
    *) die "running git_sha ${sha_running:0:8} != built HEAD ${built_head:0:8} — the deployed binary is NOT the code just built. Stale binary is live (the exact failure this bead exists for). Check the install targets and that $SERVICE loads from one of them." ;;
  esac
  echo "sha match: running == built HEAD (${built_head:0:8}) — the new binary is serving."
fi

# ── FAIL-LOUD GATE 4: shacl COMPILED IN is not
# shacl DOING ANYTHING. Shapes must be LOADED, or every validation is a silent
# pass. The deploy path loads them; this asserts it stuck (and
# catches the redeploy-wiped-shapes case). ──
#
# /shapes REQUIRES A BEARER EVEN TO LIST — unlike /health, /version, /query and
# /search, which stay open. This gate predates that auth change and sent no
# token, so it read the 401 body as "no count field", defaulted count to 0, and
# aborted with "ZERO shapes are loaded" — a SHACL outage that was not happening,
# reported by the one check meant to prove SHACL was fine.
#
# The damage is in the ORDERING: this gate runs AFTER install and restart, so
# every deploy since the auth change installed correctly, restarted correctly,
# then announced DEPLOY ABORTED. An operator who believes it either rolls back a
# good deploy or goes hunting a SHACL failure that does not exist. A check that
# cannot authenticate must say SO — it must never report the thing it failed to
# measure as broken.
#
# Token: $QUIPU_AUTH_TOKEN, or the file at $QUIPU_TOKEN_FILE if set.
#
# Run JUST this gate — against any server, without building or deploying — with
# SHAPES_CHECK_ONLY=1. A gate that fires only at the end of a deploy is a gate
# nobody can exercise, which is how this one stayed wrong: both of its branches
# are now runnable in a second.
check_shapes

say "DEPLOY OK — $BIN built with $FEATURES, gated at build + binary + runtime + shapes."
