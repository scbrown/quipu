"""Native query output must not turn incomplete answers into empty success."""

import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "receiver", Path(__file__).with_name("verify-repository-share-receiver.py"),
)
RECEIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RECEIVER)


class NativeReadTest(unittest.TestCase):
    def read(self, output):
        result = subprocess.CompletedProcess([], 0, output, "")
        with patch.object(RECEIVER, "run", return_value=result):
            return RECEIVER.iri_rows("fixture", "scratch.db", "SELECT ?iri WHERE {}")

    def test_complete_positive_and_empty_tables(self):
        self.assertEqual(self.read('iri\n----\n"urn:one"\n\n1 results\n'), ["urn:one"])
        self.assertEqual(self.read("iri\n----\n\n0 results\n"), [])

    def test_truncation_wrong_protocol_and_count_skew_refuse(self):
        for output in ('iri\n----\n"urn:one"', '{"rows":[]}',
                       'iri\n----\n"urn:one"\n\n0 results\n',
                       'iri\n----\n\n1 results\n'):
            with self.subTest(output=output), self.assertRaises(RuntimeError):
                self.read(output)


if __name__ == "__main__":
    unittest.main()
