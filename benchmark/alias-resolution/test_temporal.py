"""Leakage controls run on a tiny isolated history, never a served database."""

import json
import sqlite3
import struct
import tempfile
import unittest
from pathlib import Path

from temporal import COMMENT, LABEL, SAME, TYPE, Snapshot, clean, prepare_pair


class HistoricalEvidence(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "fixture.db"
        self.db = sqlite3.connect(self.path)
        self.addCleanup(self.db.close)
        self.db.executescript("""
            CREATE TABLE terms(id INTEGER PRIMARY KEY, iri TEXT);
            CREATE TABLE transactions(id INTEGER, timestamp TEXT);
            CREATE TABLE facts(e INTEGER,a INTEGER,v BLOB,tx INTEGER,
              valid_from TEXT,valid_to TEXT,retracted_tx INTEGER,g INTEGER,op INTEGER);
        """)
        self.terms = {
            1: "urn:left",
            2: "urn:right",
            3: "urn:bridge",
            4: "urn:Type",
            5: LABEL,
            6: COMMENT,
            7: TYPE,
            8: SAME,
            9: "urn:related",
        }
        self.db.executemany("INSERT INTO terms VALUES (?,?)", self.terms.items())
        self.before = "2020-01-01T00:00:00Z"
        self.repair = "2020-01-03T00:00:00Z"
        self.after = "2020-01-05T00:00:00Z"
        self.db.execute("INSERT INTO transactions VALUES (3,?)", (self.repair,))
        self.db.execute("INSERT INTO transactions VALUES (5,?)", (self.after,))
        for e in (1, 2, 3):
            self.add(e, 5, "record " + str(e), 1)
            self.add(e, 7, 4, 1)
        self.add(1, 8, 3, 2)
        self.add(3, 8, 2, 3, vf=self.repair)
        self.add(1, 8, 2, 5, vf=self.after)

    def add(self, e, a, value, tx, vf=None, vt=None, rt=None, graph=0, op=1):
        blob = (
            b"\x00" + struct.pack("<q", value)
            if isinstance(value, int)
            else b"\x01" + value.encode()
        )
        self.db.execute(
            "INSERT INTO facts VALUES (?,?,?,?,?,?,?,?,?)",
            (e, a, blob, tx, vf or self.before, vt, rt, graph, op),
        )
        self.db.commit()

    def snapshot(self):
        snapshot = Snapshot(self.path)
        self.addCleanup(snapshot.db.close)
        return snapshot

    def test_backdated_late_fact_and_other_graph_never_leak(self):
        self.add(1, 6, "known earlier description", 1)
        self.add(1, 6, "backdated answer", 4, vf=self.before)
        self.add(1, 6, "other graph answer", 1, graph=99)
        snapshot = self.snapshot()
        before, _ = snapshot.facts(1, 3, self.repair)
        after, _ = snapshot.facts(1, 6, self.after)
        self.assertIn("known earlier description", json.dumps(before))
        self.assertNotIn("backdated answer", json.dumps(before))
        self.assertIn("backdated answer", json.dumps(after))
        self.assertNotIn("other graph answer", json.dumps(after))

    def test_transitive_cutoff_precedes_direct_repair(self):
        self.assertEqual(self.snapshot().first_connection(1, 2), (3, self.repair))

    def test_retraction_and_equal_boundaries(self):
        self.add(1, 6, "live until repair", 1, vt=self.repair, rt=3)
        self.add(1, 6, "closed earlier", 1, vt=self.before, rt=2)
        self.add(1, 6, "unknown legacy", 1, vt=self.after)
        self.add(1, 6, "starts at repair", 2, vf=self.repair)
        facts, unknown = self.snapshot().facts(1, 3, self.repair)
        text = json.dumps(facts)
        self.assertIn("live until repair", text)
        for prohibited in ("closed earlier", "unknown legacy", "starts at repair"):
            self.assertNotIn(prohibited, text)
        self.assertEqual(unknown["legacy_closed"], 1)

    def test_ambiguous_timestamp_is_withheld_not_assumed_utc(self):
        self.add(1, 6, "unknown time", 1, vt="2020-01-06 00:00:00")
        facts, unknown = self.snapshot().facts(1, 3, self.repair)
        self.assertNotIn("unknown time", json.dumps(facts))
        self.assertEqual(unknown["ambiguous_timestamp"], 1)

    def test_identity_strip_has_positive_control(self):
        facts, _ = self.snapshot().facts(1, 3, self.repair)
        self.assertEqual(sum(f["predicate"] == SAME for f in facts), 1)
        kept, audit = clean(facts)
        self.assertFalse(any(f["predicate"] == SAME for f in kept))
        self.assertEqual(audit[0]["reason"], "identity_relation")
        self.assertTrue(any(f["predicate"] == LABEL for f in kept))

    def test_neighbours_use_same_time_and_no_audit_in_state(self):
        self.add(1, 9, 3, 1)
        self.add(3, 5, "future neighbour name", 4)
        pair = {"left": "urn:left", "right": "urn:right", "class": "secret-gold"}
        prepared = prepare_pair(self.snapshot(), pair)
        text = json.dumps(prepared["state"])
        self.assertNotIn("future neighbour name", text)
        for field in ("repair_tx", "secret-gold", "audit", "sameAs"):
            self.assertNotIn(field, text)
        self.assertIn("record 3", text)
        self.assertTrue(prepared["eligible"])

    def test_answer_prose_requires_review_and_missing_evidence_abstains(self):
        self.add(1, 6, "An alias of the second record", 1)
        prepared = prepare_pair(
            self.snapshot(), {"left": "urn:left", "right": "urn:right"}
        )
        self.assertFalse(prepared["eligible"])
        self.assertTrue(prepared["requires_content_review"])
        self.assertNotIn("An alias", json.dumps(prepared["state"]))
        self.db.execute("DELETE FROM facts WHERE e=2 AND a=7")
        self.db.commit()
        prepared = prepare_pair(
            self.snapshot(), {"left": "urn:left", "right": "urn:right"}
        )
        self.assertFalse(prepared["basic_evidence"])

    def test_read_only_and_nonempty_wal_refusal(self):
        with self.assertRaises(sqlite3.OperationalError):
            self.snapshot().db.execute("DELETE FROM facts")
        Path(str(self.path) + "-wal").write_bytes(b"pending")
        with self.assertRaisesRegex(ValueError, "nonempty WAL"):
            Snapshot(self.path)


if __name__ == "__main__":
    unittest.main()
