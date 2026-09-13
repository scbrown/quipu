#!/usr/bin/env python3
"""Tests for `watdiv_instantiate.py`.

The properties worth testing here are the REFUSALS, not the happy path. A
template instantiated against a class that is absent from the loaded slice
produces a query that parses, runs, and matches nothing in both engines -- which
reads as a clean, fast, agreeing result. That is the failure this script exists
to prevent, so it is the one the tests have to be able to see.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "watdiv_instantiate.py"

MODEL = """\
#namespace\tgn=http://www.geonames.org/ontology#
#namespace\twsdbm=http://db.uwaterloo.ca/~galuc/wsdbm/
"""

# One template that CAN bind against the slice below, one that cannot.
BINDABLE = """\
#mapping v0 wsdbm:City uniform
SELECT ?v1 WHERE {
\t%v0%\tgn:parentCountry\t?v1 .
}
"""

UNBINDABLE = """\
#mapping v0 wsdbm:Retailer uniform
SELECT ?v1 WHERE {
\t%v0%\tgn:parentCountry\t?v1 .
}
"""

# A slice containing City entities and NO Retailer, mirroring the measured shape
# of a 1M-line prefix of watdiv.10M.nt.
SLICE = (
    "<http://db.uwaterloo.ca/~galuc/wsdbm/City1>\t"
    "<http://www.geonames.org/ontology#parentCountry>\t"
    "<http://db.uwaterloo.ca/~galuc/wsdbm/Country3> .\n"
    "<http://db.uwaterloo.ca/~galuc/wsdbm/City2>\t"
    "<http://www.geonames.org/ontology#parentCountry>\t"
    "<http://db.uwaterloo.ca/~galuc/wsdbm/Country4> .\n"
)


class WatdivInstantiateTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.templates = root / "templates"
        self.templates.mkdir()
        (self.templates / "L2.txt").write_text(BINDABLE, encoding="utf-8")
        (self.templates / "S1.txt").write_text(UNBINDABLE, encoding="utf-8")
        self.model = root / "model.txt"
        self.model.write_text(MODEL, encoding="utf-8")
        self.slice = root / "slice.nt"
        self.slice.write_text(SLICE, encoding="utf-8")
        self.out = root / "out"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def run_script(self, *extra: str, out: Path | None = None):
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--templates",
                str(self.templates),
                "--model",
                str(self.model),
                "--slice",
                str(self.slice),
                "--out",
                str(out or self.out),
                "--seed",
                "1",
                *extra,
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_refuses_a_class_absent_from_the_slice_and_writes_nothing(self) -> None:
        """The load-bearing refusal: exit 3, the template NAMED, no output at all.

        Writing nothing matters as much as the exit code -- a partially written
        query directory would be picked up by the harness on a later run.
        """
        result = self.run_script()
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn("S1", result.stderr)
        self.assertIn("wsdbm:Retailer", result.stderr)
        self.assertFalse(self.out.exists() and any(self.out.iterdir()))

    def test_bindable_template_alone_is_accepted(self) -> None:
        """Control for the test above: without the unbindable template it passes.

        Without this, a script that refused unconditionally would satisfy the
        refusal test and nothing would notice.
        """
        (self.templates / "S1.txt").unlink()
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.out / "L2.rq").exists())

    def test_allow_unbindable_emits_the_rest_and_names_the_omitted(self) -> None:
        """NOT RUN is published as NOT RUN, at template granularity."""
        result = self.run_script("--allow-unbindable")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.out / "L2.rq").exists())
        self.assertFalse((self.out / "S1.rq").exists())
        manifest = json.loads((self.out / "instantiation-manifest.json").read_text())
        self.assertIn("S1", manifest["not_run"])
        self.assertEqual(manifest["not_run"]["S1"], ["wsdbm:Retailer"])

    def test_binding_comes_from_the_slice_not_from_a_class_name(self) -> None:
        """The emitted constant must be an entity that is actually in the slice."""
        self.run_script("--allow-unbindable")
        text = (self.out / "L2.rq").read_text()
        self.assertTrue(
            "wsdbm/City1>" in text or "wsdbm/City2>" in text,
            f"bound something not in the slice:\n{text}",
        )

    def test_namespaces_are_read_from_the_model_not_hardcoded(self) -> None:
        """WatDiv's foaf/gr roots are nonstandard; a baked-in map matches nothing.

        Changing the model must change the emitted PREFIX line. If the script
        ever hardcodes a prefix map this fails.
        """
        self.model.write_text(
            MODEL.replace(
                "gn=http://www.geonames.org/ontology#", "gn=http://example.test/ns#"
            ),
            encoding="utf-8",
        )
        self.run_script("--allow-unbindable")
        text = (self.out / "L2.rq").read_text()
        self.assertIn("PREFIX gn: <http://example.test/ns#>", text)

    def test_refuses_a_model_with_no_namespaces(self) -> None:
        """An empty prefix map would surface as a parse error at the harness."""
        self.model.write_text("# nothing here\n", encoding="utf-8")
        result = self.run_script("--allow-unbindable")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("namespace", result.stderr.lower())

    def test_graph_scoping_is_emitted_when_asked(self) -> None:
        """The harness hands the text verbatim to both arms, so the scope must be
        in the file or Quipu answers from ROOT while Oxigraph answers its default."""
        self.run_script("--allow-unbindable", "--graph", "urn:test:g")
        self.assertIn("GRAPH <urn:test:g>", (self.out / "L2.rq").read_text())

    def test_same_seed_reproduces_the_same_bindings(self) -> None:
        """Two runs at the same seed must be comparable, or the ledger is fiction."""
        first = self.out / "a"
        second = self.out / "b"
        self.run_script("--allow-unbindable", out=first)
        self.run_script("--allow-unbindable", out=second)
        self.assertEqual(
            (first / "L2.rq").read_text(),
            (second / "L2.rq").read_text(),
        )

    def test_manifest_pins_the_slice_digest(self) -> None:
        """A binding set is only meaningful against the bytes it was drawn from."""
        self.run_script("--allow-unbindable")
        manifest = json.loads((self.out / "instantiation-manifest.json").read_text())
        self.assertEqual(manifest["slice"]["lines"], 2)
        self.assertEqual(len(manifest["slice"]["sha256"]), 64)
        self.assertEqual(manifest["pool_sizes"]["City"], 2)
        self.assertEqual(manifest["pool_sizes"]["Retailer"], 0)


if __name__ == "__main__":
    unittest.main()
