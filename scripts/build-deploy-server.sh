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
#
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
for dest in $INSTALL_TARGETS; do
  [ -f "$dest" ] && $SUDO cp -p "$dest" "$dest.bak-$(date +%Y%m%d%H%M%S)"
  $SUDO install -m 755 "$ARTIFACT" "$dest"
  echo "installed -> $dest"
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
echo "running: sha=${sha_running:0:8} features onnx+shacl confirmed, health 200."

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
shapes=$(curl -s -m 8 -X POST -H 'Content-Type: application/json' \
  -d '{"action":"list"}' "$HEALTH_URL/shapes" || true)
count=$(echo "$shapes" | grep -o '"count":[0-9]*' | head -1 | cut -d: -f2)
[ "${count:-0}" -ge 1 ] \
  || die "shacl is compiled in but ZERO shapes are loaded — validation is a no-op. Load shapes and re-run, or the /shapes count>0 invariant is broken."
echo "shapes loaded: count=$count — SHACL is actually validating."

say "DEPLOY OK — $BIN built with $FEATURES, gated at build + binary + runtime + shapes."
