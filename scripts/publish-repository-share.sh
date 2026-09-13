#!/usr/bin/env bash
# Trusted-host producer. Its environment and working directory are private.
set -euo pipefail
umask 077

if [[ $# != 6 || ( $1 != --prepare && $1 != --publish ) ]]; then
    echo "usage: $0 <--prepare|--publish> <tag> <quipu-bin> <bobbin-bin> <source-repo> <new-output-dir>" >&2
    exit 2
fi
MODE=$1
TAG=$2
QUIPU_BIN=$(readlink -f "$3")
BOBBIN_BIN=$(readlink -f "$4")
SOURCE=$(readlink -f "$5")
OUTPUT=$6
[[ "$TAG" =~ ^quipu-ai-v[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.-]+)?$ ]] || exit 2
test ! -e "$OUTPUT" || { echo 'output already exists' >&2; exit 2; }
REVISION=$(git -C "$SOURCE" rev-parse HEAD)
test "$REVISION" = "$(git -C "$SOURCE" rev-parse "refs/tags/$TAG^{commit}")"
test -z "$(git -C "$SOURCE" status --porcelain --untracked-files=no)"

PRIVATE=$(mktemp -d "$(dirname "$OUTPUT")/.quipu-publisher.XXXXXX")
trap 'rm -rf "$PRIVATE"' EXIT
bash "$SOURCE/scripts/build-repository-share.sh" \
    "$QUIPU_BIN" "$BOBBIN_BIN" "$SOURCE" "$PRIVATE/share" "$REVISION"
mkdir "$PRIVATE/artifacts"
PACK="quipu-${TAG}-repository.qpack.tar.gz"
tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 \
    --numeric-owner -C "$PRIVATE/share" -czf "$PRIVATE/artifacts/$PACK" .
(cd "$PRIVATE/artifacts" && sha256sum "$PACK" > "$PACK.sha256")
python3 - "$PRIVATE/artifacts/$PACK" "$QUIPU_BIN" "$REVISION" "$TAG" <<'PY'
import datetime
import hashlib
import json
from pathlib import Path
import sys

pack, binary = map(Path, sys.argv[1:3])
proof = {
    "schema": "quipu.repository-share-producer/v1",
    "source_repository": "scbrown/quipu",
    "source_revision": sys.argv[3],
    "release_tag": sys.argv[4],
    "archive_sha256": hashlib.sha256(pack.read_bytes()).hexdigest(),
    "quipu_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "destination": "outward",
    "verified_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
}
pack.with_name(pack.name + ".provenance.json").write_text(json.dumps(proof, indent=2) + "\n")
PY
mv -T "$PRIVATE/artifacts" "$OUTPUT"
if [[ "$MODE" == --publish ]]; then
    # Explicit filenames only: neither source/index nor catalogue nor private
    # diagnostics can enter the release upload. Never replace an existing pack.
    gh release upload "$TAG" --repo scbrown/quipu \
        "$OUTPUT/$PACK" "$OUTPUT/$PACK.sha256" "$OUTPUT/$PACK.provenance.json"
fi
