import importlib.util
import io
import json
import pathlib
import subprocess
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
    def test_multiline_build_banner_keeps_the_version_table_cell_on_one_line(self):
        import shutil

        with tempfile.TemporaryDirectory() as directory:
            results = pathlib.Path(directory)
            for source in RESULTS.glob("*.json"):
                shutil.copyfile(source, results / source.name)
            path = results / REPORT.EVALUATION_LEDGER
            evaluation = json.loads(path.read_text())
            evaluation["quipu_version"] = "quipu 0.5.1\ngit_sha: abc1234"
            path.write_text(json.dumps(evaluation))
            data = REPORT.load(results)
            self.assertEqual(data["quipu_version"], "quipu 0.5.1")
            page = REPORT.render_markdown(data)
            self.assertIn("| Quipu version | `quipu 0.5.1` |", page)
            self.assertNotIn("git_sha: abc1234", page)

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
        # `--arm content`, deliberately: this class asserts the CONTENT
        # guarantee its own docstring states. The default arm also asks about
        # provenance, so with it this test reds whenever the branch's ledger
        # stamp is stale — a different guarantee, a different remedy, and it
        # would drag this unit test (which runs in the CONTENT job) red for a
        # PROVENANCE reason, defeating the whole point of the split
        # (aegis-fn3hdn). Provenance has its own job and its own test.
        code = REPORT.main(
            ["--results-dir", str(RESULTS), "--docs-dir", str(DOCS), "--check", "--arm", "content"]
        )
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


class SyntaxSuiteRegressionGateTest(unittest.TestCase):
    """The FIFTH suite's regression gate — the standing arm for aegis-fn3hdn.

    `sparql11-syntax` had no regression gate at all, because check_regression
    could not read its ledger: syntax rows key on `test` rather than
    class/manifest/id, and record `passed: true` rather than `status: "passed"`.
    Nothing errored — the checker simply found no rows it understood and
    reported "no regression", which is the shape of a gate that has stopped
    guarding rather than one that has broken.

    sattler's standing condition for the fn3hdn work: regress one suite -> red.
    These are that arm, kept as a test rather than as something someone once ran.
    """

    LEDGER = RESULTS / "sparql11-syntax.json"

    def test_the_syntax_ledger_is_readable_by_the_regression_checker(self):
        # The anti-vacuity precondition. Both assertions below would pass
        # against an EMPTY parse, which is exactly how the hole survived: the
        # checker answered "no regression" about rows it never read.
        rows = REGRESSION.load_rows(self.LEDGER)
        self.assertGreater(len(rows), 20, "syntax ledger read as nearly empty")
        self.assertTrue(
            any(REGRESSION._passed(row) for row in rows.values()),
            "no syntax row reads as PASSING — the gate cannot detect a regression",
        )

    def test_a_regressed_syntax_row_is_CAUGHT(self):
        data = json.loads(self.LEDGER.read_text())
        flipped = None
        for row in data["results"]:
            if row.get("passed") is True:
                row["passed"] = False
                flipped = row["test"]
                break
        self.assertIsNotNone(flipped, "no passing syntax row to sabotage")

        with tempfile.TemporaryDirectory() as tmp:
            candidate = pathlib.Path(tmp) / "sparql11-syntax.json"
            candidate.write_text(json.dumps(data))
            code = REGRESSION.main(
                ["--baseline", str(self.LEDGER), "--candidate", str(candidate)]
            )
        self.assertEqual(code, 1, f"regressing {flipped} must FAIL the gate")

    def test_an_unchanged_syntax_ledger_PASSES(self):
        # The other half of the pair: a gate that fails on everything is not a
        # gate either.
        code = REGRESSION.main(
            ["--baseline", str(self.LEDGER), "--candidate", str(self.LEDGER)]
        )
        self.assertEqual(code, 0)


class ArmSeparationTest(unittest.TestCase):
    """`--arm` must split TOIL from REGRESSION (aegis-fn3hdn).

    The two failures under one check name were indistinguishable on a PR page,
    so the routine one — a stale provenance stamp, remedied by one re-derive —
    read as broken conformance. Measured cost: three quipu PRs in one day
    (#237, #244, #245) sat red that way while every re-derive changed nothing
    but `duration_ms` and the stamps.

    Both arms still BLOCK. What is asserted here is that each fails on its OWN
    condition and stays green on the other's, because a split that does not
    separate is worse than no split — it adds a name that means nothing.
    """

    def _run(self, root, *args):
        import subprocess

        return subprocess.run(
            [sys.executable, str(pathlib.Path(REPORT.__file__)), "--check", *args],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_a_stale_stamp_fails_provenance_and_NOT_content(self):
        # The real toil condition: ledgers and page mutually consistent, the
        # revision simply older than HEAD. Reproduced faithfully rather than by
        # editing a stamp in isolation — doing that also desyncs the published
        # page (which embeds the revision) and reds the content arm for a reason
        # that never occurs in practice. That false reproduction is exactly what
        # my first attempt at this test did.
        root = pathlib.Path(REPORT.__file__).resolve().parents[2]
        head = subprocess.run(
            ["git", "rev-parse", "HEAD~3"],
            cwd=root, capture_output=True, text=True, check=False,
        ).stdout.strip()
        if not head:
            self.skipTest("no HEAD~3 in this clone")

        with tempfile.TemporaryDirectory() as tmp:
            work = pathlib.Path(tmp) / "repo"
            subprocess.run(
                ["git", "worktree", "add", "--detach", str(work), "HEAD"],
                cwd=root, capture_output=True, check=False,
            )
            try:
                if not (work / "benchmark/public/results").is_dir():
                    self.skipTest("worktree unavailable")
                for ledger in (work / "benchmark/public/results").glob("*.json"):
                    data = json.loads(ledger.read_text())
                    for key in list(data):
                        if key.endswith("quipu_revision"):
                            data[key] = head
                    ledger.write_text(json.dumps(data, indent=2))
                # Regenerate the page so content and ledgers AGREE.
                self._run(work, "--arm", "content")  # no-op; page written below
                subprocess.run(
                    [sys.executable, "benchmark/public/conformance_report.py"],
                    cwd=work, capture_output=True, check=False,
                )

                content = self._run(work, "--arm", "content")
                provenance = self._run(work, "--arm", "provenance")

                self.assertEqual(
                    content.returncode, 0,
                    "a stale STAMP must not read as a content regression:\n"
                    + content.stdout + content.stderr,
                )
                self.assertEqual(
                    provenance.returncode, 1,
                    "a stale stamp must fail the provenance arm:\n"
                    + provenance.stdout + provenance.stderr,
                )
                self.assertIn(
                    "STALE STAMP", provenance.stderr,
                    "the toil failure must SAY it is a stale stamp, not a regression",
                )
            finally:
                subprocess.run(
                    ["git", "worktree", "remove", "--force", str(work)],
                    cwd=root, capture_output=True, check=False,
                )

    def test_a_disagreeing_page_fails_content_and_NOT_provenance(self):
        # The converse, and the reason the split is safe: a real mismatch still
        # blocks, under the name that means "an outcome moved".
        root = pathlib.Path(REPORT.__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as tmp:
            work = pathlib.Path(tmp) / "repo"
            subprocess.run(
                ["git", "worktree", "add", "--detach", str(work), "HEAD"],
                cwd=root, capture_output=True, check=False,
            )
            try:
                page = work / "docs/book/src/benchmarks/conformance.md"
                if not page.is_file():
                    self.skipTest("page unavailable")
                # Provenance BEFORE the sabotage, so the claim is about what
                # the page edit CHANGES rather than about this branch happening
                # to have fresh stamps. An absolute `== 0` passes only while the
                # branch's ledgers are current, which on a src-touching PR they
                # are not — so it would fail for a reason that has nothing to do
                # with what is being tested.
                before = self._run(work, "--arm", "provenance").returncode
                page.write_text(page.read_text() + "\n<!-- disagreement -->\n")
                content = self._run(work, "--arm", "content")
                provenance = self._run(work, "--arm", "provenance")

                self.assertEqual(
                    content.returncode, 1,
                    "a disagreeing page must still BLOCK on the content arm",
                )
                self.assertEqual(
                    provenance.returncode, before,
                    "a disagreeing page must not change the provenance verdict:\n"
                    + provenance.stdout + provenance.stderr,
                )
            finally:
                subprocess.run(
                    ["git", "worktree", "remove", "--force", str(work)],
                    cwd=root, capture_output=True, check=False,
                )


class RemedyNamesEveryStepTest(unittest.TestCase):
    """The printed remedy must name ALL THREE steps (aegis-j9zw4u).

    The dispatch alone does NOT clear the check: conformance.yml is
    `contents: read` with upload-artifact as its only sink, so it produces JSON
    that nothing is permitted to write back. A remedy naming only the dispatch
    reads as complete, sends the follower to watch a green run, and leaves them
    at a still-red check with no indication which half failed — and because the
    gate itself printed it at the moment of failure, it is the instruction that
    gets trusted over any doc.

    Caught on PR #202 only because gennaro declined to assume the dispatch had
    cleared the check. This test is so that it cannot silently regress to one
    step again.
    """

    def test_the_remedy_names_the_dispatch_the_artifact_and_the_commit_target(self):
        # Rendered through main() so this asserts what an operator actually
        # sees, not what a helper returns.
        import io
        import contextlib

        # A REAL older commit, not a fabricated sha. `"0" * 40` is not in the
        # clone, so the gate returns UNVERIFIED (2) rather than DRIFT (1) and
        # never prints a remedy at all — a different branch, and one that makes
        # every assertion below vacuous. My first draft of this test did exactly
        # that, which is the same mistake the sabotage arms on aegis-fn3hdn hit.
        older = subprocess.run(
            ["git", "rev-parse", "HEAD~3"],
            cwd=pathlib.Path(REPORT.__file__).resolve().parents[2],
            capture_output=True, text=True, check=False,
        ).stdout.strip()
        if len(older) != 40:
            self.skipTest("no HEAD~3 in this clone")

        with tempfile.TemporaryDirectory() as tmp:
            results = pathlib.Path(tmp) / "results"
            results.mkdir()
            for ledger in RESULTS.glob("*.json"):
                data = json.loads(ledger.read_text())
                for key in list(data):
                    if key.endswith("quipu_revision"):
                        data[key] = older
                (results / ledger.name).write_text(json.dumps(data))
            err = io.StringIO()
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
                REPORT.main(
                    ["--results-dir", str(results), "--docs-dir", str(DOCS),
                     "--check", "--arm", "provenance"]
                )
            text = err.getvalue()

        # ANTI-VACUITY, and this one has to be exact: the gate must have taken
        # the DRIFT branch. An earlier version of this assertion tested
        # `text.lower() + "ledger"`, which contains "ledger" unconditionally and
        # so could never fail — a vacuous guard in the test whose whole job is
        # catching a vacuous remedy.
        self.assertIn(
            "was NOT derived from the code it ships with", text,
            f"the gate must have taken the DRIFT branch, got:\n{text}",
        )

        for needle, why in (
            ("gh workflow run conformance.yml", "step 1: the dispatch"),
            ("conformance-ledgers-", "step 2: the ARTIFACT to download, by name"),
            ("benchmark/public/results/", "step 3: WHERE the files must be committed"),
        ):
            self.assertIn(
                needle, text,
                f"the remedy must name {why} — a partial remedy is the defect "
                f"aegis-j9zw4u records. Got:\n{text}",
            )
