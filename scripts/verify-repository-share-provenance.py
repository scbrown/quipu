"""Verify the trusted producer's release binding without receiving its policy."""

import hashlib
import json
from pathlib import Path
import sys


def verify(pack, binary, revision, tag):
    proof = json.loads(pack.with_name(pack.name + ".provenance.json").read_text())
    expected = {
        "schema": "quipu.repository-share-producer/v1",
        "source_repository": "scbrown/quipu",
        "source_revision": revision,
        "release_tag": tag,
        "archive_sha256": hashlib.sha256(pack.read_bytes()).hexdigest(),
        "quipu_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "destination": "outward",
    }
    for key, value in expected.items():
        if proof.get(key) != value:
            raise ValueError(f"repository share provenance mismatch: {key}")
    expected_checksum = f"{expected['archive_sha256']}  {pack.name}\n"
    if pack.with_name(pack.name + ".sha256").read_text() != expected_checksum:
        raise ValueError("repository share checksum mismatch")


if __name__ == "__main__":
    if len(sys.argv) != 5:
        sys.exit("usage: verify-repository-share-provenance.py <pack> <quipu-bin> <revision> <tag>")
    verify(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4])
