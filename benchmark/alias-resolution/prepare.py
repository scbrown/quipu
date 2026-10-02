"""Prepare PRIVATE evidence; never send a request or publish source identifiers."""

import argparse
import collections
import hashlib
import json
import os
from pathlib import Path

from temporal import Snapshot, prepare_pair


def validate_cohort(pairs, corpus):
    expected = []
    for kind in ("id-form", "semantic"):
        seen = set()
        for index, pair in enumerate(corpus["alias_pairs"]):
            if pair["class"] != kind or seen.intersection(
                (pair["left"], pair["right"])
            ):
                continue
            expected.append(index)
            seen.update((pair["left"], pair["right"]))
    if [p["index"] for p in pairs] != expected:
        raise ValueError("cohort differs from registered disjoint-endpoint selection")
    forward, reverse = {}, {}
    for pair in pairs:
        public = corpus["alias_pairs"][pair["index"]]
        if pair["class"] != public["class"]:
            raise ValueError("class differs from published corpus")
        for side in ("left", "right"):
            source, opaque = pair[side], pair["public_" + side]
            if opaque != public[side]:
                raise ValueError("endpoint differs from published corpus")
            if forward.setdefault(source, opaque) != opaque:
                raise ValueError("source topology was not preserved")
            if reverse.setdefault(opaque, source) != source:
                raise ValueError("mapping is not bijective")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=Path)
    parser.add_argument("--cohort", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--corpus",
        type=Path,
        default=Path(__file__).parent / "../replay/corpus/corpus.json",
    )
    args = parser.parse_args()
    pairs = json.loads(args.cohort.read_text())
    validate_cohort(pairs, json.loads(args.corpus.read_text()))
    snapshot = Snapshot(args.db, graph=0)
    records = [prepare_pair(snapshot, pair) for pair in pairs]
    summary = collections.Counter()
    for record in records:
        kind = record["pair"]["class"]
        summary[kind + "/total"] += 1
        for key in ("basic_evidence", "requires_content_review", "eligible"):
            summary[kind + "/" + key] += int(record[key])
    output = {
        "schema": "arm-d-private-preparation-v1",
        "graph": "ROOT",
        "corpus_sha256": hashlib.sha256(args.corpus.read_bytes()).hexdigest(),
        "summary": dict(summary),
        "records": records,
    }
    # Exclusive creation prevents an accidental rerun from replacing reviewed data.
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(output, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps(dict(summary), indent=2))


if __name__ == "__main__":
    main()
