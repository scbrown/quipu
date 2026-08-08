"""The quipu evidence-regime adapter for DEMM-Bench.

Reconstructs the eight Decision Event Schema property categories from a
(possibly degraded) quipu-native record's *content* — the guard trace,
the signed verdict ledger, and the bitemporal policy snapshot. It never
reads case metadata, degradation names, or oracle labels; the functions
take only the record.

Category semantics follow the benchmark: ``complete`` (property fully
reconstructable), ``partial`` (some evidence, not enough to answer),
``opaque`` (no usable evidence), ``conflicting`` (evidence disagrees
with itself).
"""

from __future__ import annotations

import hashlib
from typing import Any

Record = dict[str, Any]

PROPERTIES = (
    "actor_identity",
    "principal_authority",
    "action_boundary",
    "policy_basis",
    "decision_basis",
    "data_resource_touch",
    "lifecycle_context",
    "verification_strength",
)


def _planes(record: Record) -> tuple[Record, Record, Record]:
    return (
        record.get("guard_trace") or {},
        record.get("verdict_ledger") or {},
        record.get("policy_snapshot") or {},
    )


def _actor_conflict(guard: Record) -> bool:
    executor = guard.get("executor")
    chain = guard.get("principal_chain")
    return bool(executor) and isinstance(chain, list) and bool(chain) and executor not in chain


def actor_identity(record: Record) -> str:
    guard, _, _ = _planes(record)
    if _actor_conflict(guard):
        return "conflicting"
    return "complete" if guard.get("executor") else "opaque"


def principal_authority(record: Record) -> str:
    guard, _, policy = _planes(record)
    if _actor_conflict(guard):
        return "conflicting"
    chain = guard.get("principal_chain")
    grants = policy.get("authority_grants")
    if chain and grants:
        # Authority is complete when nothing contradicts the chain-grant
        # pairing; whether the action stayed inside its boundary is
        # action_boundary's question, not this one's.
        graph = guard.get("graph")
        if not graph or "*" in grants or graph in grants:
            return "complete"
        return "partial"
    if chain:
        return "partial"
    return "opaque"


def action_boundary(record: Record) -> str:
    guard, _, _ = _planes(record)
    present = [bool(guard.get(k)) for k in ("tool", "target", "graph")]
    if all(present):
        return "complete"
    return "partial" if any(present) else "opaque"


def policy_basis(record: Record) -> str:
    guard, verdict, policy = _planes(record)
    constraint_ids = [
        c.get("id")
        for c in guard.get("constraints", [])
        if c.get("id") and c.get("id") != "redacted"
    ]
    named = bool(policy.get("iri") or verdict.get("predicate_id") or constraint_ids)
    claim = bool(policy.get("claim_as_of") or policy.get("claim_current"))
    if named and claim:
        return "complete"
    return "partial" if named else "opaque"


def decision_basis(record: Record) -> str:
    guard, verdict, _ = _planes(record)
    constraints = guard.get("constraints") or []
    evaluated = any("outcome" in c and "response" in c for c in constraints)
    if evaluated:
        return "complete"
    if verdict.get("outcome") and guard.get("result"):
        return "partial"
    return "opaque"


def data_resource_touch(record: Record) -> str:
    guard, _, _ = _planes(record)
    present = [bool(guard.get(k)) for k in ("target", "graph")]
    if all(present):
        return "complete"
    return "partial" if any(present) else "opaque"


def lifecycle_context(record: Record) -> str:
    guard, _, policy = _planes(record)
    instant = bool(guard.get("at"))
    as_of = bool(policy.get("as_of"))
    if instant and as_of:
        return "complete"
    return "partial" if instant else "opaque"


def verification_strength(record: Record) -> str:
    guard, verdict, policy = _planes(record)
    signed = [bool(verdict.get(k)) for k in ("signature", "verifier", "evidence_hash")]
    grounded = bool(guard.get("principal_chain") or policy.get("authority_grants"))
    if all(signed) and grounded:
        return "complete"
    return "partial" if any(signed) else "opaque"


RECONSTRUCTORS = {
    "actor_identity": actor_identity,
    "principal_authority": principal_authority,
    "action_boundary": action_boundary,
    "policy_basis": policy_basis,
    "decision_basis": decision_basis,
    "data_resource_touch": data_resource_touch,
    "lifecycle_context": lifecycle_context,
    "verification_strength": verification_strength,
}


def property_categories(record: Record) -> dict[str, str]:
    """The adapter's full property reconstruction for one record."""
    return {name: fn(record) for name, fn in RECONSTRUCTORS.items()}


def source_validator(record: Record) -> bool:
    """quipu's source-specific validator, per the benchmark's Table 2
    pattern (regime-internal validity, still not property-level scoring):
    every expected field of every plane present, the verdict's evidence
    hash recomputes from what it signs over, the executor is consistent
    with the principal chain, and the target graph falls inside the
    writer's grants. Passes only an intact record — like the benchmark's
    delegation-validator ("signed delegation-chain integrity and
    signature validity"), it certifies internal validity, not that the
    record answers a given governance question.
    """
    guard, verdict, policy = _planes(record)
    guard_ok = all(
        guard.get(k)
        for k in ("executor", "principal_chain", "tool", "target", "graph", "at", "constraints")
    )
    verdict_ok = all(
        verdict.get(k)
        for k in ("predicate_id", "target_ref", "outcome", "evidence_hash", "verifier", "signature")
    )
    policy_ok = all(
        policy.get(k) for k in ("iri", "as_of", "claim_as_of", "targets", "authority_grants")
    )
    if not (guard_ok and verdict_ok and policy_ok):
        return False
    canonical = "|".join(
        str(verdict[k]) for k in ("predicate_id", "target_ref", "outcome")
    )
    digest = "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()
    if verdict.get("evidence_hash") != digest:
        return False
    if guard["executor"] not in guard["principal_chain"]:
        return False
    grants = policy["authority_grants"]
    return "*" in grants or guard["graph"] in grants


def container_flags(record: Record) -> dict[str, Any]:
    """Container-presence indicators — what the presence baselines see.

    The first four are deliberately shallow (a plane or bundle is
    present, not sufficient); the fifth is the regime-internal validity
    check above. The gap between these flags and the property categories
    is the benchmark's point.
    """
    guard, verdict, policy = _planes(record)
    return {
        "trace_present": bool(guard),
        "ledger_present": bool(verdict.get("present")),
        "schema_valid": True,
        "checklist_complete": bool(guard) and bool(verdict) and bool(policy),
        "source_validator_passed": source_validator(record),
    }
