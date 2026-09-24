"""rdf11_syntax: the comparison must not agree with a wrong answer."""

from __future__ import annotations

import unittest

import rdf11_syntax as r

MANIFEST = """
@prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
@prefix rdft: <http://www.w3.org/ns/rdftest#> .
<> a mf:Manifest ;
    mf:assumedTestBase <https://example.org/suite/> ;
    mf:entries ( <#pos> <#neg> <#ev> ) .

<#pos> rdf:type rdft:TestTurtlePositiveSyntax ;
    rdft:approval rdft:Approved ;
    mf:action <pos.ttl> .

<#neg> a rdft:TestTurtleNegativeSyntax ;
    mf:action    <neg.ttl> ;
    .

<#ev> rdf:type rdft:TestTurtleEval ;
    rdft:approval rdft:Proposed ;
    mf:action <ev.ttl> ;
    mf:result <ev.nt> .
"""


class Manifest(unittest.TestCase):
    def test_every_block_shape_and_terminator_is_read(self):
        base, cases = r.cases_of(MANIFEST, "rdf-turtle")
        self.assertEqual(base, "https://example.org/suite/")
        self.assertEqual([(c["name"], c["kind"], c["approval"]) for c in cases],
                         [("pos", "positive", "Approved"), ("neg", "negative", "unmarked"),
                          ("ev", "eval", "Proposed")])
        self.assertEqual(cases[2]["result"], "ev.nt")

    def test_a_case_missing_from_the_parse_is_refused(self):
        with self.assertRaises(ValueError):
            r.cases_of(MANIFEST.replace("<#neg> a rdft:", "<#neg> ex:type rdft:"), "rdf-turtle")

    def test_a_manifest_without_a_base_uses_the_published_location(self):
        base, _ = r.cases_of(MANIFEST.replace("mf:assumedTestBase <https://example.org/suite/> ;", ""),
                             "rdf-n-triples")
        self.assertEqual(base, "https://w3c.github.io/rdf-tests/rdf/rdf11/rdf-n-triples/")


class Reader(unittest.TestCase):
    def test_terms_escapes_and_literal_forms(self):
        [t] = r.read_ntriples('<http://a/s> <http://a/\\u0070> "x\\r\\"y"@EN-gb .\n')
        self.assertEqual(t[1], ("I", "http://a/p"))
        self.assertEqual(t[2], ("L", 'x\r"y', r.RDF_LANG_STRING, "en-gb"))
        [t] = r.read_ntriples('<http://a/s> <http://a/p> "z" .')
        self.assertEqual(t[2], ("L", "z", r.XSD_STRING, ""))

    def test_lexical_form_is_part_of_the_term(self):
        a = r.read_ntriples('<http://a/s> <http://a/p> "01"^^<http://www.w3.org/2001/XMLSchema#integer> .')
        b = r.read_ntriples('<http://a/s> <http://a/p> "1"^^<http://www.w3.org/2001/XMLSchema#integer> .')
        self.assertFalse(r.isomorphic(a, b))

    def test_a_blank_label_may_contain_dots_and_touch_the_terminator(self):
        [t] = r.read_ntriples("<http://a/s> <http://a/p> _:a.b.")
        self.assertEqual(t[2], ("B", "a.b"))
        [t] = r.read_ntriples("<http://a/s> <http://a/p> _:a·‿.⁀ .")
        self.assertEqual(t[2], ("B", "a·‿.⁀"))

    def test_only_lf_ends_a_line(self):
        self.assertEqual(len(r.read_ntriples('<http://a/s> <http://a/p> " " .\n')), 1)

    def test_garbage_is_refused_not_skipped(self):
        with self.assertRaises(ValueError):
            r.read_ntriples("<http://a/s> <http://a/p> .")


class Isomorphism(unittest.TestCase):
    def graph(self, text):
        return r.read_ntriples(text)

    def test_blank_labels_do_not_matter(self):
        self.assertTrue(r.isomorphic(self.graph("_:x <http://a/p> _:y .\n_:y <http://a/q> <http://a/o> ."),
                                     self.graph("_:m <http://a/p> _:n .\n_:n <http://a/q> <http://a/o> .")))

    def test_structure_does(self):
        self.assertFalse(r.isomorphic(self.graph("_:x <http://a/p> _:y .\n_:y <http://a/q> <http://a/o> ."),
                                      self.graph("_:m <http://a/p> _:n .\n_:m <http://a/q> <http://a/o> .")))

    def test_one_blank_cannot_stand_for_two(self):
        self.assertFalse(r.isomorphic(self.graph("_:x <http://a/p> <http://a/o> .\n_:y <http://a/p> <http://a/o> ."),
                                      self.graph("_:m <http://a/p> <http://a/o> .")))


if __name__ == "__main__":
    unittest.main()
