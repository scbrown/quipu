"""Tests for the source-homogeneity check.

The check exists because two cheap eyeball tests -- predicate count and bytes per
line -- both PASS on the WatDiv 100M prefix while new-terms-per-triple sweeps
1.47x. So the arms that matter are the ones where the misleading signals are held
constant and only the real one moves.
"""

import importlib.util
import io
import pathlib
import unittest

MODULE_PATH = pathlib.Path(__file__).with_name("source_homogeneity.py")
SPEC = importlib.util.spec_from_file_location("source_homogeneity", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)


def nt(triples, sep=b"\t"):
    return io.BytesIO(b"".join(sep.join(t) + b" .\n" for t in triples))


class SplittingTests(unittest.TestCase):
    def test_tab_separated_is_parsed(self):
        # WatDiv's .nt is TAB-separated. A space-only splitter finds nothing here
        # and reports 0.0 new terms per triple for every band, which reads as a
        # perfectly homogeneous source rather than as a broken parser.
        rows = list(MODULE.bands(nt([(b"<s1>", b"<p>", b"<o1>")]), band=1))
        self.assertEqual(rows[0]["new_terms"], 3)
        self.assertEqual(rows[0]["distinct_predicates"], 1)

    def test_space_separated_is_parsed_too(self):
        rows = list(MODULE.bands(nt([(b"<s1>", b"<p>", b"<o1>")], sep=b" "), band=1))
        self.assertEqual(rows[0]["new_terms"], 3)

    def test_object_containing_spaces_stays_one_term(self):
        rows = list(MODULE.bands(nt([(b"<s>", b"<p>", b'"a b c"')]), band=1))
        self.assertEqual(rows[0]["new_terms"], 3)

    def test_malformed_lines_are_skipped_not_counted_as_terms(self):
        stream = io.BytesIO(b"garbage\n<s>\t<p>\t<o> .\n")
        rows = list(MODULE.bands(stream, band=2))
        self.assertEqual(rows[0]["new_terms"], 3)


class BandingTests(unittest.TestCase):
    def test_terms_are_new_only_once_across_bands(self):
        # The same triple twice: the second band introduces nothing, which is the
        # cumulative property the check depends on.
        rows = list(MODULE.bands(nt([(b"<s>", b"<p>", b"<o>")] * 2), band=1))
        self.assertEqual([r["new_terms"] for r in rows], [3, 0])
        self.assertEqual(rows[-1]["cumulative_terms"], 3)

    def test_a_short_final_band_is_reported_with_its_own_span(self):
        rows = list(MODULE.bands(nt([(b"<s%d>" % i, b"<p>", b"<o>") for i in range(5)]), band=2))
        self.assertEqual([r["triples"] for r in rows], [2, 2, 1])

    def test_limit_stops_the_scan(self):
        rows = list(MODULE.bands(nt([(b"<s%d>" % i, b"<p>", b"<o>") for i in range(100)]),
                                 band=10, limit=30))
        self.assertEqual(sum(r["triples"] for r in rows), 30)


class VerdictTests(unittest.TestCase):
    def test_flat_source_passes(self):
        rows = [{"new_per_triple": 0.50}, {"new_per_triple": 0.52}, {"new_per_triple": 0.51}]
        ok, why = MODULE.verdict(rows, 0.10)
        self.assertTrue(ok, why)

    def test_the_watdiv_prefix_sweep_is_caught(self):
        # The real numbers from the 100M archive's first 6.25M triples. Flat
        # predicate count, flat bytes per line, and a 1.47x sweep underneath.
        rows = [{"new_per_triple": v} for v in (0.4225, 0.3592, 0.3258, 0.2964, 0.2877)]
        ok, why = MODULE.verdict(rows, 0.10)
        self.assertFalse(ok)
        self.assertIn("1.4", why)

    def test_a_step_is_caught(self):
        rows = [{"new_per_triple": v} for v in (0.288, 0.288, 0.433, 0.496)]
        self.assertFalse(MODULE.verdict(rows, 0.10)[0])

    def test_single_band_cannot_be_judged(self):
        ok, why = MODULE.verdict([{"new_per_triple": 0.4}], 0.10)
        self.assertTrue(ok)
        self.assertIn("nothing to compare", why)

    def test_a_band_with_no_new_terms_is_not_silently_homogeneous(self):
        ok, why = MODULE.verdict([{"new_per_triple": 0.4}, {"new_per_triple": 0.0}], 0.10)
        self.assertFalse(ok)
        self.assertIn("undefined", why)

    def test_exit_status_reports_the_verdict(self):
        import contextlib
        src = io.BytesIO(b"".join(b"<s%d>\t<p>\t<o%d> .\n" % (i, i) for i in range(20)))
        import sys
        old = sys.stdin
        try:
            sys.stdin = type("F", (), {"buffer": src})()
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                code = MODULE.main(["--band", "10"])
        finally:
            sys.stdin = old
        self.assertIn(code, (0, 1))


if __name__ == "__main__":
    unittest.main()
