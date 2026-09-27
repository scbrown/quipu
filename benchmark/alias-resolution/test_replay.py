import math
import unittest

from replay import MODEL, baseline, decision, metrics, report


def response(choice="same", confidence=0.8):
    return {
        "model": MODEL,
        "answers": {
            "q": {
                "choice": choice,
                "confidence": confidence,
                "probabilities": {
                    k: 0.8 if k == choice else 0.1
                    for k in ("same", "different", "cannot_tell")
                },
            }
        },
    }


class Scoring(unittest.TestCase):
    def test_floor_and_explicit_abstention(self):
        self.assertEqual(decision(response(confidence=0.75), 0.75), "same")
        self.assertEqual(decision(response(confidence=0.749), 0.75), "abstain")
        self.assertEqual(decision(response("cannot_tell"), 0), "abstain")

    def test_invalid_is_unavailable_not_negative(self):
        for raw in (
            None,
            {},
            {"model": "different"},
            response(confidence=math.nan),
            response(confidence=True),
            response(confidence=1.1),
        ):
            self.assertEqual(decision(raw, 0.75), "unavailable")
        raw = response()
        del raw["answers"]["q"]["probabilities"]["different"]
        self.assertEqual(decision(raw, 0.75), "unavailable")
        for value in (None, [], "text"):
            self.assertEqual(
                decision({"model": MODEL, "answers": value}, 0.75), "unavailable"
            )
            raw = response()
            raw["answers"]["q"]["probabilities"] = value
            self.assertEqual(decision(raw, 0.75), "unavailable")

    def test_missing_positives_remain_in_denominator(self):
        items = [{"gold": "same"}] * 3 + [{"gold": "different"}] * 3
        result = metrics(
            items, ["same", "abstain", "unavailable", "same", "different", "abstain"]
        )
        self.assertEqual(result["recall"], 1 / 3)
        self.assertEqual(result["precision"], 1 / 2)
        self.assertEqual((result["fp"], result["tn"], result["fn"]), (1, 1, 2))
        self.assertEqual(result["actual_merges"], 0)
        self.assertIsNone(metrics(items[:1], ["different"])["precision"])

    def test_union_requires_both_components(self):
        item = {"eligible": True, "baselines": {"label": 1}}
        self.assertEqual(baseline(item, "union", 0.9), "unavailable")
        item["baselines"]["cosine"] = 0
        self.assertEqual(baseline(item, "union", 0.9), "same")

    def test_replay_retains_heldout_and_rejects_extra_responses(self):
        items = [
            {
                "id": "x",
                "gold": "same",
                "class": "semantic",
                "pilot": False,
                "eligible": True,
                "baselines": {"label": 0.5, "cosine": 0.5},
            }
        ]
        result = report(items, {"x": response()})
        self.assertEqual(result["held-out"]["jev"][15]["tp"], 1)
        with self.assertRaises(ValueError):
            report(items, {"unknown": response()})


if __name__ == "__main__":
    unittest.main()
