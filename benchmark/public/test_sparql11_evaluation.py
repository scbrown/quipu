import importlib.util
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

MODULE_PATH = pathlib.Path(__file__).with_name("sparql11_evaluation.py")
SPEC = importlib.util.spec_from_file_location("sparql11_evaluation", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class EvaluationManifestTests(unittest.TestCase):
    def test_statement_split_ignores_dots_in_nested_actions_and_strings(self):
        text = '''
:a rdf:type mf:QueryEvaluationTest ; mf:name "A. test" ;
   dawgt:approval dawgt:Approved ;
   mf:action [ qt:query <a.rq> ; qt:data <data.ttl> ] ; mf:result <a.srx> .
:b rdf:type mf:QueryEvaluationTest ; dawgt:approval dawgt:NotClassified .
'''
        statements = MODULE.turtle_statements(text)
        self.assertEqual(len(statements), 2)
        self.assertIn("data.ttl", statements[0])

    def test_parse_manifest_selects_only_approved_cases(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = pathlib.Path(directory) / "manifest.ttl"
            manifest.write_text('''
:a rdf:type mf:QueryEvaluationTest ; mf:name "A" ; dawgt:approval dawgt:Approved ;
   mf:action [ qt:query <a.rq> ; qt:data <data.ttl> ] ; mf:result <a.srx> .
:b rdf:type mf:QueryEvaluationTest ; dawgt:approval dawgt:NotClassified .
''')
            cases = MODULE.parse_manifest("query-evaluation", manifest)
            self.assertEqual(len(cases), 1)
            self.assertEqual(cases[0].query, manifest.parent / "a.rq")
            self.assertEqual(cases[0].data, (manifest.parent / "data.ttl",))

    def test_actual_result_parses_cli_table_and_boolean(self):
        self.assertEqual(MODULE.actual_result("true\n"), True)
        self.assertEqual(
            MODULE.actual_result("x\ty\n----------------------------------------\n<a>\t1\n\n1 results\n"),
            (["x", "y"], [("<a>", "1")]),
        )
        self.assertEqual(MODULE.actual_result("\n\n\n1 results\n"), ([], [()]))

    def test_actual_graph_parses_cli_triples_and_checks_count(self):
        self.assertEqual(
            MODULE.actual_graph("s\tp\to\n\n1 triples\n"),
            MODULE.Counter({("s", "p", "o"): 1}),
        )
        with self.assertRaisesRegex(ValueError, "count"):
            MODULE.actual_graph("s\tp\to\n\n2 triples\n")

    def test_expected_uri_matches_the_cli_reference_rendering(self):
        self.assertEqual(MODULE.term("uri", "http://example.test/resource"), "http://example.test/resource")

    def test_result_columns_are_compared_by_variable_name(self):
        self.assertEqual(
            MODULE.reorder_rows(["o", "s"], [("value", "subject")], ["s", "o"]),
            [("subject", "value")],
        )
        self.assertIsNone(MODULE.reorder_rows(["s"], [("subject",)], ["s", "o"]))

    def test_blank_node_results_compare_by_global_bijection(self):
        actual = [("x", "_:generated-a", "_:generated-a"), ("y", "_:generated-b", "_:generated-c")]
        expected = [("y", "_:e2", "_:e3"), ("x", "_:e1", "_:e1")]
        self.assertTrue(MODULE.rows_equal_with_blank_nodes(actual, expected))
        self.assertFalse(
            MODULE.rows_equal_with_blank_nodes(
                [("x", "_:same", "_:same")], [("x", "_:left", "_:right")]
            )
        )

    def test_class_specific_unsupported_reasons_are_explicit(self):
        case = MODULE.Case("update", pathlib.Path("m"), ":u", "u", "UpdateEvaluationTest", None, (), (), None)
        self.assertIn("SPARQL Update", MODULE.unsupported_reason(case))

    def test_executable_path_supports_path_lookup(self):
        with mock.patch.object(MODULE.shutil, "which", return_value="/opt/bin/quipu"):
            self.assertEqual(MODULE.executable_path(pathlib.Path("quipu")), pathlib.Path("/opt/bin/quipu"))


if __name__ == "__main__":
    unittest.main()
