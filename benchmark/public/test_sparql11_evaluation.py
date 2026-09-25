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

    def test_parse_manifest_accepts_the_turtle_a_shorthand(self):
        # 56 approved update tests are declared with `a`, not `rdf:type`.
        with tempfile.TemporaryDirectory() as directory:
            manifest = pathlib.Path(directory) / "manifest.ttl"
            manifest.write_text('''
:d a mf:UpdateEvaluationTest ; mf:name "D" ; dawgt:approval dawgt:Approved;
    mf:action [ ut:request <d.ru> ; ut:data <pre.ttl> ] ; mf:result [ ut:data <post.ttl> ] .
:e rdf:type mf:UpdateEvaluationTest ; mf:name "E" ; dawgt:approval dawgt:Approved ;
    mf:action [ ut:request <e.ru> ] ; mf:result [ ] .
''')
            cases = MODULE.parse_manifest("update", manifest)
            self.assertEqual(sorted(c.identifier for c in cases), [":d", ":e"])

    def test_the_approved_inventory_is_pinned_per_class(self):
        self.assertEqual(MODULE.APPROVED_INVENTORY["update"], 93)
        self.assertEqual(set(MODULE.APPROVED_INVENTORY), set(MODULE.CLASS_MANIFESTS))

    def test_parse_manifest_includes_csv_result_format_cases(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = pathlib.Path(directory) / "manifest.ttl"
            manifest.write_text('''
:csv rdf:type mf:CSVResultFormatTest ; mf:name "CSV" ; dawgt:approval dawgt:Approved ;
   mf:action [ qt:query <a.rq> ; qt:data <data.ttl> ] ; mf:result <a.csv> .
''')
            cases = MODULE.parse_manifest("result-format", manifest)
            self.assertEqual([(case.identifier, case.kind) for case in cases], [(":csv", "CSVResultFormatTest")])

    def test_parse_protocol_preserves_request_order_and_expectations(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = pathlib.Path(directory) / "manifest.ttl"
            manifest.write_text('''
:p rdf:type mf:ProtocolTest ; mf:name "Protocol" ; dawgt:approval dawgt:Approved ;
 mf:action [ ht:requests ([ a ht:Request ; ht:absolutePath "/sparql/?query=ASK%20%7B%7D" ;
  ht:methodName "GET" ; ht:resp [ mf:expectedBoolean true ; mf:expectedFormat "boolean" ;
  mf:expectedStatus hts:StatusCode2xx ] ] [ a ht:Request ; ht:absolutePath "/sparql/" ;
  ht:methodName "PUT" ; ht:resp [ mf:expectedStatus hts:StatusCode4xx ] ]) ] .
''')
            case = MODULE.parse_manifest("protocol", manifest)[0]
            self.assertEqual([request.method for request in case.protocol_requests], ["GET", "PUT"])
            self.assertTrue(case.protocol_requests[0].expected_boolean)
            self.assertEqual(case.protocol_requests[1].status_family, 4)

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

    def test_delimited_result_preserves_csv_and_tsv_spelling(self):
        self.assertEqual(
            MODULE.delimited_result(b"s,o\r\na,\"x,y\"\r\n", ".csv"),
            (["s", "o"], [("a", "x,y")]),
        )
        self.assertEqual(MODULE.normalize_delimited_numeric("1.0E6"), "1.0e6")
        self.assertEqual(MODULE.normalize_delimited_numeric("notE10"), "notE10")
        self.assertEqual(
            MODULE.delimited_result(b"?s\t?o\n<a>\t_:b0\n", ".tsv"),
            (["s", "o"], [("<a>", "_:b0")]),
        )

    def test_update_cases_are_executable(self):
        case = MODULE.Case("update", pathlib.Path("m"), ":u", "u", "UpdateEvaluationTest", None, (), (), None)
        self.assertIsNone(MODULE.unsupported_reason(case))

    def test_executable_path_supports_path_lookup(self):
        with mock.patch.object(MODULE.shutil, "which", return_value="/opt/bin/quipu"):
            self.assertEqual(MODULE.executable_path(pathlib.Path("quipu")), pathlib.Path("/opt/bin/quipu"))


class NonCompletionIsNotAnEmptyResultTests(unittest.TestCase):
    """A query that did not COMPLETE must never be scored as a passing case.

    The published ledgers are produced by run_case, and the worst case is a
    case whose expected result is EMPTY: there, "the engine returned nothing"
    and "the engine never answered" compare equal unless the runner
    distinguishes them. aegis-41rc28.

    Measured 2026-09-16 against the installed CLI (quipu 0.6.0), so these
    stubs reproduce a real shape rather than an imagined one: a timed-out
    `quipu read` writes **0 bytes** to stdout (no partial table at 1/50/200 ms
    budgets), puts its message on stderr, and exits **2** -- 2 for did-not-
    complete, 1 for refused/malformed.
    """

    TIMEOUT_STDERR = (
        "query error: query timeout: exceeded 1ms (ran 1ms) - narrow the query "
        "or raise [quipu.search] query_timeout_ms"
    )
    # The SAME failure with the `query error:` literal absent. The runner's
    # protection must not rest on the wording of a message: rewording it is a
    # one-line change nobody would flag in review.
    REWORDED_STDERR = "the query budget was exhausted before the engine answered"

    def _case_expecting_no_rows(self, root):
        query = root / "q.rq"
        query.write_text("SELECT ?s WHERE { ?s ?p ?o }")
        result = root / "r.srj"
        result.write_text('{"head":{"vars":["s"]},"results":{"bindings":[]}}')
        return MODULE.Case(
            "evaluation", root / "manifest.ttl", ":timeout-case", "timeout case",
            "QueryEvaluationTest", query, (), (), result,
        )

    def _stub_quipu(self, root, stderr_text):
        """A stub CLI: `knot` succeeds, `read` does not complete.

        Written as a real file, never a symlink to the real binary -- a stub
        installed over a symlink writes THROUGH it and destroys the tool
        (aegis-ydrml).
        """
        stub = root / "quipu-stub"
        stub.write_text(
            "#!/usr/bin/env python3\n"
            "import sys\n"
            "if sys.argv[1] == 'read':\n"
            f"    sys.stderr.write({stderr_text!r})\n"
            "    sys.exit(2)\n"
            "sys.exit(0)\n"
        )
        stub.chmod(0o755)
        return stub

    def _run(self, stderr_text):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            return MODULE.run_case(
                self._case_expecting_no_rows(root),
                self._stub_quipu(root, stderr_text),
                root / "quipu-server-unused",
            )

    def test_a_timeout_is_not_scored_as_a_passing_empty_result(self):
        outcome = self._run(self.TIMEOUT_STDERR)
        self.assertNotEqual(outcome["status"], "passed")
        self.assertEqual(outcome["status"], "failed")

    def test_detection_does_not_depend_on_the_error_message_wording(self):
        # The pin. Without a returncode check this is scored from whatever the
        # stdout parser happens to do with 0 bytes, which is an accident rather
        # than a decision -- and an accident that changes if the renderer ever
        # emits an empty table instead of nothing.
        outcome = self._run(self.REWORDED_STDERR)
        self.assertNotEqual(outcome["status"], "passed")
        self.assertEqual(outcome["status"], "failed")

    def test_a_completing_query_is_still_judged_on_its_results(self):
        # The negative control. A guard that refused everything would satisfy
        # both assertions above while making the whole suite unfalsifiable.
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            stub = root / "quipu-stub"
            stub.write_text(
                "#!/usr/bin/env python3\n"
                "import sys\n"
                "if sys.argv[1] == 'read':\n"
                "    sys.stdout.write('s\\n---\\n0 results\\n')\n"
                "sys.exit(0)\n"
            )
            stub.chmod(0o755)
            outcome = MODULE.run_case(
                self._case_expecting_no_rows(root), stub, root / "unused"
            )
        self.assertEqual(outcome["status"], "passed")


if __name__ == "__main__":
    unittest.main()
