#!/usr/bin/env python3
"""Score Quipu's parser against the approved W3C SPARQL 1.1 syntax manifest."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import tempfile
from pathlib import Path

PINNED_SUITE_REVISION = "369a90d1a60c021b746df2e411da0ff36258a758"
BLOCK = re.compile(r"(?ms)^:[^\s]+\s+rdf:type\s+mf:(PositiveSyntaxTest11|NegativeSyntaxTest11)\s*;(.*?)(?=^:[^\s]+\s+rdf:type|\Z)")
ACTION = re.compile(r"mf:action\s+<([^>]+)>")


def approved_cases(manifest: str) -> list[tuple[str, bool]]:
    cases: list[tuple[str, bool]] = []
    for kind, body in BLOCK.findall(manifest):
        if "dawgt:approval dawgt:Approved" not in body:
            continue
        action = ACTION.search(body)
        if action:
            cases.append((action.group(1), kind == "PositiveSyntaxTest11"))
    return cases


def git_output(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(cwd), *args], check=True, text=True, capture_output=True
    ).stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path)
    parser.add_argument("--quipu", default="quipu", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--allow-unpinned-suite", action="store_true")
    args = parser.parse_args()

    revision = git_output(args.suite, "rev-parse", "HEAD")
    dirty = git_output(args.suite, "status", "--porcelain")
    if not args.allow_unpinned_suite and (revision != PINNED_SUITE_REVISION or dirty):
        parser.error(f"suite must be clean at {PINNED_SUITE_REVISION}; got {revision}")

    manifest_path = args.suite / "manifest.ttl"
    cases = approved_cases(manifest_path.read_text())
    if not cases:
        parser.error("manifest produced zero approved SPARQL 1.1 syntax cases")

    version = subprocess.run(
        [str(args.quipu), "--version"], check=True, text=True, capture_output=True
    ).stdout.strip()
    quipu_root = Path(__file__).resolve().parents[2]
    quipu_revision = git_output(quipu_root, "rev-parse", "HEAD")

    results = []
    with tempfile.TemporaryDirectory(prefix="quipu-w3c-syntax-") as tmp:
        database = Path(tmp) / "suite.db"
        for relative, positive in cases:
            query = (args.suite / relative).read_text()
            run = subprocess.run(
                [str(args.quipu), "read", query, "--db", str(database)],
                text=True,
                capture_output=True,
            )
            parse_error = "SPARQL parse error" in run.stderr
            passed = not parse_error if positive else parse_error
            results.append(
                {
                    "test": relative,
                    "expect": "accept" if positive else "reject",
                    "observed": "reject" if parse_error else "accept",
                    "passed": passed,
                    "diagnostic": run.stderr.strip() if parse_error else "",
                }
            )

    passed = sum(item["passed"] for item in results)
    report = {
        "benchmark": "W3C RDF Tests SPARQL 1.1 approved query syntax",
        "suite_revision": revision,
        "quipu_revision": quipu_revision,
        "quipu_version": version,
        "scope": "parser syntax only; evaluation and protocol excluded",
        "totals": {"passed": passed, "failed": len(results) - passed, "cases": len(results)},
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["totals"], sort_keys=True))
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
