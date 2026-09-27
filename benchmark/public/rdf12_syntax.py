#!/usr/bin/env python3
"""Pin the W3C RDF 1.2 syntax suites and record them as UNSUPPORTED (aegis-mhee08).

Quipu is built without RDF 1.2 (no `rdf-12` feature on oxrdf, oxttl or
spargebra), so it cannot parse a triple term. This script does NOT run Quipu.
It enumerates every case the pinned RDF 1.2 manifests add and records each one
as unsupported, with the reason, so the published table carries an honest
row instead of omitting RDF 1.2.

Running the suite anyway would be worse than not publishing it: a loader that
rejects all RDF 1.2 input "passes" every negative-syntax case. Those passes
would read as partial support, so nothing here is ever scored as a pass. The
work to support RDF 1.2 is aegis-6l8hkk; when it lands, these suites move to a
real runner.

Only the RDF 1.2 sub-manifests are enumerated. The top-level RDF 1.2 manifests
also include the RDF 1.1 manifests, which `rdf11_syntax.py` already scores.

    python3 benchmark/public/rdf12_syntax.py --suite <rdf-tests checkout> \\
        --output benchmark/public/results/rdf12-syntax.json
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path

import rdf11_syntax
from conformance_provenance import provenance

PINNED_SUITE_REVISION = rdf11_syntax.PINNED_SUITE_REVISION
REASON = "RDF 1.2 not implemented: Quipu is built without the rdf-12 feature (aegis-6l8hkk)"

# suite -> the RDF 1.2 sub-manifests it adds (never the included RDF 1.1 ones).
SUITES = {
    "rdf-turtle": ("syntax", "eval"),
    "rdf-n-triples": ("syntax", "c14n"),
    "rdf-n-quads": ("syntax", "c14n"),
    "rdf-trig": ("syntax", "eval"),
}


# RDF 1.2 manifests name cases with a prefixed name (`trs:turtle12-1`, or the
# empty prefix `:comment_following_triple` in the C14N manifests), not the
# RDF 1.1 manifests' `<#name>`, so they get their own small reader.
START = re.compile(r"(?m)^(\w*):([\w.-]+)\s+(?:rdf:type|a)\s+rdft:(Test\w+)")
ENTRIES = re.compile(r"(?ms)mf:entries\s*\((.*?)\)")
APPROVAL = re.compile(r"rdft:approval\s+rdft:(\w+)")


def kind_of(test_type: str) -> str:
    """RDF 1.2 adds canonicalisation cases; everything else is the 1.1 vocabulary."""
    if test_type.endswith("PositiveC14N"):
        return "c14n"
    return rdf11_syntax.kind_of(test_type)


def cases_of(manifest: str) -> list[dict]:
    starts = list(START.finditer(manifest))
    parsed = []
    for index, start in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(manifest)
        approval = APPROVAL.search(manifest[start.end():end])
        parsed.append({"name": start.group(2), "kind": kind_of(start.group(3)),
                       "approval": approval.group(1) if approval else "unmarked"})
    listed = ENTRIES.search(manifest)
    if not listed:
        raise ValueError("manifest has no mf:entries list")
    # Strip Turtle comments first: the C14N manifests comment out a listed case.
    listed_text = re.sub(r"(?m)#.*$", "", listed.group(1))
    entries = re.findall(r"(?:^|\s)\w*:([\w.-]+)", listed_text)
    found = [c["name"] for c in parsed]
    if sorted(entries) != sorted(found):
        missing = sorted(set(entries) - set(found))[:5]
        extra = sorted(set(found) - set(entries))[:5]
        raise ValueError(f"parsed cases do not match mf:entries (missing {missing}, extra {extra})")
    return parsed


def cases(suite_root: Path) -> list[dict]:
    rows = []
    for suite, parts in SUITES.items():
        for part in parts:
            manifest = (suite_root / "rdf" / "rdf12" / suite / part / "manifest.ttl").read_text()
            parsed = cases_of(manifest)
            if not parsed:
                raise ValueError(f"{suite}/{part}: manifest produced zero cases")
            for case in parsed:
                rows.append({
                    "class": "rdf12-syntax",
                    "manifest": f"rdf/rdf12/{suite}/{part}/manifest.ttl",
                    "id": case["name"],
                    "test": f"{suite}/{part}/{case['name']}",
                    "kind": case["kind"],
                    "approval": case["approval"],
                    "observed": "unsupported",
                    "passed": False,
                    "diagnostic": REASON,
                })
    return rows


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path, help="a W3C rdf-tests checkout")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    run = lambda *a: subprocess.run(  # noqa: E731
        ["git", "-C", str(args.suite), *a], check=True, text=True, capture_output=True
    ).stdout.strip()
    revision = run("rev-parse", "HEAD")
    if revision != PINNED_SUITE_REVISION or run("status", "--porcelain"):
        parser.error(f"suite must be clean at {PINNED_SUITE_REVISION}; got {revision}")

    rows = cases(args.suite)
    by_suite = {}
    for suite in SUITES:
        mine = [r for r in rows if r["test"].startswith(suite + "/")]
        by_suite[suite] = {
            "cases": len(mine), "passed": 0, "unsupported": len(mine),
            "by_kind": {k: sum(r["kind"] == k for r in mine) for k in sorted({r["kind"] for r in mine})},
        }
    report = {
        "benchmark": "W3C RDF 1.2 syntax suites (Turtle, N-Triples, N-Quads, TriG)",
        "suite_revision": revision,
        "scope": "enumerated, not run: every case is unsupported until aegis-6l8hkk",
        "reason": REASON,
        "totals": {"all": {"cases": len(rows), "passed": 0, "unsupported": len(rows)},
                   "suites": by_suite},
        "results": rows,
    }
    report["generated_at"], report["generated_by"] = provenance()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({s: v["cases"] for s, v in by_suite.items()}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
