#!/usr/bin/env python3
"""Measure the W3C SPARQL 1.2 query tests honestly (aegis-mhee08).

Quipu is built without RDF 1.2, so most of the SPARQL 1.2 suite needs grammar
or terms it does not have: triple terms, the VERSION declaration, base
direction, the new codepoint escapes. Those cases are ENUMERATED and recorded
as unsupported with a named reason, never run. Running them would be worse
than not publishing them, because a parser that rejects all 1.2 input "passes"
every negative-syntax case, and those passes would read as partial support.

A few evaluation cases need nothing new: grouping, RDF 1.1 literal
clarifications, and one expression test. Those are RUN through the same
runner and comparison as the 1.1 and 1.0 suites, so they can pass or fail.
Every SPARQL 1.2 case is dawgt:Proposed at the pinned revision; the ledger
says so.

    uv run --with rdflib==7.6.0 python3 benchmark/public/sparql12.py \\
        --suite <rdf-tests checkout> --quipu <bin> \\
        --output benchmark/public/results/sparql12.json
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path

import sparql11_evaluation as evaluation
from conformance_provenance import provenance

PINNED_SUITE_REVISION = evaluation.PINNED_SUITE_REVISION

TRIPLE_TERMS = "needs RDF 1.2 triple terms: Quipu is built without rdf-12 (aegis-6l8hkk)"
# sub-manifest -> reason it is not run. A sub-manifest absent here is RUN.
NOT_RUN = {
    "syntax-triple-terms-positive": TRIPLE_TERMS,
    "syntax-triple-terms-negative": TRIPLE_TERMS,
    "eval-triple-terms": TRIPLE_TERMS,
    "version": "the SPARQL 1.2 VERSION declaration is not implemented",
    "lang-basedir": "RDF 1.2 base direction (rdf:dirLangString) is not implemented",
    "codepoint-escapes": "SPARQL 1.2 codepoint-escape grammar is not implemented; "
    "its negative cases would pass by rejection",
    "syntax": "not wired: the syntax runner scores the 1.1 syntax manifest only",
}
# Cases inside an otherwise-run sub-manifest that still need triple terms.
NOT_RUN_CASES = {
    ("expression", ":triple-on-literals"): TRIPLE_TERMS,
    ("expression", ":triple-on-str-literals"): TRIPLE_TERMS,
    ("expression", ":triple-on-triple-terms"): TRIPLE_TERMS,
    ("expression", ":triple-on-undefs"): TRIPLE_TERMS,
}
CASE = re.compile(r"(?m)^\s*(:[A-Za-z0-9_.-]+)\s+(?:rdf:type|a)\s+mf:(\w+)")


def sub_manifests(suite: Path) -> list[Path]:
    return evaluation.includes(suite / "sparql" / "sparql12" / "manifest.ttl")


ENTRIES = re.compile(r"(?s)mf:entries\s*\((.*?)\)")


def enumerate_cases(manifest: Path) -> list[tuple[str, str]]:
    """Every (identifier, test type) the sub-manifest lists, checked against mf:entries.

    The manifest resource itself (`:manifest a mf:Manifest`) is not a test.
    A mismatch with the manifest's own entry list refuses, rather than
    publishing a count that quietly drifted.
    """
    text = "\n".join(line for line in manifest.read_text().splitlines() if not line.lstrip().startswith("#"))
    cases = [(i, kind) for i, kind in CASE.findall(text) if kind != "Manifest"]
    listed = ENTRIES.search(text)
    entries = sorted(re.findall(r"(:[A-Za-z0-9_.-]+)", listed.group(1))) if listed else []
    if sorted(i for i, _ in cases) != entries:
        raise ValueError(f"{manifest}: parsed cases do not match mf:entries")
    return cases


def measure(suite: Path, quipu: Path, server: Path) -> list[dict]:
    rows = []
    for manifest in sub_manifests(suite):
        part = manifest.parent.name
        runnable = {}
        if part not in NOT_RUN:
            # The runner reads only Approved cases. No 1.2 case is Approved:
            # most are Proposed and some carry no approval at all, so accept any.
            approved, evaluation.APPROVED = evaluation.APPROVED, ""
            try:
                runnable = {c.identifier: c for c in evaluation.parse_manifest("sparql12", manifest)}
            finally:
                evaluation.APPROVED = approved
        for identifier, kind in enumerate_cases(manifest):
            base = {"class": "sparql12", "manifest": f"sparql/sparql12/{part}/manifest.ttl",
                    "id": identifier, "kind": kind, "approval": "Proposed"}
            reason = NOT_RUN.get(part) or NOT_RUN_CASES.get((part, identifier))
            case = runnable.get(identifier)
            if reason or case is None:
                rows.append({**base, "status": "unsupported",
                             "reason": reason or f"test type {kind} is not executable by this runner"})
                continue
            result = evaluation.run_case(case, quipu, server)
            rows.append({**base, "status": result["status"],
                         "diagnostic": result.get("diagnostic") or result.get("reason", "")})
    return rows


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path, help="a W3C rdf-tests checkout")
    parser.add_argument("--quipu", default="quipu", type=Path)
    parser.add_argument("--server", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    git = lambda *a: subprocess.run(  # noqa: E731
        ["git", "-C", str(args.suite), *a], check=True, text=True, capture_output=True
    ).stdout.strip()
    revision = git("rev-parse", "HEAD")
    if revision != PINNED_SUITE_REVISION or git("status", "--porcelain"):
        parser.error(f"suite must be clean at {PINNED_SUITE_REVISION}; got {revision}")
    quipu = evaluation.executable_path(args.quipu)
    server = evaluation.executable_path(args.server or quipu.with_name("quipu-server"))

    rows = measure(args.suite, quipu, server)
    by_part: dict[str, dict[str, int]] = {}
    for row in rows:
        part = row["manifest"].split("/")[2]
        counts = by_part.setdefault(part, {"cases": 0})
        counts["cases"] += 1
        counts[row["status"]] = counts.get(row["status"], 0) + 1
    totals = {"cases": len(rows)}
    for row in rows:
        totals[row["status"]] = totals.get(row["status"], 0) + 1
    quipu_root = Path(__file__).resolve().parents[2]
    report = {
        "benchmark": "W3C SPARQL 1.2 query tests (all dawgt:Proposed at the pinned revision)",
        "suite_revision": revision,
        "quipu_revision": subprocess.run(["git", "-C", str(quipu_root), "rev-parse", "HEAD"],
                                         check=True, text=True, capture_output=True).stdout.strip(),
        "scope": "cases needing RDF 1.2 grammar or terms are enumerated, not run; the rest are run",
        "totals": {"all": totals, "parts": dict(sorted(by_part.items()))},
        "results": rows,
    }
    report["generated_at"], report["generated_by"] = provenance()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(totals, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
