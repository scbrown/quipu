"""Producer identity survives serialization by each previously unstamped runner."""
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

import conformance_provenance
import shacl_core
import sparql11_federated
import sparql11_syntax


class ProvenanceTests(unittest.TestCase):
    def test_local_and_partial_ci_do_not_claim_a_run(self):
        for env in ({}, {"GITHUB_RUN_ID": "123"}):
            with self.subTest(env=env), patch.dict(os.environ, env, clear=True):
                at, by = conformance_provenance.provenance()
                self.assertRegex(at, r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
                self.assertEqual(by, "local")

    def test_each_runner_serializes_ci_and_local_origin(self):
        ci = {"GITHUB_SERVER_URL": "https://github.com",
              "GITHUB_REPOSITORY": "example/project", "GITHUB_RUN_ID": "123"}
        for runner in (sparql11_syntax, sparql11_federated, shacl_core):
            for env, expected in ((ci, "https://github.com/example/project/actions/runs/123"),
                                  ({}, "local")):
                with self.subTest(runner=runner.__name__, env=env), tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    (root / "manifest.ttl").write_text("fixture")
                    (root / "a.rq").write_text("ASK {}")
                    output = root / "ledger.json"
                    argv = ["runner", "--suite", str(root), "--quipu", "stub",
                            "--output", str(output), "--allow-unpinned-suite"]
                    with contextlib.ExitStack() as stack:
                        stack.enter_context(patch.dict(os.environ, env, clear=True))
                        stack.enter_context(patch("sys.argv", argv))
                        stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
                        stack.enter_context(patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout="stub", stderr="", returncode=0)))
                        if runner is sparql11_syntax:
                            stack.enter_context(patch.object(runner, "approved_cases", return_value=[("a.rq", True)]))
                        elif runner is sparql11_federated:
                            stack.enter_context(patch.object(runner.EVAL, "executable_path", side_effect=Path))
                            stack.enter_context(patch.object(runner, "discover", return_value=[(None, ())] * 7))
                            stack.enter_context(patch.object(runner, "run_case", return_value={"status": "passed"}))
                        else:
                            stack.enter_context(patch.object(runner, "executable_path", side_effect=Path))
                            stack.enter_context(patch.object(runner, "discover_cases", return_value=[None]))
                            stack.enter_context(patch.object(runner, "run_case", return_value={"id": "one", "fixture": "one", "category": "core", "status": "passed"}))
                        self.assertEqual(runner.main(), 0)
                    ledger = json.loads(output.read_text())
                    self.assertEqual(ledger["generated_by"], expected)
                    self.assertRegex(ledger["generated_at"], r"^\d{4}-\d{2}-\d{2}T.*Z$")
