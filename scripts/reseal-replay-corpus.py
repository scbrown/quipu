#!/usr/bin/env python3
"""Re-label an existing replay corpus with unguessable names.

WHY THIS EXISTS — the corpus was pseudonymised, and the pseudonymisation was
REVERSIBLE BY ANYONE HOLDING THE ARTIFACT.

`build-replay-corpus.py` named each entity `sha256(salt + iri)[:10]` with the
salt defaulting to a literal committed in that same public file. The source
namespace was meant to be the secret half, but it appears in 66 files of this
repository. So both inputs to the digest shipped with the artifact, and a
candidate IRI could be CONFIRMED by recomputing the hash and looking for it in
`corpus.json`.

That is not theoretical. A ~60-word hand-typed wordlist recovered 41 real
entity names in one pass — host names, service names and the full crew roster —
from a corpus whose own provenance line reads "pseudonymised" and which passes
every pattern-matching scrub gate clean. A regex sweep cannot see this class at
all: there is no forbidden string in the file. The leak is in the FUNCTION, not
in the bytes.

THE FIX IS TO REMOVE THE PREIMAGE, NOT TO HIDE THE SALT. A digest invites the
question "where is the key"; a random label has no key and no preimage. Labels
here are drawn from `secrets` and the mapping is never written down — not to
disk, not to stdout — so this script cannot be run "in reverse" even by the
person who ran it.

The rename is STRUCTURE-PRESERVING and that is checked, not asserted: same pair
count, same class split, same doubled-subject count, same multiset of comment
values, and an injective mapping. Every number in `benchmark/replay/BUILD_REPORT.md`
therefore still regenerates from the resealed fixture.

  python3 scripts/reseal-replay-corpus.py                    # reseal in place
  python3 scripts/reseal-replay-corpus.py --check            # is it still reversible?
  python3 scripts/reseal-replay-corpus.py --selftest         # prove both outcomes
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
import secrets
import sys
from pathlib import Path

DEFAULT = Path("benchmark/replay/corpus/corpus.json")
PUB = "https://example.org/kg/"

# The salt and namespace that were used to build the leaking corpus, recorded
# here in the remediation so `--check` can prove the leak is closed.
#
# Both are written out WHOLE, deliberately. The first draft assembled the
# namespace from fragments to keep it out of a grep for internal names, which is
# the move this repository already warns against in build-replay-corpus.py:
# splitting a literal to get past a checker also hides it from the next person
# who searches for it, and a string nobody can find is worse than one that is
# merely present. Neither value is a secret — that both of them are public IS
# the finding — and the pre-push guard does not govern this namespace, which
# already appears in 66 files here. Nothing is protected by obscuring it; the
# check that depends on it just becomes unreadable.
LEAK_SALT = "mergebench-replay-v1"
LEAK_NS = "http://aegis.gastown.local/ontology/"


def labels(doc: dict) -> set[str]:
    """Every pseudonym the corpus mentions, from every position it can occupy."""
    out: set[str] = set()
    for p in doc.get("alias_pairs", []):
        out.add(p["left"])
        out.add(p["right"])
    for c in doc.get("comment_doublings", []):
        out.add(c["subject"])
    return out


def fingerprint(doc: dict) -> dict:
    """The structure that must survive a reseal, reduced to comparable values.

    Deliberately does NOT include the labels themselves — those are what change.
    Everything the merge operator reasons about is a cardinality or a class, and
    all of it is here, so a reseal that altered the corpus's meaning cannot pass.
    """
    return {
        "n_pairs": len(doc.get("alias_pairs", [])),
        "classes": dict(collections.Counter(
            p["class"] for p in doc.get("alias_pairs", []))),
        "n_doubled": len(doc.get("comment_doublings", [])),
        "value_multiset": dict(collections.Counter(
            v for c in doc.get("comment_doublings", []) for v in c["values"])),
        "n_labels": len(labels(doc)),
        # Degree per label: preserves the alias-chain topology the build report
        # reasons about (12 chained pairs were excluded on exactly this basis).
        "degree_histogram": dict(collections.Counter(sorted(
            collections.Counter(
                x for p in doc.get("alias_pairs", []) for x in (p["left"], p["right"])
            ).values()))),
        "counts": doc.get("counts"),
    }


def reseal(doc: dict) -> dict:
    """Return doc with every label replaced by a fresh unguessable one."""
    old = labels(doc)
    mapping: dict[str, str] = {}
    used: set[str] = set()
    for iri in sorted(old):                       # sorted: deterministic ORDER,
        kind = iri[len(PUB):].split("-", 1)[0] if iri.startswith(PUB) else "entity"
        while True:
            new = f"{PUB}{kind}-{secrets.token_hex(5)}"
            if new not in used and new not in old:
                break                             # never collide, never reuse an
            continue                              # old name (which would be a hint)
        used.add(new)
        mapping[iri] = new

    out = json.loads(json.dumps(doc))             # deep copy, key order preserved
    for p in out.get("alias_pairs", []):
        p["left"] = mapping[p["left"]]
        p["right"] = mapping[p["right"]]
    for c in out.get("comment_doublings", []):
        c["subject"] = mapping[c["subject"]]
    out["provenance"] = (
        "aegis production knowledge graph; entity names replaced by random "
        "labels, mapping discarded at build time and not recoverable"
    )
    # mapping goes out of scope here and is never persisted. That is the point.
    return out


def oracle_hits(doc: dict, words: list[str]) -> list[str]:
    """Candidate names CONFIRMED present under the old salted-digest scheme.

    This is the leak, executable. Non-empty means the corpus is still reversible.
    """
    present = labels(doc)
    hits = []
    for w in words:
        h = hashlib.sha256((LEAK_SALT + LEAK_NS + w).encode()).hexdigest()[:10]
        if f"{PUB}entity-{h}" in present:
            hits.append(w)
    return hits


# A wordlist an outsider could plausibly assemble: project names visible on the
# public remote, plus ordinary English words. It is deliberately SMALL — the
# finding is that a trivial list works, so a large one would understate it.
WORDS = """quipu bobbin hank yupana shantytown shanty skein beads gastown aegis
reactor tapestry mayor prometheus grafana loki ntfy traefik forgejo librechat
pushgateway alertmanager automation dolt Dolt Quipu desire-path""".split()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus", nargs="?", default=str(DEFAULT), type=Path)
    ap.add_argument("--check", action="store_true",
                    help="exit 1 if the corpus is still reversible")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    if not args.corpus.exists():
        print(f"reseal: no corpus at {args.corpus}", file=sys.stderr)
        return 2
    doc = json.loads(args.corpus.read_text())

    if args.check:
        hits = oracle_hits(doc, WORDS)
        # A zero from an instrument that cannot fire is not a pass. Prove the
        # oracle WORKS before reading its silence as safety: reseal a copy under
        # the KNOWN-LEAKING scheme and confirm the check catches it.
        probe = {"alias_pairs": [{"class": "semantic",
                                  "left": f"{PUB}entity-" + hashlib.sha256(
                                      (LEAK_SALT + LEAK_NS + "quipu").encode()
                                  ).hexdigest()[:10],
                                  "right": f"{PUB}entity-0000000000"}],
                 "comment_doublings": []}
        if not oracle_hits(probe, WORDS):
            print("reseal --check: CONTROL FAILED — the oracle cannot detect a "
                  "known-leaking corpus, so its silence proves nothing.",
                  file=sys.stderr)
            return 2
        print(f"control ok  — oracle detects a known-leaking corpus")
        if hits:
            print(f"REVERSIBLE  — {len(hits)}/{len(WORDS)} candidates confirmed: "
                  f"{' '.join(hits)}", file=sys.stderr)
            return 1
        print(f"ok          — {len(WORDS)} candidates, 0 confirmed; labels carry "
              f"no preimage")
        return 0

    before = fingerprint(doc)
    out = reseal(doc)
    after = fingerprint(out)
    if before != after:
        print("reseal: REFUSING to write — the reseal changed the corpus's "
              "structure, not just its labels.", file=sys.stderr)
        for k in before:
            if before[k] != after[k]:
                print(f"  {k}: {before[k]!r} -> {after[k]!r}", file=sys.stderr)
        return 2
    if oracle_hits(out, WORDS):
        print("reseal: REFUSING to write — output is still reversible.",
              file=sys.stderr)
        return 2
    args.corpus.write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
    print(f"resealed {after['n_labels']} labels in {args.corpus}")
    print(f"structure preserved: {after['n_pairs']} pairs {after['classes']}, "
          f"{after['n_doubled']} doubled subjects")
    return 0


def selftest() -> int:
    """Both outcomes, on a corpus built to be reversible."""
    fail = 0
    leaky = {
        "alias_pairs": [], "comment_doublings": [],
        "counts": {"x": 1}, "provenance": "p", "schema": "s",
    }
    def lab(w):
        return f"{PUB}entity-" + hashlib.sha256(
            (LEAK_SALT + LEAK_NS + w).encode()).hexdigest()[:10]
    leaky["alias_pairs"] = [
        {"class": "semantic", "left": lab("quipu"), "right": lab("bobbin")},
        {"class": "id-form", "left": lab("hank"), "right": lab("skein")},
    ]
    leaky["comment_doublings"] = [
        {"subject": lab("reactor"), "values": ["a", "b"]},
        {"subject": lab("mayor"), "values": ["a", "b", "c"]},
    ]

    hits = oracle_hits(leaky, WORDS)
    if len(hits) == 6:
        print(f"ok   oracle recovers all 6 planted names: {' '.join(sorted(hits))}")
    else:
        print(f"FAIL oracle recovered {len(hits)}/6: {hits}"); fail = 1

    sealed = reseal(leaky)
    if not oracle_hits(sealed, WORDS):
        print("ok   resealed corpus recovers 0")
    else:
        print("FAIL resealed corpus is still reversible"); fail = 1

    if fingerprint(leaky) == fingerprint(sealed):
        print("ok   structure preserved across reseal")
    else:
        print("FAIL reseal changed structure"); fail = 1

    if not (labels(leaky) & labels(sealed)):
        print("ok   no original label survives")
    else:
        print("FAIL a label survived the reseal"); fail = 1

    # Two reseals must differ — a deterministic 'random' label would be the same
    # defect wearing a different coat.
    if labels(reseal(leaky)) != labels(reseal(leaky)):
        print("ok   labels are non-deterministic across runs")
    else:
        print("FAIL reseal is deterministic"); fail = 1

    print("selftest PASSED" if not fail else "selftest FAILED")
    return fail


if __name__ == "__main__":
    raise SystemExit(main())
