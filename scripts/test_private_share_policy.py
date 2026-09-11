"""Exercise the private policy boundary with a real HTTP authority stub."""

import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest


SCRIPTS = Path(__file__).resolve().parent


class Authority(http.server.BaseHTTPRequestHandler):
    document = {}
    status = 200

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        assert "GRAPH ?catalogue" in body["query"]
        self.send_response(self.status)
        self.end_headers()
        self.wfile.write(json.dumps(self.document).encode())

    def log_message(self, *_args):
        pass


class PrivatePolicyTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Authority)
        self.thread = threading.Thread(target=self.server.serve_forever)
        self.thread.start()
        self.env = dict(os.environ)
        self.env.pop("QUIPU_POLICY_TOKEN_FILE", None)
        self.env["QUIPU_POLICY_SERVER"] = f"http://127.0.0.1:{self.server.server_port}"
        Authority.status = 200
        self.rule("FIRST_PRIVATE_RULE")

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.temp.cleanup()

    def rule(self, pattern):
        Authority.document = {
            "count": 1,
            "truncated": False,
            "rows": [{"iri": "urn:policy:one", "label": "private", "regex": pattern}],
        }

    def run_projection(self, filename="policy.ttl"):
        output = self.root / filename
        result = subprocess.run(
            ["python3", str(SCRIPTS / "private-share-policy.py"), str(output)],
            env=self.env, capture_output=True, text=True, check=False,
        )
        return result, output

    def test_each_build_fetches_current_private_policy(self):
        result, first = self.run_projection()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(first.stat().st_mode & 0o777, 0o600)
        self.assertIn("FIRST_PRIVATE_RULE", first.read_text())
        self.rule("SECOND_PRIVATE_RULE")
        result, second = self.run_projection("second.ttl")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("SECOND_PRIVATE_RULE", second.read_text())
        self.assertNotIn("FIRST_PRIVATE_RULE", second.read_text())
        self.assertNotIn("SECOND_PRIVATE_RULE", result.stdout + result.stderr)

    def test_empty_truncated_and_partial_catalogues_refuse_without_output(self):
        for index, document in enumerate((
            {"count": 0, "rows": [], "truncated": False},
            {"count": 1, "rows": Authority.document["rows"], "truncated": True},
            {"count": 2, "rows": Authority.document["rows"], "truncated": False},
            {"count": 1, "rows": [{"iri": "urn:policy:one", "label": "private"}], "truncated": False},
        )):
            with self.subTest(document=document):
                Authority.document = document
                result, output = self.run_projection(f"refused-{index}.ttl")
                self.assertEqual(result.returncode, 2)
                self.assertFalse(output.exists())

    def test_authority_failure_cannot_reuse_an_existing_catalogue(self):
        result, output = self.run_projection()
        self.assertEqual(result.returncode, 0)
        before = output.read_bytes()
        Authority.status = 503
        result, output = self.run_projection()
        self.assertEqual(result.returncode, 2)
        self.assertEqual(output.read_bytes(), before)
        self.assertNotIn(self.env["QUIPU_POLICY_SERVER"], result.stderr)

    def test_builder_refuses_before_indexing_when_authority_is_unavailable(self):
        # This is the real consumer, not a test-only copy of its precondition.
        Authority.status = 503
        marker = self.root / "index-ran"
        indexer = self.root / "bobbin"
        indexer.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
        indexer.chmod(0o700)
        output = self.root / "share"
        result = subprocess.run(
            ["bash", str(SCRIPTS / "build-repository-share.sh"), "/bin/true",
             str(indexer), str(SCRIPTS.parent), str(output), "0" * 40],
            env=self.env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertFalse(marker.exists())
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
