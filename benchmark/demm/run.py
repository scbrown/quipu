"""Run DEMM-Bench (arXiv:2606.20634) against quipu's census evidence.

Builds a 64-case single-regime corpus — 8 degradation conditions x 8
question families, each case backed by a real decision from the seeded
census run's `demm-export/native_records.jsonl` — then scores:

- the five deterministic container-presence baselines the benchmark
  ships (trace-present, ledger-present, schema-present,
  container-checklist, source-specific validator), and
- the quipu property-level reconstructor (`adapter.py`) as the
  candidate scorer,

with ground truth from the benchmark's own construction oracle. Usage:

    python3 benchmark/demm/run.py --demm-src /path/to/decision-evidence-benchmark

The benchmark package must be importable (pip install -e the clone, or
run with its venv's python). Everything is deterministic; the only
inputs are the census export and the benchmark's oracle spec.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from adapter import PROPERTIES, container_flags, property_categories  # noqa: E402
from degrade import DEGRADATIONS  # noqa: E402


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--native",
        default=str(HERE.parent / "census/out/demm-export/native_records.jsonl"),
        help="quipu-native decision records from the census run (CEN-X2)",
    )
    parser.add_argument(
        "--demm-src",
        default=None,
        help="path to a decision-evidence-benchmark clone (adds src/ to sys.path)",
    )
    parser.add_argument("--out", default=str(HERE / "out"))
    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.demm_src:
        sys.path.insert(0, str(Path(args.demm_src) / "src"))
    from decision_evidence_benchmark.baselines.imported import baseline_result_rows
    from decision_evidence_benchmark.baselines.registry import BASELINE_REGISTRY
    from decision_evidence_benchmark.construction_oracle import (
        CONSTRUCTION_ORACLE_VERSION,
        labels_for_degradation,
    )
    from decision_evidence_benchmark.evaluation import (
        evaluate_scorer_outputs,
        validate_scorer_outputs,
    )
    from decision_evidence_benchmark.io import write_cases_jsonl, write_jsonl
    from decision_evidence_benchmark.metrics.overclaim import summarize_outputs
    from decision_evidence_benchmark.schema import CaseManifest, PropertyLabel, ScorerOutput

    natives = [
        json.loads(line)
        for line in Path(args.native).read_text().splitlines()
        if line.strip()
    ]
    if not natives:
        raise SystemExit(f"no native records in {args.native}")

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    # 64 cases: 8 degradation conditions x 8 question families, each over
    # a real census decision. Case ids are opaque (the benchmark's
    # label-leakage guard forbids degradation names in scorer-facing ids).
    cases: list[CaseManifest] = []
    outputs: list[ScorerOutput] = []
    for d_index, (condition, transform) in enumerate(DEGRADATIONS.items()):
        for f_index, family in enumerate(PROPERTIES):
            case_index = d_index * len(PROPERTIES) + f_index
            native = natives[case_index % len(natives)]
            degraded = transform(native)
            case_id = f"quipu-demm-{case_index + 1:03d}"
            cases.append(
                CaseManifest(
                    case_id=case_id,
                    regime="quipu",
                    question_family=family,
                    degradation_condition=condition,
                    evidence={
                        "quipu_native": degraded,
                        "native_record_id": native["record_id"],
                        "evidence_plane": "census_seed42_gated_export",
                    },
                    container_flags=container_flags(degraded),
                    property_labels=labels_for_degradation(condition),
                    metadata={
                        "source": "benchmark/census/out/demm-export/native_records.jsonl",
                        "oracle": CONSTRUCTION_ORACLE_VERSION,
                        "result_honesty": (
                            "Ground-truth labels are the benchmark's construction-"
                            "oracle vectors for the applied degradation; evidence is "
                            "the degraded quipu-native record, from a real seeded "
                            "census decision."
                        ),
                    },
                )
            )
            # The candidate scorer sees ONLY the degraded record — built
            # here from `degraded` before the case's labels can leak.
            categories = property_categories(degraded)
            predictions = tuple(
                PropertyLabel(
                    property=name,
                    category=categories[name],
                    source="quipu_native_reconstructor",
                    notes="content-rule-v1",
                )
                for name in PROPERTIES
            )
            outputs.append(
                ScorerOutput(
                    case_id=case_id,
                    scorer="decision_trace_reconstructor",
                    verdict=(
                        "sufficient"
                        if all(categories[name] == "complete" for name in PROPERTIES)
                        else "insufficient"
                    ),
                    metadata={
                        "implementation_status": "quipu_native_reconstructor_v1",
                        "prediction_source": "quipu_native_evidence_content",
                    },
                    property_predictions=predictions,
                )
            )

    write_cases_jsonl(out_dir / "quipu_cases.jsonl", cases)
    write_jsonl(out_dir / "quipu_scorer_outputs.jsonl", (o.to_dict() for o in outputs))

    validation = validate_scorer_outputs(cases, outputs)
    evaluation = evaluate_scorer_outputs(cases, outputs)
    write_jsonl(out_dir / "quipu_scorer_results.jsonl", evaluation["rows"])
    (out_dir / "quipu_scorer_summary.json").write_text(
        json.dumps(evaluation["summary"], indent=2, sort_keys=True) + "\n"
    )

    # The five deterministic container-presence baselines; llm_judge is a
    # pinned-import baseline and no real LLM judgment exists in this run.
    baselines = tuple(sorted(name for name in BASELINE_REGISTRY if name != "llm_judge"))
    baseline_rows = baseline_result_rows(cases, baselines)
    write_jsonl(out_dir / "quipu_baseline_results.jsonl", baseline_rows)
    baseline_summary = summarize_outputs(baseline_rows)
    (out_dir / "quipu_baseline_summary.json").write_text(
        json.dumps(baseline_summary, indent=2, sort_keys=True) + "\n"
    )

    scorer = evaluation["summary"]["scorers"]["decision_trace_reconstructor"]
    headline = {
        "cases": len(cases),
        "regime": "quipu",
        "native_decisions": len(natives),
        "scorer_valid": validation["valid"],
        "quipu_native_reconstructor": {
            "mean_property_sufficiency_accuracy": scorer[
                "mean_property_sufficiency_accuracy"
            ],
            "overclaim_rate": scorer["overclaim_rate"],
            "overclaim_cases": scorer["overclaim_cases"],
        },
        "container_baselines": {
            name: {
                "overclaim_rate": stats.get("overclaim_rate"),
                "overclaim_cases": stats.get("overclaim_cases"),
            }
            for name, stats in sorted(baseline_summary["scorers"].items())
        },
    }
    (out_dir / "quipu_headline.json").write_text(
        json.dumps(headline, indent=2, sort_keys=True) + "\n"
    )
    print(json.dumps(headline, indent=2, sort_keys=True))
    return 0 if validation["valid"] and evaluation["summary"]["valid"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
