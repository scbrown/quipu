#!/usr/bin/env python3
"""Prove the shipped pack can share outward without injecting receiver policy."""

import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import re


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


def verify(binary, share, db, work):
    query = (
        "SELECT (STR(?rule) AS ?iri) WHERE { ?rule a "
        "<http://aegis.gastown.local/ontology/InternalIdentifierPattern> }"
    )
    rows = iri_rows(binary, db, query)
    if not rows or any(
        not row.startswith("https://quipu.dev/knowledge/publication-policy/")
        for row in rows
    ):
        raise RuntimeError("receiver catalogue missing or private policy escaped the scope")

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
    if iri_rows(binary, final, query) != rows:
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
