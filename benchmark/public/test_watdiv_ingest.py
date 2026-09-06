"""Tests for the WatDiv bulk-ingest runner.

Every arm here exercises a REFUSAL or a LABEL -- the parts that decide whether a
number may be published -- without running an ingest. A test that needed a real
10.9M load would never run in CI, so the guards would be the untested half of a
runner whose entire job is to refuse bad numbers.
"""

import importlib.util
import json
import pathlib
import tarfile
import tempfile
import unittest

MODULE_PATH = pathlib.Path(__file__).with_name("watdiv_ingest.py")
SPEC = importlib.util.spec_from_file_location("watdiv_ingest", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)


def row(**over):
    base = dict(
        scale="10M", archive_url="u", archive_sha="a", source_sha="b",
        triples=10, source_bytes=100, quipu_bin="q", quipu_version="v",
        chunk=50_000, timestamp="2026-01-01T00:00:00Z", exit_code=0,
        seconds=10.0, facts_before=0, facts_after=1000, store_bytes=5,
        load1=1.0, ncpu=20,
    )
    base.update(over)
    return MODULE.build_row(**base)


class ThroughputTests(unittest.TestCase):
    def test_rate_comes_from_the_live_fact_delta(self):
        # 1000 facts in 10s. If this ever read a parse count instead, a re-ingest
        # of identical content would report full throughput while writing nothing.
        self.assertEqual(row()["throughput_facts_per_sec"], 100.0)
        self.assertEqual(row()["result"]["live_fact_delta"], 1000)

    def test_an_unreadable_store_does_not_become_a_zero_baseline(self):
        # live_facts returns -1 for UNKNOWN. Treating that as 0 would inflate the
        # delta by whatever the store already held.
        self.assertEqual(row(facts_before=-1, facts_after=1000)["result"]["live_fact_delta"], 1000)

    def test_zero_seconds_yields_no_rate_rather_than_a_division(self):
        self.assertIsNone(row(seconds=0)["throughput_facts_per_sec"])


class ValidityTests(unittest.TestCase):
    def test_a_clean_run_on_a_quiet_host_is_valid(self):
        r = row(exit_code=0, load1=2.0, ncpu=20)
        self.assertTrue(r["valid_result"])
        self.assertIsNone(r["invalid_reason"])

    def test_a_nonzero_exit_is_invalid_and_says_so(self):
        r = row(exit_code=143)
        self.assertFalse(r["valid_result"])
        self.assertIn("exit 143", r["invalid_reason"])

    def test_a_contended_host_invalidates_the_row(self):
        # The row is still WRITTEN -- an unlabelled fast number is the hazard,
        # not a labelled slow one.
        r = row(load1=15.0, ncpu=20)
        self.assertFalse(r["valid_result"])
        self.assertIn("CONTENDED", r["invalid_reason"])
        self.assertIsNotNone(r["throughput_facts_per_sec"])

    def test_both_reasons_are_reported_not_just_the_first(self):
        r = row(exit_code=1, load1=19.0, ncpu=20)
        self.assertIn("exit 1", r["invalid_reason"])
        self.assertIn("CONTENDED", r["invalid_reason"])


class PopulationTests(unittest.TestCase):
    def test_the_declared_triple_count_rides_with_the_rate(self):
        # WatDiv's "10M" holds 10,916,457 triples. A row carrying a rate but not
        # its population invites "at 10M", which is wrong by 9%.
        r = row(triples=10_916_457)
        self.assertEqual(r["source"]["triples_declared"], 10_916_457)
        self.assertIn("throughput_facts_per_sec", r)


class PinTests(unittest.TestCase):
    def test_first_sight_records_a_pin(self):
        with tempfile.TemporaryDirectory() as d:
            pins = pathlib.Path(d) / "pins.tsv"
            self.assertEqual(MODULE.verify_or_record_pin(pins, "w.tar.bz2", "abc"), "recorded")
            self.assertIn("abc", pins.read_text())

    def test_a_matching_digest_verifies(self):
        with tempfile.TemporaryDirectory() as d:
            pins = pathlib.Path(d) / "pins.tsv"
            MODULE.verify_or_record_pin(pins, "w.tar.bz2", "abc")
            self.assertEqual(MODULE.verify_or_record_pin(pins, "w.tar.bz2", "abc"), "verified")

    def test_a_mismatch_ABORTS_rather_than_continuing(self):
        # The upstream artifact changing under us is a finding. Continuing would
        # publish a number about bytes nobody pinned.
        with tempfile.TemporaryDirectory() as d:
            pins = pathlib.Path(d) / "pins.tsv"
            MODULE.verify_or_record_pin(pins, "w.tar.bz2", "abc")
            with self.assertRaises(SystemExit) as cm:
                MODULE.verify_or_record_pin(pins, "w.tar.bz2", "def")
            self.assertIn("PIN MISMATCH", str(cm.exception))


class SourceTests(unittest.TestCase):
    def _archive(self, d, body: bytes, name="data.nt"):
        nt = pathlib.Path(d) / name
        nt.write_bytes(body)
        arc = pathlib.Path(d) / "w.tar.bz2"
        with tarfile.open(arc, "w:bz2") as tar:
            tar.add(nt, arcname=name)
        return arc

    def test_measures_triples_and_digest_without_unpacking(self):
        body = b'<a> <b> "1" .\n<a> <b> "2" .\n<a> <b> "3" .\n'
        with tempfile.TemporaryDirectory() as d:
            arc = self._archive(d, body)
            triples, digest, size = MODULE.measure_source(arc)
            self.assertEqual(triples, 3)
            self.assertEqual(size, len(body))
            import hashlib
            self.assertEqual(digest, hashlib.sha256(body).hexdigest())
            # The whole point: nothing was written beside the archive.
            self.assertEqual(
                sorted(p.name for p in pathlib.Path(d).iterdir()), ["data.nt", "w.tar.bz2"]
            )

    def test_an_archive_with_no_nt_member_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            other = pathlib.Path(d) / "readme.txt"
            other.write_text("hi")
            arc = pathlib.Path(d) / "w.tar.bz2"
            with tarfile.open(arc, "w:bz2") as tar:
                tar.add(other, arcname="readme.txt")
            with self.assertRaises(SystemExit):
                MODULE.measure_source(arc)


class LedgerTests(unittest.TestCase):
    def test_a_row_is_json_serialisable_and_carries_its_caveat(self):
        r = row()
        text = json.dumps(r)
        self.assertIn("PARSE count", text)
        self.assertIn("live_fact_delta", text)


if __name__ == "__main__":
    unittest.main()
