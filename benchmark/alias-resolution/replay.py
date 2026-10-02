"""Offline scoring for stored arm-D decisions; never calls a model."""

import argparse
import collections
import json
import math
from pathlib import Path

MODEL = "jev-1.13.0"
CHOICES = {"same", "different", "cannot_tell"}


def probability(value):
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and 0 <= value <= 1
    )


def decision(raw, floor):
    if not isinstance(raw, dict) or raw.get("model") != MODEL:
        return "unavailable"
    answers = raw.get("answers")
    if not isinstance(answers, dict) or not isinstance(answers.get("q"), dict):
        return "unavailable"
    answer = answers["q"]
    probs = answer.get("probabilities", {})
    confidence = answer.get("confidence")
    choice = answer.get("choice")
    if (
        not isinstance(choice, str)
        or choice not in CHOICES
        or not isinstance(probs, dict)
        or set(probs) != CHOICES
        or not all(probability(p) for p in probs.values())
        or not probability(confidence)
        or not math.isclose(sum(probs.values()), 1.0, abs_tol=0.01)
        or probs[choice] < max(probs.values())
    ):
        return "unavailable"
    return "abstain" if choice == "cannot_tell" or confidence < floor else choice


def wilson(success, total):
    if not total:
        return None
    z = 1.959963984540054
    p = success / total
    denominator = 1 + z * z / total
    midpoint = (p + z * z / (2 * total)) / denominator
    margin = (
        z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / denominator
    )
    return [max(0, midpoint - margin), min(1, midpoint + margin)]


def metrics(items, predictions):
    count = collections.Counter(
        {
            k: 0
            for k in (
                "n",
                "positives",
                "negatives",
                "tp",
                "fp",
                "fn",
                "tn",
                "abstain",
                "unavailable",
            )
        }
    )
    for item, predicted in zip(items, predictions, strict=True):
        gold = item["gold"]
        if gold not in ("same", "different"):
            raise ValueError("invalid gold label")
        if predicted not in ("same", "different", "abstain", "unavailable"):
            raise ValueError("invalid prediction")
        count["n"] += 1
        count["positives" if gold == "same" else "negatives"] += 1
        if predicted in ("abstain", "unavailable"):
            count[predicted] += 1
        if gold == "same":
            count["tp" if predicted == "same" else "fn"] += 1
        elif predicted == "same":
            count["fp"] += 1
        elif predicted == "different":
            count["tn"] += 1
    proposals = count["tp"] + count["fp"]
    count.update({"identity_proposals": proposals, "actual_merges": 0})
    return {
        **dict(count),
        "precision": count["tp"] / proposals if proposals else None,
        "recall": count["tp"] / count["positives"] if count["positives"] else None,
        "abstention_rate": (count["abstain"] + count["unavailable"]) / count["n"]
        if count["n"]
        else None,
        "precision_wilson95": wilson(count["tp"], proposals),
        "recall_wilson95": wilson(count["tp"], count["positives"]),
    }


def baseline(item, name, threshold):
    if not item["eligible"]:
        return "unavailable"
    scores = item.get("baselines", {})
    values = [
        scores.get(k) for k in (("label", "cosine") if name == "union" else (name,))
    ]
    # Incomplete union stays unavailable, even if its observed component wins.
    if any(
        v is None or not isinstance(v, (float, int)) or not math.isfinite(v)
        for v in values
    ):
        return "unavailable"
    return "same" if any(v >= threshold for v in values) else "different"


def report(items, responses):
    ids = [item["id"] for item in items]
    if len(ids) != len(set(ids)):
        raise ValueError("duplicate item IDs")
    if set(responses) - set(ids):
        raise ValueError("response not in frozen item manifest")
    output = {}
    for group in ("all", "id-form", "semantic", "held-out"):
        selected = [
            i
            for i in items
            if group == "all"
            or i["class"] == group
            or (group == "held-out" and not i["pilot"])
        ]
        curves = []
        for step in range(21):
            floor = step / 20
            predictions = [
                decision(responses.get(i["id"]), floor)
                if i["eligible"]
                else "unavailable"
                for i in selected
            ]
            curves.append({"floor": floor, **metrics(selected, predictions)})
        free = {}
        for name in ("label", "cosine", "union", "id-control"):
            free[name] = [
                {
                    "threshold": step / 20,
                    **metrics(
                        selected, [baseline(i, name, step / 20) for i in selected]
                    ),
                }
                for step in range(21)
            ]
        output[group] = {"jev": curves, "free": free}
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--items", required=True, type=Path)
    parser.add_argument("--responses", required=True, type=Path)
    args = parser.parse_args()
    print(
        json.dumps(
            report(
                json.loads(args.items.read_text()),
                json.loads(args.responses.read_text()),
            ),
            indent=2,
            allow_nan=False,
        )
    )


if __name__ == "__main__":
    main()
