# Recompute the decision metrics without inference

From this directory, after the full response artifacts are published:

```sh
python3 replay.py --items proposed-items.json --responses responses.json > recomputed-curves.json
```

The default output includes the original denominators, both strata, and a
held-out-only group. To reproduce each stratum separately for the pilot and
held-out cohorts, with both recall denominators:

```python
import json
from pathlib import Path
from replay import report

items = json.loads(Path("proposed-items.json").read_text())
responses = json.loads(Path("responses.json").read_text())
for cohort in ("pilot", "held-out"):
    selected = [i for i in items if i["pilot"] == (cohort == "pilot")]
    for denominator in ("all-original", "eligible-only"):
        subset = [i for i in selected if denominator == "all-original" or i["eligible"]]
        ids = {i["id"] for i in subset}
        result = report(subset, {k: v for k, v in responses.items() if k in ids})
        Path(f"{cohort}-{denominator}.json").write_text(json.dumps(result, indent=2) + "\n")
```

Jev's registered point is floor `0.75`; the free baselines use threshold `0.90`.
Use the `id-form` and `semantic` groups in each result. An eligible-only recall
still counts model abstentions and unavailable requests as unrecovered aliases.
The all-original denominator additionally includes missing-evidence positives.

`tp` is a correct proposed identity, `fp` a false proposed identity, and
`actual_merges` is always zero. Negative abstentions and unavailable responses
are not true negatives. No-proposal precision is null. The `abstention_rate`
includes unavailable items; separate counts distinguish these cases. Scores are
frozen before model calls and thresholds are not selected from the results.

Availability and billing use the attempt ledgers, not confusion-matrix counts.
A valid low-confidence or cannot-tell response is available even when it abstains.
First-attempt availability counts each unique item's first attempt; eventual
availability counts whether an applicable bounded attempt policy produced any
valid response. Missing usage on a timeout means unknown possible cost, never
zero. The two pilot timeouts were not retried by subsequent authorizations.
