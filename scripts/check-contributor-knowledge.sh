#!/usr/bin/env bash
# Shared by pre-commit (including CI) and the local just recipe.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
node --test "$ROOT/scripts/build-contributor-knowledge.test.mjs"
"$ROOT/scripts/check-explorer-published.test.sh"
python3 "$ROOT/scripts/test_private_share_policy.py"
python3 "$ROOT/scripts/test_repository_share_provenance.py"
