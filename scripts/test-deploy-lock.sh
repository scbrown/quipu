#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
deploy="$here/build-deploy-server.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
lock="$tmp/deploy.lock"
log="$tmp/deploy.log"

DEPLOY_ACTOR=first DEPLOY_LOCK="$lock" DEPLOY_LOG="$log" \
  DEPLOY_LOCK_CHECK_ONLY=1 DEPLOY_LOCK_HOLD_SECONDS=20 \
  "$deploy" >"$tmp/first.out" 2>"$tmp/first.err" &
holder=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
  [ -s "${lock}.holder" ] && break
  sleep 0.1
done
[ -s "${lock}.holder" ] || { echo "FAIL: first deploy never acquired lock"; exit 1; }

set +e
blocked=$(DEPLOY_ACTOR=second DEPLOY_LOCK="$lock" DEPLOY_LOG="$log" \
  DEPLOY_LOCK_CHECK_ONLY=1 "$deploy" 2>&1)
blocked_rc=$?
set -e
[ "$blocked_rc" -eq 1 ] || { echo "FAIL: contender returned $blocked_rc, want 1"; exit 1; }
printf '%s' "$blocked" | grep -q 'actor=first' \
  || { echo "FAIL: refusal did not name holder"; exit 1; }

# SIGTERM simulates a killed deploy. flock is process-owned, so the next run
# must acquire without deleting the lock file or rebooting.
kill "$holder"
wait "$holder" 2>/dev/null || true
DEPLOY_ACTOR=recovery DEPLOY_LOCK="$lock" DEPLOY_LOG="$log" \
  DEPLOY_LOCK_CHECK_ONLY=1 "$deploy"

[ "$(grep -c 'event=lock-acquire' "$log")" -eq 2 ]
[ "$(grep -c 'event=lock-release' "$log")" -eq 2 ]
echo "deploy lock: contention refused with holder; killed holder recovered; 2 acquire/release pairs logged"
