"""Deterministic CI checks: metric math, isolation, malformed responses and drift."""
import importlib.util
import math
import json
from unittest.mock import patch
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("search_eval", Path(__file__).with_name("search-eval.py"))
eval = importlib.util.module_from_spec(spec)
spec.loader.exec_module(eval)


def hits(*names):
    return [{"entity": name, "score": 0.5} for name in names]


class EvaluationTests(unittest.TestCase):
    def test_five_class_fixture_replay(self):
        root = Path(__file__).parent.parent / "tests/fixtures/search-eval"
        suite = json.loads((root / "suite.json").read_text())
        captured = json.loads((root / "responses.json").read_text())
        def replay(endpoint, path, payload):
            self.assertEqual(path, "/search")
            self.assertEqual(payload["limit"], 20)
            return captured[payload["query"]], 10.0
        with patch.object(eval, "request", side_effect=replay) as mock:
            rows = eval.evaluate(suite, "http://127.0.0.1:1", 2)
        self.assertEqual(mock.call_count, 15)
        summary = eval.summarize(rows)
        self.assertEqual(summary["queries"], 5)
        self.assertEqual(summary["mrr@20"], .5)
        self.assertEqual(summary["recall@20"], 1)
        self.assertEqual(summary["latency_p95_ms"], 10)

    def test_request_error_is_not_an_empty_ranking(self):
        root = Path(__file__).parent.parent / "tests/fixtures/search-eval"
        suite = json.loads((root / "suite.json").read_text())
        with patch.object(eval, "request", side_effect=ValueError("HTTP error")):
            with self.assertRaises(ValueError):
                eval.evaluate(suite, "http://127.0.0.1:1", 1)

    def test_known_ranking(self):
        actual = eval.metrics(hits("miss", "a", "b"), {"a": 3, "b": 1, "c": 2})
        ideal = 7 + 3 / math.log2(3) + 1 / math.log2(4)
        self.assertAlmostEqual(actual["ndcg@10"], (7 / math.log2(3) + .5) / ideal)
        self.assertEqual(actual["recall@20"], 2 / 3)
        self.assertEqual(actual["mrr@20"], .5)

    def test_perfect_empty_and_cutoffs(self):
        self.assertEqual(eval.metrics(hits("a"), {"a": 3})["ndcg@10"], 1)
        self.assertEqual(eval.metrics([], {"a": 3})["mrr@20"], 0)
        result = eval.metrics(hits(*(str(i) for i in range(20)), "a"), {"a": 3})
        self.assertEqual(result["recall@20"], 0)
        self.assertEqual(result["mrr@20"], 0)

    def test_prefix_and_duplicate(self):
        self.assertEqual(eval.metrics(hits("ex:a"), {"https://example.org/a": 1},
                                      {"ex": "https://example.org/"})["recall@20"], 1)
        with self.assertRaises(ValueError):
            eval.metrics(hits("a", "a"), {"a": 1})
        with self.assertRaises(ValueError):
            eval.metrics([{"entity": "a", "score": float("nan")}], {"a": 1})

    def test_endpoint_isolation(self):
        for good in ("http://127.0.0.1:3031", "http://[::1]:3031"):
            self.assertEqual(eval.local_endpoint(good), good)
        for bad in ("http://example.org", "http://localhost", "http://127.0.0.1/path",
                    "https://127.0.0.1", "http://user@127.0.0.1", "http://127.0.0.1?q=x"):
            with self.assertRaises(ValueError):
                eval.local_endpoint(bad)

    def test_schema_rejects_vacuous_suite(self):
        for queries in ([], [{"id": "x"}, {"id": "x"}]):
            with self.assertRaises(ValueError):
                eval.validate_suite({"schema_version": 1, "queries": queries})

    def test_nearest_rank_percentiles(self):
        self.assertEqual(eval.percentile(list(range(1, 101)), .95), 95)
        self.assertEqual(eval.percentile([5], .5), 5)

    def test_comparison_requires_frozen_inputs(self):
        a = {"suite_sha256": "s", "corpus_sha256": "c", "rows": []}
        eval.compare(a, a)
        with self.assertRaises(ValueError):
            eval.compare(a, {**a, "corpus_sha256": "other"})
        row = {"id": "q", "request": {"query": "x"}, "results": hits("a")}
        with self.assertRaises(ValueError):
            eval.compare({**a, "rows": [row]}, {**a, "rows": [{**row, "results": hits("b")}]})


if __name__ == "__main__":
    unittest.main()
