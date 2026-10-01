import unittest

import tempfile
from pathlib import Path

from recall import available, published_encoder, rank, summarize


def row(kind, status, strict=None, lenient=None):
    ranks = None
    if strict is not None:
        ranks = {
            "strict": {s: strict for s in ("max", "label", "cosine")},
            "lenient": {s: lenient for s in ("max", "label", "cosine")},
        }
    return {"class": kind, "status": status, "rank": ranks, "pool_usable": 3}


class RecallTest(unittest.TestCase):
    def test_ties_count_against_the_partner(self):
        self.assertEqual(rank(0.5, [0.4, 0.3]), 1)
        self.assertEqual(rank(0.5, [0.5, 0.3]), 2)
        self.assertEqual(rank(0.5, [0.9, 0.5, 0.5]), 4)

    def test_unranked_pairs_stay_in_the_denominator(self):
        rows = [
            row("semantic", "ranked", 1, 1),
            row("semantic", "ranked", 7, 3),
            row("semantic", "no_anchor_evidence"),
        ]
        out = summarize(rows)["semantic"]
        self.assertEqual(out["total"], 3)
        self.assertEqual(out["recall_strict_max"], {"1": 1, "5": 1, "10": 2, "20": 2, "50": 2})
        self.assertEqual(out["recall_lenient_max"]["5"], 2)
        self.assertEqual(summarize(rows)["id-form"]["total"], 0)

    def test_availability_matches_arm_d(self):
        self.assertFalse(available({"types": [], "labels": ["x"], "description": ""}))
        self.assertFalse(available({"types": ["t"], "labels": [], "description": ""}))
        self.assertTrue(available({"types": ["t"], "labels": [], "description": "d"}))

    def test_encoder_must_be_the_published_one(self):
        manifest = {"model_sha256": "a" * 64, "tokenizer_sha256": "b" * 64, "x": 1}
        with tempfile.TemporaryDirectory() as d:
            prov = Path(d, "PROVENANCE.md")
            prov.write_text(f"`{'a' * 64}`\n`{'b' * 64}`\n")
            out = published_encoder(manifest, prov)
            self.assertEqual(out["x"], 1)
            self.assertNotIn("model_sha256", out)
            prov.write_text(f"`{'a' * 64}`\n")
            with self.assertRaises(ValueError):
                published_encoder(manifest, prov)


if __name__ == "__main__":
    unittest.main()
