# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy==2.3.3", "onnxruntime==1.23.0", "tokenizers==0.22.0"]
# ///
"""Rank PRIVATE historical candidates; emit evidence for independent review.

This is not a paid-call gate or a label adjudicator. All candidate labels stay
provisional until reviewed. No API or graph mutation is performed.
"""

import argparse
import collections
import json
import os
import re
import struct
from pathlib import Path

from baselines import Encoder, label, label_score, text
from temporal import TYPE, Snapshot, endpoint, local


def components(snapshot):
    parent = {}

    def root(x):
        while x in parent:
            x = parent[x]
        return x

    for left, value, *_ in snapshot.identity_history():
        right = struct.unpack("<q", value[1:])[0]
        a, b = root(left), root(right)
        if a != b:
            parent[a] = b
    return root


def distinguishing_evidence(snapshot, anchor, candidate, tx, at, left, right):
    # Explicit historical negative adjudication stays OUTSIDE model evidence.
    for a, b in ((anchor, candidate), (candidate, anchor)):
        facts, _ = snapshot.facts(a, tx, at)
        for fact in facts:
            if local(fact["predicate"]).lower() in (
                "distinctfrom",
                "differentfrom",
            ) and fact["value"].get("iri") == snapshot.resolve(b):
                return {"kind": "recorded_distinct", "fact": fact}
    # Two commit labels with incompatible hash prefixes cannot name one commit.
    # Use labels, not arbitrary hashes mentioned in a description about dependencies.
    if not any(
        local(t).lower() in ("commit", "gitcommit")
        for t in set(left["types"]) & set(right["types"])
    ):
        return None
    pattern = r"(?<![0-9a-f])[0-9a-f]{7,40}(?![0-9a-f])"
    aa, bb = (
        set(re.findall(pattern, label(left))),
        set(re.findall(pattern, label(right))),
    )
    if len(aa) != 1 or len(bb) != 1:
        return None
    a, b = next(iter(aa)), next(iter(bb))
    if a.startswith(b) or b.startswith(a):
        return None
    return {
        "kind": "incompatible_commit_identifiers",
        "left": a,
        "right": b,
        "source": "pre-repair asserted labels; same asserted commit type",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("db", "prepared", "model", "tokenizer", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError("refusing to overwrite a preparation artifact")
    snapshot = Snapshot(args.db)
    encoder = Encoder(args.model, args.tokenizer)
    np = encoder.np
    same_component = components(snapshot)
    prepared = json.loads(args.prepared.read_text())
    type_id = snapshot.lookup(TYPE)
    histories, vectors, candidates_by_type = {}, {}, {}
    records, positive_scores = [], []
    totals = collections.Counter()
    for number, pair in enumerate(prepared["records"]):
        if not pair["basic_evidence"]:
            continue
        tx, at = pair["repair_tx"], pair["valid_before"]
        anchor = snapshot.lookup(pair["pair"]["left"])
        state = pair["state"]["left"]
        pool = set()
        for kind in state["types"]:
            key = kind, tx
            if key not in candidates_by_type:
                blob = b"\x00" + struct.pack("<q", snapshot.lookup(kind))
                candidates_by_type[key] = [
                    r[0]
                    for r in snapshot.db.execute(
                        "SELECT DISTINCT e FROM facts WHERE a=? AND v=? AND g=0 AND op=1 AND tx<?",
                        (type_id, blob, tx),
                    )
                ]
            pool.update(candidates_by_type[key])
        usable = []
        for candidate in sorted(pool):
            if same_component(anchor) == same_component(candidate):
                continue
            key = candidate, tx, at
            if key not in histories:
                histories[key] = endpoint(snapshot, candidate, tx, at)
            record, audit, available = histories[key]
            if available and set(record["types"]) & set(state["types"]):
                usable.append((candidate, record, audit))
        texts = [text(state), text(pair["state"]["right"])] + [
            text(c[1]) for c in usable
        ]
        missing = list(dict.fromkeys(t for t in texts if t not in vectors))
        if missing:
            vectors.update(zip(missing, encoder.encode(missing), strict=True))
        anchor_vector = np.array(vectors[text(state)])
        positive_scores.append(
            {
                "index": pair["pair"]["index"],
                "label": label_score(label(state), label(pair["state"]["right"])),
                "cosine": float(
                    anchor_vector @ np.array(vectors[text(pair["state"]["right"])])
                ),
                "eligible": pair["eligible"],
            }
        )
        ranked = []
        for candidate, record, audit in usable:
            ls = label_score(label(state), label(record))
            cosine = float(anchor_vector @ np.array(vectors[text(record)]))
            ranked.append(
                (
                    max(ls if ls is not None else -1, cosine),
                    snapshot.resolve(candidate),
                    candidate,
                    record,
                    audit,
                    ls,
                    cosine,
                )
            )
        ranked.sort(key=lambda row: (-row[0], row[1]))
        for rank, (_, iri, candidate, record, audit, ls, cosine) in enumerate(
            ranked[:20], 1
        ):
            proof = distinguishing_evidence(
                snapshot, anchor, candidate, tx, at, state, record
            )
            pending = pair["requires_content_review"] or any(
                f["reason"] == "answer_bearing_content_requires_review"
                for f in audit["removed"]
            )
            records.append(
                {
                    "anchor_index": pair["pair"]["index"],
                    "class": pair["pair"]["class"],
                    "left": pair["pair"]["left"],
                    "right": iri,
                    "repair_tx": tx,
                    "valid_before": at,
                    "rank": rank,
                    "state": {"left": state, "right": record},
                    "candidate_audit": audit,
                    "label_score": ls,
                    "cosine": cosine,
                    "distinguishing_evidence": proof,
                    "requires_content_review": pending,
                }
            )
            totals["top20"] += 1
            totals["supported/" + pair["pair"]["class"]] += bool(proof and not pending)
        print(
            json.dumps(
                {
                    "anchor_done": number + 1,
                    "pool": len(pool),
                    "usable": len(usable),
                    "unique_texts_encoded": len(vectors),
                }
            ),
            flush=True,
        )
    output = {
        "schema": "arm-d-private-candidates-v1",
        "encoder": encoder.manifest,
        "summary": dict(totals),
        "positive_scores": positive_scores,
        "records": records,
    }
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(output, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps(dict(totals), indent=2))


if __name__ == "__main__":
    main()
