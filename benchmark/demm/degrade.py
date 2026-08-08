"""DEMM-Bench degradation conditions over quipu-native decision records.

Each transform deletes (or, for ``conflicting_identity``, contradicts)
evidence *content* in one of the three planes the census run exports —
``guard_trace``, ``verdict_ledger``, ``policy_snapshot`` — mirroring the
benchmark's construction-oracle semantics (``construction_oracle_v1``):
the oracle's per-property category vector for a condition is what the
surviving evidence should honestly support, no more.

Transforms never write the condition's name into the record: the adapter
must reconstruct sufficiency from content alone (the benchmark's
label-leakage guard).
"""

from __future__ import annotations

import copy
from typing import Any

Record = dict[str, Any]


def _base(record: Record) -> tuple[Record, Record, Record, Record]:
    out = copy.deepcopy(record)
    return out, out.setdefault("guard_trace", {}), out.setdefault(
        "verdict_ledger", {}
    ), out.setdefault("policy_snapshot", {})


def complete(record: Record) -> Record:
    """All evidence planes intact."""
    return copy.deepcopy(record)


def missing_delegation(record: Record) -> Record:
    """Delegation evidence gone: no principal chain, no authority grants."""
    out, guard, _, policy = _base(record)
    guard.pop("principal_chain", None)
    policy.pop("authority_grants", None)
    return out


def missing_policy(record: Record) -> Record:
    """Policy basis gone: no policy IRI, no claim; the evaluation record
    (outcomes, responses) and the delegation evidence survive."""
    out, guard, verdict, policy = _base(record)
    for constraint in guard.get("constraints", []):
        constraint["id"] = "redacted"
    verdict.pop("predicate_id", None)
    for key in ("iri", "claim_as_of", "claim_current", "targets"):
        policy.pop(key, None)
    return out


def missing_context(record: Record) -> Record:
    """Lifecycle context gone: no decision instant, no as-of anchor."""
    out, guard, _, policy = _base(record)
    guard.pop("at", None)
    policy.pop("as_of", None)
    policy.pop("claim_as_of", None)
    return out


def conflicting_identity(record: Record) -> Record:
    """Actor evidence made internally inconsistent: the trace's executor
    contradicts the principal chain it arrived with."""
    out, guard, _, _ = _base(record)
    executor = str(guard.get("executor", ""))
    guard["executor"] = "quilla" if executor != "quilla" else "amaru"
    return out


def partial_graph(record: Record) -> Record:
    """Decision graph incomplete: the per-constraint evaluation record,
    the as-of anchor, and the evidence hash are missing; outcomes,
    instants, claims, and signatures survive."""
    out, guard, verdict, policy = _base(record)
    guard.pop("constraints", None)
    verdict.pop("evidence_hash", None)
    policy.pop("as_of", None)
    policy.pop("claim_as_of", None)
    return out


def final_only(record: Record) -> Record:
    """Only the final outcome and the action surface survive: no
    evaluation record, no chain, no policy, no instants, no signature."""
    out, guard, verdict, _ = _base(record)
    out["guard_trace"] = {
        key: guard[key] for key in ("kind", "executor", "tool", "target", "graph") if key in guard
    }
    out["verdict_ledger"] = {
        key: verdict[key] for key in ("present", "outcome") if key in verdict
    }
    out.pop("policy_snapshot", None)
    return out


def artifact_only(record: Record) -> Record:
    """An artifact reference without actor/action/resource sufficiency:
    the outcome, the evidence hash, the policy IRI, the instants, and the
    delegation evidence survive (the oracle keeps principal authority
    complete here); who executed, with what tool, on which resource do
    not."""
    out, guard, verdict, policy = _base(record)
    out["guard_trace"] = {
        key: guard[key] for key in ("kind", "at", "result", "principal_chain") if key in guard
    }
    out["verdict_ledger"] = {
        key: verdict[key] for key in ("present", "outcome", "evidence_hash") if key in verdict
    }
    out["policy_snapshot"] = {
        key: policy[key] for key in ("iri", "as_of", "authority_grants") if key in policy
    }
    return out


DEGRADATIONS: dict[str, Any] = {
    "complete": complete,
    "missing_delegation": missing_delegation,
    "missing_policy": missing_policy,
    "missing_context": missing_context,
    "conflicting_identity": conflicting_identity,
    "partial_graph": partial_graph,
    "final_only": final_only,
    "artifact_only": artifact_only,
}
