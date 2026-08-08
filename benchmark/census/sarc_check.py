#!/usr/bin/env python3
"""CEN-X1's external arm: score a Census sarc-export against the SARC
reference checker (github.com/besanson/sarc-governance).

The Census run writes ``out/sarc-export/{spec.yaml, trace-faithful.json,
trace-padded.json}``. This driver loads the spec through the reference
library (stubbing predicate callables — the audit checks structure, it
never calls predicates), audits both traces, and prints a JSON summary.

Usage:
    python3 benchmark/census/sarc_check.py \
        --export benchmark/census/out/sarc-export \
        --sarc-src /path/to/sarc-governance/src

The two traces are the point: ``trace-faithful`` records only the
evaluations quipu actually ran (the target-type pre-filter — GS1's
zero-cost abstention), which the reference checker's per-action coverage
invariant flags; ``trace-padded`` adds explicit not-fired records for the
non-applicable constraints and should PASS. The delta is coverage
*semantics*, not verdict disagreement.
"""

import argparse
import json
import pathlib
import sys


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--export", required=True, help="the run's sarc-export directory")
    ap.add_argument("--sarc-src", required=True, help="sarc-governance repo's src/ directory")
    args = ap.parse_args()

    sys.path.insert(0, args.sarc_src)
    try:
        import yaml
        from sarc_governance import audit_trace
        from sarc_governance.specs import load_spec
    except ImportError as e:
        print(f"error: cannot import the reference checker: {e}", file=sys.stderr)
        return 2

    export = pathlib.Path(args.export)
    spec_path = export / "spec.yaml"
    raw = yaml.safe_load(spec_path.read_text())
    stubs = {
        c["predicate"]: (lambda _record: True)
        for c in raw.get("constraints", [])
        if "predicate" in c
    }
    spec = load_spec(raw, extra_predicates=stubs)

    summary = {}
    for variant in ("trace-faithful", "trace-padded"):
        trace = json.loads((export / f"{variant}.json").read_text())
        discrepancies = audit_trace(spec, trace)
        by_type = {}
        for d in discrepancies:
            by_type[d["type"]] = by_type.get(d["type"], 0) + 1
        summary[variant] = {
            "records": len(trace),
            "discrepancies": len(discrepancies),
            "by_type": by_type,
            "verdict": "PASS" if not discrepancies else "FAIL",
        }
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
