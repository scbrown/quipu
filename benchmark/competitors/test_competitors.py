"""The comparison rules the competitor table rests on (aegis-hit21a).

Each rule here was a HARNESS defect found while triaging a competitor failure,
fixed for every system alike. The tests pin both directions: the rule makes
two genuinely identical RDF terms compare equal, and it never makes two
different ones equal. A rule that only ever says "equal" would turn every
competitor green, which is the failure these tests exist to catch.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import competitors as c  # noqa: E402

XSD = "http://www.w3.org/2001/XMLSchema#"


class RdfTermIdentity(unittest.TestCase):
    def test_language_tags_are_case_insensitive(self):
        self.assertEqual(c.rdf_term('"bar"@en-US'), c.rdf_term('"bar"@en-us'))

    def test_different_language_tags_stay_different(self):
        self.assertNotEqual(c.rdf_term('"bar"@en'), c.rdf_term('"bar"@de'))

    def test_a_simple_literal_is_an_xsd_string(self):
        self.assertEqual(c.rdf_term(f'"ABC"^^<{XSD}string>'), c.rdf_term('"ABC"'))

    def test_other_datatypes_are_not_stripped(self):
        self.assertNotEqual(c.rdf_term(f'"1"^^<{XSD}decimal>'), c.rdf_term('"1"'))


class StrictVerdictStaysStrict(unittest.TestCase):
    def test_numeric_lexical_forms_differ_under_the_verdict(self):
        # The PRIMARY verdict is RDF term equality: "1.0" and "1" differ.
        self.assertFalse(c.same_rows([(f'"1"^^<{XSD}decimal>',)], [(f'"1.0"^^<{XSD}decimal>',)]))

    def test_but_are_tagged_same_value(self):
        self.assertTrue(
            c.same_rows_by_value([(f'"1"^^<{XSD}decimal>',)], [(f'"1.0"^^<{XSD}decimal>',)]))

    def test_different_values_are_never_same_value(self):
        self.assertFalse(
            c.same_rows_by_value([(f'"1"^^<{XSD}decimal>',)], [(f'"2"^^<{XSD}decimal>',)]))

    def test_zero_durations_are_one_value(self):
        a = f'"P0D"^^<{XSD}dayTimeDuration>'
        b = f'"PT0S"^^<{XSD}dayTimeDuration>'
        self.assertFalse(c.same_rows([(a,)], [(b,)]))
        self.assertTrue(c.same_rows_by_value([(a,)], [(b,)]))
        self.assertFalse(c.same_rows_by_value([(a,)], [(f'"-PT8H"^^<{XSD}dayTimeDuration>',)]))


class BlankNodes(unittest.TestCase):
    def test_equal_up_to_one_consistent_renaming(self):
        self.assertTrue(c.same_rows([("_:a", "_:a")], [("_:x", "_:x")]))

    def test_not_equal_when_the_renaming_is_inconsistent(self):
        # bnode01: two solutions that must get DIFFERENT blank nodes.
        self.assertFalse(c.same_rows([("_:foo",), ("_:foo",)], [("_:b1",), ("_:b2",)]))


class Pins(unittest.TestCase):
    def test_every_binary_pin_names_a_sha256(self):
        for name, pin in c.PINS.items():
            with self.subTest(name=name):
                self.assertRegex(pin["sha256"], r"^[0-9a-f]{64}$")
                self.assertIn(pin["version"], pin["url"])


if __name__ == "__main__":
    unittest.main()
