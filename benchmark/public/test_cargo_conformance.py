"""Cargo-only changes must reach and invalidate the conformance ledger gate."""
import contextlib
import fnmatch
import pathlib
import re
import unittest

import test_conformance_report as report_tests


class CargoConformanceTest(unittest.TestCase):
    def test_cargo_changes_invalidate_both_strict_and_pr_provenance(self):
        probe = report_tests.LedgerProvenancePrModeTest()
        for filename in ("Cargo.toml", "Cargo.lock"):
            with self.subTest(filename=filename), contextlib.ExitStack() as stack:
                root, run, base = probe._repo(stack)
                run("checkout", "-qb", "pr")
                (root / filename).write_text('# changed build input\n')
                run("add", "-A")
                run("commit", "-qm", "change Cargo input")
                for pr_base in (None, "main"):
                    code, messages = probe._provenance(root, {"quipu_revision": base}, pr_base)
                    self.assertEqual(code, 1, (filename, pr_base, messages))
                    self.assertIn(filename, " ".join(messages))

    def test_docs_only_change_keeps_provenance_fresh(self):
        probe = report_tests.LedgerProvenancePrModeTest()
        with contextlib.ExitStack() as stack:
            root, run, base = probe._repo(stack)
            (root / "README.md").write_text("documentation\n")
            run("add", "-A")
            run("commit", "-qm", "documentation only")
            for pr_base in (None, base):
                code, messages = probe._provenance(root, {"quipu_revision": base}, pr_base)
                self.assertEqual(code, 0, messages)

    def test_workflow_selects_cargo_on_pr_and_main_push(self):
        root = pathlib.Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/conformance.yml").read_text()
        for event in ("pull_request", "push"):
            section = re.search(rf"^  {event}:\n(.*?)(?=^  [a-z_]+:|\Z)", workflow, re.M | re.S)
            self.assertIsNotNone(section)
            paths = re.findall(r"^      - '([^']+)'$", section.group(1), re.M)
            for filename in ("Cargo.toml", "Cargo.lock", "src/lib.rs"):
                with self.subTest(event=event, filename=filename):
                    self.assertTrue(any(fnmatch.fnmatchcase(filename, p) for p in paths),
                                    (event, filename, paths))


if __name__ == "__main__":
    unittest.main()
