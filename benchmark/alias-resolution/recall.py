# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy==2.3.3", "onnxruntime==1.23.0", "tokenizers==0.22.0"]
# ///
"""Blocking recall of free top-k candidate generation over the 93 alias pairs.

Anchor = left endpoint. Candidates = entities asserting one of the anchor's
types before the repair transaction, ranked by max(label score, MiniLM cosine)
over the SAME cleaned pre-repair evidence as arm D. No API call, no current
vector, no graph mutation. The denominator stays every cohort pair.

STRICT rank (primary) lets every other usable candidate compete and counts ties
against the partner; it needs no post-repair knowledge. LENIENT rank also drops
competitors recorded as aliases of the anchor at any time, which uses later
knowledge and is reported only as a secondary bound.
"""

import argparse
import collections
import json
import os
import statistics
from pathlib import Path

from baselines import Encoder, label, label_score, text
from candidates import components
from temporal import SAME, TYPE, Snapshot, endpoint

KS = (1, 5, 10, 20, 50)
SIGNALS = ("max", "label", "cosine")


def rank(partner, others):
    """1 + competitors scoring at least the partner: ties count against it."""
    return 1 + sum(score >= partner for score in others)


def available(record):
    return bool(record["types"] and (record["labels"] or record["description"]))


def published_encoder(manifest, provenance):
    """Hashes live in PROVENANCE.md; refuse an encoder that is not the published one."""
    text = provenance.read_text()
    for key in ("model_sha256", "tokenizer_sha256"):
        if f"`{manifest[key]}`" not in text:
            raise ValueError(f"{key} differs from {provenance.name}")
    kept = {k: v for k, v in manifest.items() if not k.endswith("_sha256")}
    return {**kept, "file_hashes": f"{provenance.name}, verified equal at run time"}


def summarize(rows):
    out = {}
    for kind in ("id-form", "semantic", "all"):
        chosen = [r for r in rows if kind == "all" or r["class"] == kind]
        entry = {
            "total": len(chosen),
            "status": dict(collections.Counter(r["status"] for r in chosen)),
        }
        for mode in ("strict", "lenient"):
            for signal in SIGNALS:
                ranks = [r["rank"][mode][signal] for r in chosen if r["rank"]]
                entry[f"recall_{mode}_{signal}"] = {
                    str(k): sum(x <= k for x in ranks) for k in KS
                }
        pools = [r["pool_usable"] for r in chosen if r["pool_usable"] is not None]
        if pools:
            entry["pool_usable"] = {
                "min": min(pools),
                "median": statistics.median(pools),
                "max": max(pools),
            }
        out[kind] = entry
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("db", "prepared", "model", "tokenizer", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError("refusing to overwrite a result artifact")
    snapshot = Snapshot(args.db)
    encoder = Encoder(args.model, args.tokenizer)
    manifest = published_encoder(encoder.manifest, Path(__file__).parent / "PROVENANCE.md")
    np = encoder.np
    same_component = components(snapshot)
    prepared = json.loads(args.prepared.read_text())
    type_id = snapshot.lookup(TYPE)
    typed, histories, vectors, rows = {}, {}, {}, []

    def vector(record):
        return np.array(vectors[text(record)])

    for pair in prepared["records"]:
        tx, at = pair["repair_tx"], pair["valid_before"]
        anchor = snapshot.lookup(pair["pair"]["left"])
        partner = snapshot.lookup(pair["pair"]["right"])
        state = pair["state"]["left"]
        row = {
            "index": pair["pair"]["index"],
            "class": pair["pair"]["class"],
            "pool_usable": None,
            "rank": None,
        }
        rows.append(row)
        if not available(state):
            row["status"] = "no_anchor_evidence"
            continue
        pool = set()
        for kind in state["types"]:
            if (kind, tx) not in typed:
                blob = b"\x00" + snapshot.lookup(kind).to_bytes(8, "little", signed=True)
                typed[kind, tx] = [
                    r[0]
                    for r in snapshot.db.execute(
                        "SELECT DISTINCT e FROM facts WHERE a=? AND v=? AND g=0 AND op=1 AND tx<?",
                        (type_id, blob, tx),
                    )
                ]
            pool.update(typed[kind, tx])
        pool.discard(anchor)
        usable = {}
        for candidate in pool:
            key = candidate, tx, at
            if key not in histories:
                histories[key] = endpoint(snapshot, candidate, tx, at)
            record, _, ok = histories[key]
            if ok and set(record["types"]) & set(state["types"]):
                usable[candidate] = record
        row["pool_usable"] = len(usable)
        if partner not in pool:
            row["status"] = "partner_not_in_typed_pool"
            continue
        if partner not in usable:
            row["status"] = "partner_without_usable_evidence"
            continue
        # Control: the partner's view is exactly arm D's frozen right-hand view.
        if usable[partner] != pair["state"]["right"]:
            raise ValueError("partner evidence differs from the prepared arm D view")
        for record in [state, *usable.values()]:
            if any(e["predicate"] == SAME for e in record["edges"]):
                raise ValueError("identity edge reached ranking evidence")
        texts = [text(state)] + [text(r) for r in usable.values()]
        missing = list(dict.fromkeys(t for t in texts if t not in vectors))
        if missing:
            vectors.update(zip(missing, encoder.encode(missing), strict=True))
        av = vector(state)
        scores = {}
        for candidate, record in usable.items():
            ls = label_score(label(state), label(record))
            ls = -1.0 if ls is None else ls
            cos = float(av @ vector(record))
            scores[candidate] = {"max": max(ls, cos), "label": ls, "cosine": cos}
        aliases = {c for c in usable if same_component(c) == same_component(anchor)}
        row["rank"] = {
            mode: {
                signal: rank(
                    scores[partner][signal],
                    [
                        s[signal]
                        for c, s in scores.items()
                        if c != partner and (mode == "strict" or c not in aliases)
                    ],
                )
                for signal in SIGNALS
            }
            for mode in ("strict", "lenient")
        }
        row["known_alias_competitors"] = len(aliases - {partner})
        row["status"] = "ranked"
        print(json.dumps({"index": row["index"], "rank": row["rank"]["strict"]}), flush=True)

    output = {
        "schema": "arm-d-blocking-recall-v1",
        "anchor": "left endpoint",
        "score": "max(normalized label score, MiniLM cosine); ties count against the partner",
        "ks": list(KS),
        "encoder": manifest,
        "summary": summarize(rows),
        "rows": rows,
    }
    # Rows carry only public corpus indexes, classes, statuses and ranks.
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    with os.fdopen(fd, "w") as stream:
        json.dump(output, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps(output["summary"], indent=2))


if __name__ == "__main__":
    main()
