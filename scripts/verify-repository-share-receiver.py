#!/usr/bin/env python3
"""Prove the shipped pack can share outward without injecting receiver policy."""

import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import re

POLICY_BASE = "https://quipu.dev/knowledge/publication-policy/"
AEGIS = "http://aegis.gastown.local/ontology/"
# Review pins: changes here and in the carried Turtle need boundary review.
EXPECTED_RULES = {
    POLICY_BASE + "access-tokens": (
        "Access token formats",
        "(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|AKIA[A-Z0-9]{16}|sk-ant-[A-Za-z0-9_-]{40,})",
        "block",
    ),
    POLICY_BASE + "private-keys": (
        "Private key headers", "-----BEGIN (?:[A-Z0-9]+ )?PRIVATE KEY-----", "block",
    ),
    POLICY_BASE + "user-directories": (
        "User home directory paths", "/(?:home|Users)/[A-Za-z0-9_.-]+/", "block",
    ),
    POLICY_BASE + "demo-private-domain": (
        "Fictional private domain control", "private[.]example", "block",
    ),
}


def run(binary, db, *args, expected=0):
    result = subprocess.run(
        [binary, *args, "--db", str(db)], capture_output=True, text=True, check=False,
    )
    if result.returncode != expected:
        raise RuntimeError(f"receiver {args[0]}: expected {expected}, got {result.returncode}")
    return result


def clone(source, destination):
    # Copy a coherent SQLite snapshot, including any outstanding WAL pages.
    with sqlite3.connect(f"file:{source}?mode=ro", uri=True) as original:
        with sqlite3.connect(destination) as target:
            original.backup(target)


def iri_rows(binary, db, query):
    # The native read command is a table, not the HTTP endpoint's JSON. Only
    # consume our single STR column, and require its complete result footer.
    text = run(binary, db, "query", query + " ORDER BY ?iri").stdout
    footer = re.search(r"\n(\d+) results\s*$", text)
    values = [json.loads(line) for line in text.splitlines() if line.startswith('"')]
    if not footer or int(footer[1]) != len(values):
        raise RuntimeError("incomplete native query result")
    return values


def catalogue(binary, db):
    rows = iri_rows(binary, db, "SELECT (STR(?rule) AS ?iri) WHERE { ?rule a "
                    f"<{AEGIS}InternalIdentifierPattern> }}")
    if set(rows) != set(EXPECTED_RULES) or len(rows) != len(EXPECTED_RULES):
        raise RuntimeError("receiver catalogue differs from the four reviewed IRIs")
    predicates = ["http://www.w3.org/2000/01/rdf-schema#label",
                  AEGIS + "regex", AEGIS + "enforcementTier"]
    for rule, values in EXPECTED_RULES.items():
        for predicate, expected in zip(predicates, values):
            actual = iri_rows(binary, db, "SELECT (STR(?value) AS ?iri) WHERE { "
                              f"<{rule}> <{predicate}> ?value }}")
            if actual != [expected]:
                raise RuntimeError("receiver rule differs from reviewed label/regex/block tier")
    return rows


def catalogue_controls(binary, work):
    # Isolated fixtures avoid staging copies concealing a missing ROOT rule.
    # Prove the same read path accepts the intact catalogue first.
    predicates = ["http://www.w3.org/2000/01/rdf-schema#label",
                  AEGIS + "regex", AEGIS + "enforcementTier"]
    changed = ["changed-label", "changed-regex", "changed-tier"]
    first = next(iter(EXPECTED_RULES))
    for case in ["positive", "missing-one", *changed]:
        triples = []
        for rule, values in EXPECTED_RULES.items():
            if case == "missing-one" and rule == first:
                continue
            triples.append(f"<{rule}> a <{AEGIS}InternalIdentifierPattern> .")
            for index, (predicate, value) in enumerate(zip(predicates, values)):
                if rule == first and case == changed[index]:
                    value = "changed-control"
                triples.append(f"<{rule}> <{predicate}> {json.dumps(value)} .")
        fixture = work / f"catalogue-{case}.ttl"
        fixture.write_text("\n".join(triples) + "\n")
        db = work / f"catalogue-{case}.db"
        run(binary, db, "knot", str(fixture))
        try:
            catalogue(binary, db)
        except RuntimeError:
            if case == "positive":
                raise
        else:
            if case != "positive":
                raise RuntimeError("altered catalogue passed acceptance")


def verify(binary, share, db, work):
    if re.search(r"(^|[^A-Za-z0-9_])aegis-[A-Za-z0-9]", (share / "export.nt").read_text()):
        raise RuntimeError("internal work identifiers escaped the public scope")
    query = (
        "SELECT (STR(?rule) AS ?iri) WHERE { ?rule a "
        "<http://aegis.gastown.local/ontology/InternalIdentifierPattern> }"
    )
    rows = catalogue(binary, db)
    catalogue_controls(binary, work)

    clean = work / "receiver-outward"
    run(binary, db, "share", "--output", str(clean))
    manifest = json.loads((clean / "manifest.json").read_text())
    if manifest.get("destination") is not None:
        raise RuntimeError("receiver proof must use the default outward destination")

    # Native receiver validates the carried shapes. No policy is injected.
    final = work / "roundtrip.db"
    run(binary, final, "shapes", "load", "roundtrip", str(clean / "shapes.ttl"))
    imported = json.loads(run(binary, final, "import", str(clean)).stdout)
    if imported["outcome"] != "staged" or imported["triples"]["quarantined"] != 0:
        raise RuntimeError("round-trip admission failed")
    run(binary, final, "import", "promote", imported["share_id"])
    if catalogue(binary, final) != rows:
        raise RuntimeError("round-trip catalogue changed")

    # A fresh receiver with shapes and application data but no policy must
    # still fail closed. Removing only ROOT rules is not this condition: an
    # imported store also retains a complete catalogue in its staging graph.
    empty = work / "receiver-no-policy.db"
    run(binary, empty, "shapes", "load", "control", str(share / "shapes.ttl"))
    positive = work / "application-control.ttl"
    positive.write_text('<urn:publication:control> <urn:publication:label> "control" .\n')
    run(binary, empty, "knot", str(positive))
    if iri_rows(binary, empty, query):
        raise RuntimeError("missing-policy fixture contains a catalogue")
    control = run(binary, empty, "query", "ASK { ?s ?p ?o }").stdout.strip()
    if control != "true":
        raise RuntimeError("missing-policy control lost its application data")
    output = work / "receiver-no-policy-output"
    refused = run(binary, empty, "share", "--output", str(output), expected=2)
    if "no block-tier patterns" not in refused.stderr or output.exists():
        raise RuntimeError("missing catalogue did not refuse without output")

    blocked = work / "receiver-blocked.db"
    clone(db, blocked)
    probe = work / "blocked-control.ttl"
    probe.write_text(
        '<urn:publication:control> <http://www.w3.org/2000/01/rdf-schema#label> '
        '"private.example" .\n'
    )
    run(binary, blocked, "knot", str(probe))
    output = work / "receiver-blocked-output"
    refused = run(binary, blocked, "share", "--output", str(output), expected=1)
    if "matched bytes" not in refused.stderr or output.exists():
        raise RuntimeError("blocked payload did not refuse without output")

    invalid = work / "receiver-invalid.db"
    clone(db, invalid)
    probe.write_text(
        '@prefix aegis: <http://aegis.gastown.local/ontology/> .\n'
        '@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n'
        '<urn:publication:incomplete-policy> a aegis:InternalIdentifierPattern ; '
        'rdfs:label "Incomplete policy control" .\n'
    )
    refused = subprocess.run(
        [binary, "knot", str(probe), "--shapes", str(share / "shapes.ttl"),
         "--db", str(invalid)],
        capture_output=True, text=True, check=False,
    )
    if refused.returncode == 0 or "SHACL" not in refused.stderr:
        raise RuntimeError("carried policy shapes did not reject an incomplete rule")
    if iri_rows(binary, invalid, query) != rows:
        raise RuntimeError("invalid rule was admitted")

    print(json.dumps({
        "result": "receiver acceptance passed", "catalogue_rules": len(rows),
        "outward_roundtrip": True, "missing_policy_refused": True,
        "blocked_bytes_refused": True, "native_shapes_refused_incomplete_rule": True,
        "exact_rule_values_pinned": True, "missing_one_refused": True,
        "changed_label_regex_tier_refused": True,
        "internal_work_ids_absent": True,
        "browser_shacl_claimed": False, "source_share": str(share),
    }))


if __name__ == "__main__":
    if len(sys.argv) != 5:
        sys.exit("usage: verify-repository-share-receiver.py BIN SHARE RECEIVER_DB WORK")
    try:
        verify(sys.argv[1], *(Path(arg) for arg in sys.argv[2:]))
    except (OSError, ValueError, KeyError, RuntimeError):
        # Errors may carry payload values. Do not put them in a hosted log.
        sys.exit("repository receiver acceptance failed; no release share may be emitted")
