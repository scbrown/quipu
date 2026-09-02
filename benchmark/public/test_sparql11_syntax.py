import importlib.util
import pathlib
import unittest

MODULE_PATH = pathlib.Path(__file__).with_name("sparql11_syntax.py")
SPEC = importlib.util.spec_from_file_location("sparql11_syntax", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)


class ManifestTests(unittest.TestCase):
    def test_extracts_only_approved_query_syntax(self):
        manifest = """
:a rdf:type mf:PositiveSyntaxTest11 ; dawgt:approval dawgt:Approved ; mf:action <a.rq> ;.
:b rdf:type mf:NegativeSyntaxTest11 ; dawgt:approval dawgt:Approved ; mf:action <b.rq> ;.
:c rdf:type mf:PositiveSyntaxTest11 ; dawgt:approval dawgt:NotClassified ; mf:action <c.rq> ;.
:d rdf:type mf:QueryEvaluationTest ; dawgt:approval dawgt:Approved ; mf:action <d.rq> ;.
"""
        self.assertEqual(MODULE.approved_cases(manifest), [("a.rq", True), ("b.rq", False)])


if __name__ == "__main__":
    unittest.main()
