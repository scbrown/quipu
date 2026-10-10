"""Read-only, explicitly scoped historical evidence for the registered arm D.

Outputs are PRIVATE preparation artifacts, not a publication scrubber. No API
calls, entailment, alias canonicalization, or current-vector fallback occur here.
"""

import datetime as dt
import json
import math
import re
import sqlite3
import struct
from pathlib import Path

SAME = "http://www.w3.org/2002/07/owl#sameAs"
TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
LABEL = "http://www.w3.org/2000/01/rdf-schema#label"
COMMENT = "http://www.w3.org/2000/01/rdf-schema#comment"
IDENTITY = re.compile(
    r"^(sameas|same_as|exactmatch|closematch|altlabel|alternatelabel|"
    r"canonicalname|canonical_name|alias|aliases|aliasof|alias_of|distinctfrom|distinct_from|differentfrom)$",
    re.IGNORECASE,
)
ANSWER_PROSE = re.compile(
    r"\b(sameAs|same_as|alias(?:es|ed)?|duplicate(?:s|d)?|canonicaliz\w*|"
    r"same (?:real.world )?(?:entity|node|thing)|renamed (?:to|from)|"
    r"adjudicat\w*|never merge|spelling variant|canonical_name)\b",
    re.IGNORECASE,
)
PROVENANCE = re.compile(
    r"^(source|sourcekind|source_kind|wasderivedfrom|wasgeneratedby|"
    r"derived_from|repair|adjudication|resolution|confidence)$",
    re.IGNORECASE,
)


def instant(value):
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("timestamp must include timezone")
    return parsed


def local(iri):
    return iri.rsplit("/", 1)[-1].rsplit("#", 1)[-1]


def decode(blob, resolve):
    tag, data = blob[0], blob[1:]
    if tag == 0 and len(data) == 8:
        return {"iri": resolve(struct.unpack("<q", data)[0])}
    if tag == 1:
        return {"text": data.decode("utf-8")}
    if tag == 2 and len(data) == 8:
        return {"integer": struct.unpack("<q", data)[0]}
    if tag == 3 and len(data) == 8:
        value = struct.unpack("<d", data)[0]
        if not math.isfinite(value):
            raise ValueError("nonfinite value")
        return {"float": value}
    if tag == 4 and data in (b"\x00", b"\x01"):
        return {"boolean": data == b"\x01"}
    if tag in (6, 7) and len(data) >= 2:
        size = int.from_bytes(data[:2], "little")
        if size > len(data) - 2:
            raise ValueError("truncated prefixed value")
        return {
            "text": data[2 + size :].decode(),
            "language" if tag == 6 else "datatype": data[2 : 2 + size].decode(),
        }
    raise ValueError("unsupported or malformed value")


class Snapshot:
    def __init__(self, path, graph=0):
        path = Path(path).resolve()
        wal = Path(str(path) + "-wal")
        if wal.exists() and wal.stat().st_size:
            raise ValueError("checkpointed private copy required; nonempty WAL")
        self.db = sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)
        self.db.execute("PRAGMA query_only=ON")
        self.graph = graph
        self._terms = {}
        self._histories = {}
        self._identities = None

    def resolve(self, entity):
        if entity in self._terms:
            return self._terms[entity]
        row = self.db.execute("SELECT iri FROM terms WHERE id=?", (entity,)).fetchone()
        if row is None:
            raise ValueError("unknown term")
        self._terms[entity] = row[0]
        return row[0]

    def lookup(self, iri):
        row = self.db.execute("SELECT id FROM terms WHERE iri=?", (iri,)).fetchone()
        if row is None:
            raise ValueError("entity missing from source dictionary")
        return row[0]

    def history(self, entity):
        if entity not in self._histories:
            self._histories[entity] = self.db.execute(
                "SELECT a,v,tx,valid_from,valid_to,retracted_tx FROM facts "
                "WHERE e=? AND g=? AND op=1 ORDER BY a,v,tx",
                (entity, self.graph),
            ).fetchall()
        return self._histories[entity]

    def facts(self, entity, repair_tx, repair_time):
        """Immediately BEFORE both bounds; do not guess legacy retractions.

        In addition to the server's valid_at+tx predicate, require transaction
        liveness. A legacy closed row without retracted_tx is withheld even if
        valid time would admit it. Return its count as an availability limit.
        """
        boundary = instant(repair_time)
        kept, unknown = {}, {"legacy_closed": 0, "ambiguous_timestamp": 0}
        for a, v, tx, vf, vt, rt in self.history(entity):
            if tx >= repair_tx:
                continue
            try:
                started = instant(vf)
                ended = instant(vt) if vt is not None else None
            except ValueError:
                unknown["ambiguous_timestamp"] += 1
                continue
            if started >= boundary or (ended is not None and ended < boundary):
                continue
            if vt is not None and rt is None:
                unknown["legacy_closed"] += 1
                continue
            if rt is not None and rt < repair_tx:
                continue
            kept[(a, v)] = {
                "predicate": self.resolve(a),
                "value": decode(v, self.resolve),
                "tx": tx,
            }
        return list(kept.values()), unknown

    def identity_history(self):
        if self._identities is None:
            self._identities = self.db.execute(
                "SELECT e,v,tx,valid_from FROM facts WHERE a=? AND g=? AND op=1 "
                "ORDER BY tx,valid_from,e,v",
                (self.lookup(SAME), self.graph),
            ).fetchall()
        return self._identities

    def first_connection(self, left, right):
        """Earliest recorded ROOT identity connection, including transitive paths.

        Ever-asserted edges are deliberately retained for this adjudication
        cutoff: retracting an identity claim cannot erase knowledge of it.
        """
        parent = {}

        def root(x):
            while x in parent:
                x = parent[x]
            return x

        for e, v, tx, vf in self.identity_history():
            if len(v) != 9 or v[0] != 0:
                raise ValueError("non-reference identity edge")
            a, b = root(e), root(struct.unpack("<q", v[1:])[0])
            if a != b:
                parent[a] = b
            if root(left) == root(right):
                recorded = self.db.execute(
                    "SELECT timestamp FROM transactions WHERE id=?", (tx,)
                ).fetchone()[0]
                # If the repair was backdated, use its earlier effective date;
                # never extend the evidence window beyond recording time.
                boundary = min((vf, recorded), key=instant)
                return tx, boundary
        raise ValueError("no recorded identity connection in selected graph")


def clean(facts):
    kept, audit = [], []
    for fact in facts:
        predicate, value = fact["predicate"], fact["value"]
        reason = None
        if IDENTITY.fullmatch(local(predicate)):
            reason = "identity_relation"
        elif PROVENANCE.fullmatch(local(predicate)):
            reason = "source_annotation"
        elif ANSWER_PROSE.search(json.dumps(value, ensure_ascii=False)):
            reason = "answer_bearing_content_requires_review"
        if reason:
            audit.append({**fact, "reason": reason})
        else:
            kept.append(fact)
    return kept, audit


def endpoint(snapshot, entity, tx, at):
    facts, unknown = snapshot.facts(entity, tx, at)
    facts, removed = clean(facts)
    # All ordinary descriptions survive cleaning, even conflicting ones.
    labels, comments, types, edges = [], [], [], []
    for fact in facts:
        p, v = fact["predicate"], fact["value"]
        if p == LABEL and "text" in v:
            labels.append(v["text"])
        elif p == COMMENT and "text" in v:
            comments.append(v["text"])
        elif p == TYPE and "iri" in v:
            types.append(v["iri"])
        else:
            edges.append({"predicate": p, "value": v})
    edges.sort(key=lambda x: (x["predicate"], json.dumps(x["value"], sort_keys=True)))
    expanded = []
    for edge in edges[:12]:
        value = edge["value"]
        if "iri" in value:
            neighbours, _ = snapshot.facts(snapshot.lookup(value["iri"]), tx, at)
            neighbours, rejected = clean(neighbours)
            removed.extend(rejected)
            edge = {
                **edge,
                "neighbour": [
                    {"predicate": f["predicate"], "value": f["value"]}
                    for f in neighbours
                    if f["predicate"] in (TYPE, LABEL)
                ],
            }
        expanded.append(edge)
    description = "\n".join(sorted(set(comments)))
    record = {
        "labels": sorted(set(labels)),
        "description": description[:1024],
        "types": sorted(set(types)),
        "edges": expanded,
    }
    audit = {
        "history_withheld": unknown,
        "removed": removed,
        "description_truncated": len(description) > 1024,
        "edges_truncated": len(edges) > 12,
    }
    available = bool(types and (labels or comments))
    return record, audit, available


def prepare_pair(snapshot, pair):
    left, right = snapshot.lookup(pair["left"]), snapshot.lookup(pair["right"])
    tx, at = snapshot.first_connection(left, right)
    a, aa, usable_a = endpoint(snapshot, left, tx, at)
    b, ba, usable_b = endpoint(snapshot, right, tx, at)
    state = {"left": a, "right": b}
    encoded = json.dumps(state, ensure_ascii=False, sort_keys=True).encode()
    pending = any(
        f["reason"] == "answer_bearing_content_requires_review"
        for audit in (aa, ba)
        for f in audit["removed"]
    )
    return {
        "pair": pair,
        "repair_tx": tx,
        "valid_before": at,
        "graph": snapshot.graph,
        "state": state,
        "audit": [aa, ba],
        "basic_evidence": usable_a and usable_b,
        "requires_content_review": pending,
        "state_bytes": len(encoded),
        "eligible": usable_a and usable_b and not pending and len(encoded) < 12000,
    }
