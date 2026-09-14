#!/usr/bin/env bash
# Stage the committed text share and shared renderer, without Rust or a server.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
DEST=docs/book/src/datalinks
mkdir -p "$DEST/vendor"
cp ui/datalinks.js ui/graph-canvas.js "$DEST/"
cp ui/vendor/three.module.min.js "$DEST/vendor/"
# Only transport packaging: the canonical manifest and payload bytes are unchanged.
# Explicit names keep unrelated authoring files out of the downloadable pack.
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
    -C "$DEST/qpack" -cf - export.nt manifest.json manifest.ttl shapes.ttl | gzip -n > "$DEST/demo.qpack.tar.gz"
