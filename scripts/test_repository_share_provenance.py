"""A changed archive, binary, source revision or destination must be refused."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify-repository-share-provenance.py")
SPEC = importlib.util.spec_from_file_location("share_provenance", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ProvenanceTest(unittest.TestCase):
    def test_each_release_binding_is_load_bearing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack, binary = root / "share.tar.gz", root / "quipu"
            pack.write_bytes(b"verified archive")
            binary.write_bytes(b"released binary")
            digest = hashlib.sha256(pack.read_bytes()).hexdigest()
            proof = {
                "schema": "quipu.repository-share-producer/v1",
                "source_repository": "scbrown/quipu",
                "source_revision": "a" * 40,
                "release_tag": "quipu-ai-v1.0.0",
                "archive_sha256": digest,
                "quipu_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "destination": "outward",
            }
            proof_path = root / "share.tar.gz.provenance.json"
            proof_path.write_text(json.dumps(proof))
            (root / "share.tar.gz.sha256").write_text(f"{digest}  {pack.name}\n")
            args = (pack, binary, proof["source_revision"], proof["release_tag"])
            MODULE.verify(*args)
            for field in proof:
                with self.subTest(field=field):
                    changed = dict(proof, **{field: "wrong"})
                    proof_path.write_text(json.dumps(changed))
                    with self.assertRaises(ValueError):
                        MODULE.verify(*args)
            proof_path.write_text(json.dumps(proof))
            for path in (pack, binary):
                with self.subTest(bytes=path.name):
                    original = path.read_bytes()
                    path.write_bytes(b"replacement")
                    with self.assertRaises(ValueError):
                        MODULE.verify(*args)
                    path.write_bytes(original)


if __name__ == "__main__":
    unittest.main()
