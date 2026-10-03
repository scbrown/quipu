import importlib.util
import pathlib
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))
SPEC = importlib.util.spec_from_file_location("sparql12", HERE / "sparql12.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)

MANIFEST = """
:manifest rdf:type mf:Manifest ;
    mf:entries ( :a :b ) .
:a rdf:type mf:QueryEvaluationTest ; mf:name "a" .
## :c rdf:type mf:QueryEvaluationTest .
:b rdf:type mf:NegativeSyntaxTest ; mf:name "b" .
"""


class EnumerationTests(unittest.TestCase):
    def write(self, text):
        path = pathlib.Path(tempfile.mkdtemp()) / "manifest.ttl"
        path.write_text(text)
        return path

    def test_cases_match_entries_and_skip_the_manifest_and_comments(self):
        cases = MODULE.enumerate_cases(self.write(MANIFEST))
        self.assertEqual(cases, [(":a", "QueryEvaluationTest"), (":b", "NegativeSyntaxTest")])

    def test_a_mismatch_with_entries_refuses(self):
        with self.assertRaises(ValueError):
            MODULE.enumerate_cases(self.write(MANIFEST.replace("( :a :b )", "( :a )")))

    def test_every_not_run_reason_is_named(self):
        self.assertTrue(all(MODULE.NOT_RUN.values()))
        self.assertTrue(all(MODULE.NOT_RUN_CASES.values()))


if __name__ == "__main__":
    unittest.main()
