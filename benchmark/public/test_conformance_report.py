import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import unittest


def _load(name):
    path = pathlib.Path(__file__).with_name(f"{name}.py")
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


REPORT = _load("conformance_report")
REGRESSION = _load("check_regression")

REPO = pathlib.Path(__file__).resolve().parents[2]
RESULTS = REPO / "benchmark" / "public" / "results"
DOCS = REPO / "docs" / "book" / "src" / "benchmarks"


def evaluation_row(identifier, status, test_class="query-evaluation", manifest="aggregates/manifest.ttl"):
    row = {
        "class": test_class,
        "id": identifier,
        "name": identifier,
        "manifest": manifest,
        "query": f"{identifier}.rq",
        "result": f"{identifier}.srx",
        "status": status,
    }
    row["reason" if status == "unsupported" else "diagnostic"] = "because"
    return row


def ledger(rows):
    classes = {}
    for row in rows:
        classes.setdefault(row["class"], {})
    return {
        "suite_revision": "369a90d1",
        "quipu_revision": "abc1234",
        "quipu_version": "quipu 0.0.0",
        "isolation": "one temporary store per test",
        "reproduce": {"build": "cargo build", "environment": {"SUITE": "/tmp/s"}},
        "classes": classes,
        "results": rows,
    }


class BadgeTests(unittest.TestCase):
    def test_unsupported_only_class_is_grey_not_red(self):
        # 0/34 in red would claim we tried and failed; grey says not implemented.
        counts = REPORT.tally([evaluation_row(":p1", "unsupported", "protocol")])
        self.assertEqual(REPORT.badge_color(counts), "lightgrey")

    def test_colour_tracks_the_honest_denominator(self):
        # Unsupported cases stay in the denominator, so a class cannot go green
        # by declaring its hard tests out of scope.
        rows = [evaluation_row(f":t{index}", "passed") for index in range(9)]
        rows.append(evaluation_row(":u", "unsupported"))
        counts = REPORT.tally(rows)
        self.assertEqual(counts["passed"], 9)
        self.assertEqual(counts["cases"], 10)
        self.assertEqual(REPORT.badge_color(counts), "yellow")

    def test_badge_is_a_valid_shields_endpoint(self):
        counts = REPORT.tally([evaluation_row(":a", "passed"), evaluation_row(":b", "failed")])
        payload = REPORT.badge("query-evaluation", counts)
        self.assertEqual(payload["schemaVersion"], 1)
        self.assertEqual(payload["message"], "1/2")
        self.assertEqual(payload["color"], "orange")


class LedgerShapeTests(unittest.TestCase):
    def test_unsupported_rows_carry_reason_executed_rows_carry_diagnostic(self):
        self.assertEqual(REPORT.reason_of(evaluation_row(":u", "unsupported")), "because")
        self.assertEqual(REPORT.reason_of(evaluation_row(":f", "failed")), "because")

    def test_family_comes_from_the_pinned_suite_layout(self):
        self.assertEqual(REPORT.family_of({"manifest": "property-path/manifest.ttl"}), "property-path")
        self.assertEqual(REPORT.family_of({"manifest": "manifest.ttl"}), "(root)")

    def test_unknown_status_is_refused_rather_than_silently_dropped(self):
        with self.assertRaises(REPORT.LedgerError):
            REPORT.tally([evaluation_row(":x", "skipped")])

    def test_a_new_class_must_be_placed_deliberately(self):
        with tempfile.TemporaryDirectory() as directory:
            results = pathlib.Path(directory)
            (results / REPORT.SYNTAX_LEDGER).write_text(json.dumps({
                "quipu_revision": "abc",
                "results": [{"test": "a.rq", "passed": True}],
            }))
            (results / REPORT.EVALUATION_LEDGER).write_text(
                json.dumps(ledger([evaluation_row(":n", "passed", "brand-new-class")]))
            )
            (results / REPORT.ENTAILMENT_LEDGER).write_text(json.dumps(ledger([
                evaluation_row(f":e{index}", "unsupported", "entailment") for index in range(70)
            ])))
            (results / REPORT.SHACL_LEDGER).write_text(json.dumps({"results": []}))
            (results / REPORT.FEDERATED_LEDGER).write_text(json.dumps(ledger([
                evaluation_row(f":service{index}", "unsupported", "federated-query", "service/manifest.ttl")
                for index in range(7)
            ])))
            with self.assertRaises(REPORT.LedgerError) as caught:
                REPORT.load(results)
            self.assertIn("brand-new-class", str(caught.exception))


class RegressionGateTests(unittest.TestCase):
    def _write(self, directory, name, rows):
        path = pathlib.Path(directory) / name
        path.write_text(json.dumps(ledger(rows)))
        return path

    def test_still_failing_is_not_a_regression(self):
        # The whole point: 18/168 must exit 0 until it gets worse, or the gate
        # is red on every commit and stops being read.
        rows = [evaluation_row(":a", "passed"), evaluation_row(":b", "failed")]
        result = REGRESSION.compare(
            {(r["class"], r["manifest"], r["id"]): r for r in rows},
            {(r["class"], r["manifest"], r["id"]): r for r in rows},
        )
        self.assertEqual(result["regressed"], [])
        self.assertEqual(result["class_drops"], [])

    def test_a_test_that_stops_passing_is_named(self):
        base = {("query-evaluation", "m", ":a"): evaluation_row(":a", "passed")}
        cand = {("query-evaluation", "m", ":a"): evaluation_row(":a", "failed")}
        result = REGRESSION.compare(base, cand)
        self.assertEqual(result["regressed"], [("query-evaluation", "m", ":a")])
        self.assertEqual(result["class_drops"], ["query-evaluation"])

    def test_a_disappearing_test_is_a_regression(self):
        # Deleting a failing test raises every ratio for free; refuse it.
        base = {
            ("query-evaluation", "m", ":a"): evaluation_row(":a", "passed"),
            ("query-evaluation", "m", ":b"): evaluation_row(":b", "failed"),
        }
        cand = {("query-evaluation", "m", ":a"): evaluation_row(":a", "passed")}
        result = REGRESSION.compare(base, cand)
        self.assertEqual(result["dropped"], [("query-evaluation", "m", ":b")])
        self.assertEqual(result["regressed"], [])

    def test_improvement_exits_zero_and_asks_for_a_refreshed_baseline(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline = self._write(directory, "base.json", [evaluation_row(":a", "failed")])
            candidate = self._write(directory, "cand.json", [evaluation_row(":a", "passed")])
            captured = io.StringIO()
            stdout = sys.stdout
            sys.stdout = captured
            try:
                code = REGRESSION.main(["--baseline", str(baseline), "--candidate", str(candidate)])
            finally:
                sys.stdout = stdout
            self.assertEqual(code, 0)
            self.assertIn("regenerate", captured.getvalue().lower())

    def test_regression_exits_one(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline = self._write(directory, "base.json", [evaluation_row(":a", "passed")])
            candidate = self._write(directory, "cand.json", [evaluation_row(":a", "failed")])
            stdout, stderr = sys.stdout, sys.stderr
            sys.stdout = sys.stderr = io.StringIO()
            try:
                code = REGRESSION.main(["--baseline", str(baseline), "--candidate", str(candidate)])
            finally:
                sys.stdout, sys.stderr = stdout, stderr
            self.assertEqual(code, 1)

    def test_a_malformed_ledger_exits_two_not_one(self):
        # 2 is "could not measure"; conflating it with 1 turns a broken harness
        # into a reported regression.
        with tempfile.TemporaryDirectory() as directory:
            good = self._write(directory, "base.json", [evaluation_row(":a", "passed")])
            bad = pathlib.Path(directory) / "bad.json"
            bad.write_text("{not json")
            stdout, stderr = sys.stdout, sys.stderr
            sys.stdout = sys.stderr = io.StringIO()
            try:
                code = REGRESSION.main(["--baseline", str(good), "--candidate", str(bad)])
            finally:
                sys.stdout, sys.stderr = stdout, stderr
            self.assertEqual(code, 2)

    def test_duplicate_rows_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write(
                directory, "dupe.json", [evaluation_row(":a", "passed"), evaluation_row(":a", "failed")]
            )
            with self.assertRaises(REGRESSION.LedgerError):
                REGRESSION.load_rows(path)


class PublishedArtifactsTests(unittest.TestCase):
    """The committed page and badges must match the committed ledgers."""

    def test_check_mode_passes_against_what_is_committed(self):
        code = REPORT.main(["--results-dir", str(RESULTS), "--docs-dir", str(DOCS), "--check"])
        self.assertEqual(code, 0, "run: python3 benchmark/public/conformance_report.py")

    def test_the_page_states_the_claim_boundary_and_the_real_numbers(self):
        data = REPORT.load(RESULTS)
        page = REPORT.render_markdown(data)
        evaluation = data["classes"]["query-evaluation"]["counts"]
        self.assertIn("not a conformant SPARQL 1.1 implementation", page)
        self.assertIn(f"{evaluation['passed']}/{evaluation['cases']}", page)
        self.assertNotIn("%", page.split("## Full ledgers")[0])


if __name__ == "__main__":
    unittest.main()


class LedgerProvenanceTest(unittest.TestCase):
    """`ledger_provenance` — is a ledger derived from the code it ships with?

    Three outcomes, and the third is the point: a pass/fail verdict is forced to
    render "I could not look" as "nothing was wrong", which is the direction
    that lets drift through a shallow clone.
    """

    def _repo(self, stack):
        import subprocess

        root = pathlib.Path(stack.enter_context(tempfile.TemporaryDirectory()))
        run = lambda *a: subprocess.run(  # noqa: E731
            ["git", *a], cwd=root, check=True, capture_output=True, text=True
        )
        run("init", "-q", "-b", "main")
        run("config", "user.email", "t@example.com")
        run("config", "user.name", "t")
        (root / "src").mkdir()
        (root / "src" / "lib.rs").write_text("// v1\n")
        run("add", "-A")
        run("commit", "-qm", "one")
        first = run("rev-parse", "HEAD").stdout.strip()
        return root, run, first

    def _provenance(self, root, data):
        """Run the real function against `root` by pointing its git calls there."""
        original = REPORT._git
        import subprocess

        def fake(*args):
            proc = subprocess.run(
                ["git", *args], cwd=root, capture_output=True, text=True, check=False
            )
            return proc.returncode, proc.stdout.strip()

        REPORT._git = fake
        try:
            return REPORT.ledger_provenance(data)
        finally:
            REPORT._git = original

    def test_revision_equal_to_head_is_clean(self):
        import contextlib

        with contextlib.ExitStack() as stack:
            root, _run, first = self._repo(stack)
            code, messages = self._provenance(root, {"quipu_revision": first})
            self.assertEqual((code, messages), (0, []))

    def test_docs_only_drift_is_clean(self):
        # A ledger stays valid across a commit that cannot move a number.
        import contextlib

        with contextlib.ExitStack() as stack:
            root, run, first = self._repo(stack)
            (root / "README.md").write_text("docs\n")
            run("add", "-A")
            run("commit", "-qm", "docs only")
            code, messages = self._provenance(root, {"quipu_revision": first})
            self.assertEqual(code, 0, messages)

    def test_code_drift_is_detected(self):
        import contextlib

        with contextlib.ExitStack() as stack:
            root, run, first = self._repo(stack)
            (root / "src" / "lib.rs").write_text("// v2\n")
            run("add", "-A")
            run("commit", "-qm", "code change")
            code, messages = self._provenance(root, {"quipu_revision": first})
            self.assertEqual(code, 1, messages)
            self.assertIn("src/lib.rs", " ".join(messages))

    def test_absent_revision_is_UNVERIFIED_not_clean(self):
        # The shallow-clone case. A revision this checkout has never heard of
        # must NOT read as "no drift".
        import contextlib

        with contextlib.ExitStack() as stack:
            root, _run, _first = self._repo(stack)
            code, messages = self._provenance(root, {"quipu_revision": "0" * 40})
            self.assertEqual(code, 2, messages)
            self.assertIn("UNVERIFIED", " ".join(messages))

    def test_no_revision_at_all_is_UNVERIFIED_not_clean(self):
        import contextlib

        with contextlib.ExitStack() as stack:
            root, _run, _first = self._repo(stack)
            code, messages = self._provenance(root, {"suite_revision": "abc"})
            self.assertEqual(code, 2, messages)


class LedgerProvenancePrModeTest(unittest.TestCase):
    """PR mode: only THIS PR's own changes can stale its ledger (aegis-1gp76j).

    The strict arm asks "is this ledger derived from the code it ships beside?"
    and belongs on main. On a PR it accidentally asks "has main moved?", which
    charges a full re-derive for somebody else's merge. Both arms are tested
    here because a green from one is not a green from the other — which is why
    the arm is printed.
    """

    def _repo(self, stack):
        import subprocess

        root = pathlib.Path(stack.enter_context(tempfile.TemporaryDirectory()))
        run = lambda *a: subprocess.run(  # noqa: E731
            ["git", *a], cwd=root, check=True, capture_output=True, text=True
        )
        run("init", "-q", "-b", "main")
        run("config", "user.email", "t@example.com")
        run("config", "user.name", "t")
        (root / "src").mkdir()
        (root / "src" / "mine.rs").write_text("// v1\n")
        (root / "src" / "theirs.rs").write_text("// v1\n")
        run("add", "-A")
        run("commit", "-qm", "base")
        base = run("rev-parse", "HEAD").stdout.strip()
        return root, run, base

    def _provenance(self, root, data, pr_base=None):
        original = REPORT._git
        import subprocess

        def fake(*args):
            p = subprocess.run(["git", *args], cwd=root, capture_output=True, text=True, check=False)
            return p.returncode, p.stdout.strip()

        REPORT._git = fake
        try:
            return REPORT.ledger_provenance(data, pr_base)
        finally:
            REPORT._git = original

    def _pr_over_someone_elses_merge(self, stack):
        """A PR that changed NOTHING relevant, on a main that moved."""
        root, run, base = self._repo(stack)
        run("checkout", "-qb", "pr")
        (root / "README.md").write_text("docs\n")
        run("add", "-A")
        run("commit", "-qm", "pr: docs only")
        run("checkout", "-q", "main")
        (root / "src" / "theirs.rs").write_text("// v2 — somebody else\n")
        run("add", "-A")
        run("commit", "-qm", "main: unrelated src change")
        run("checkout", "-q", "pr")
        run("merge", "-q", "--no-edit", "main")
        return root, run, base

    def test_pr_mode_ignores_a_src_change_that_came_from_main(self):
        import contextlib

        with contextlib.ExitStack() as stack:
            root, _run, base = self._pr_over_someone_elses_merge(stack)
            code, messages = self._provenance(root, {"quipu_revision": base}, "main")
            self.assertEqual(code, 0, messages)

    def test_STRICT_mode_still_fails_on_that_same_tree(self):
        # The discriminating control. Same repo, same ledger, no pr_base: the
        # strict arm MUST still object, or PR mode has not narrowed anything and
        # the first test would pass for the wrong reason.
        import contextlib

        with contextlib.ExitStack() as stack:
            root, _run, base = self._pr_over_someone_elses_merge(stack)
            code, messages = self._provenance(root, {"quipu_revision": base})
            self.assertEqual(code, 1, messages)
            self.assertIn("theirs.rs", " ".join(messages))

    def test_pr_mode_STILL_fails_when_the_PR_ITSELF_touches_src(self):
        # wu's condition 3: a PR touching src re-derives, always.
        import contextlib

        with contextlib.ExitStack() as stack:
            root, run, base = self._repo(stack)
            run("checkout", "-qb", "pr")
            (root / "src" / "mine.rs").write_text("// v2 — mine\n")
            run("add", "-A")
            run("commit", "-qm", "pr: my own src change")
            code, messages = self._provenance(root, {"quipu_revision": base}, "main")
            self.assertEqual(code, 1, messages)
            self.assertIn("mine.rs", " ".join(messages))

    def test_pr_mode_with_an_unresolvable_base_is_UNVERIFIED(self):
        import contextlib

        with contextlib.ExitStack() as stack:
            root, _run, base = self._repo(stack)
            code, messages = self._provenance(root, {"quipu_revision": base}, "no-such-ref")
            self.assertEqual(code, 2, messages)
            self.assertIn("UNVERIFIED", " ".join(messages))
