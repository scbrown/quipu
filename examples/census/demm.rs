//! CEN-X2 — export the run's decisions as quipu-native evidence records
//! for the DEMM-Bench decision-evidence sufficiency benchmark
//! (`agent-runtime-evidence/decision-evidence-benchmark`, arXiv:2606.20634).
//!
//! One JSONL record per recorded decision, three evidence planes, each
//! holding only what that plane really carries:
//!
//! - `guard_trace` — what the governed writer presented at the gate
//!   (writer, principal chain, tool, target graph), in the same shape as
//!   the wild traces `quipu audit` consumes. Since Q-VERDICT-ATTRIB the
//!   ledger carries the writer and chain too; the guard trace remains
//!   the only plane with the action surface (tool, graph, planner).
//! - `verdict_ledger` — the signed `aegis:Verdict` fact as persisted:
//!   policy, target, outcome, attribution (Q-VERDICT-ATTRIB: the writer
//!   and chain, sealed inside the evidence hash), evidence hash,
//!   verifier, signature, tier. Queried back from the store, not echoed
//!   from harness state.
//! - `policy_snapshot` — Σ as the store can serve it bitemporally: the
//!   claim as of the decision instant, the claim now (the amendment makes
//!   them differ for phase-2 decisions), targets, and the writer's
//!   authority grants.
//!
//! The degradation conditions, the adapter, and the scoring live in
//! `benchmark/demm/` (Python); this export is the evidence boundary.

use quipu::Value;
use quipu::sparql::{self, QueryResult, TemporalContext};

use crate::phase6::SARC_SPEC;
use crate::phases::Ctx;

const AEGIS: &str = "http://aegis.gastown.local/ontology/";

pub fn demm_export(ctx: &mut Ctx, out_dir: &str) {
    if ctx.replay.is_empty() {
        ctx.probe(
            "CEN-X2",
            6,
            "quipu-native decision evidence export for DEMM-Bench",
            "n/a: no decisions recorded in this arm",
            "RQ3",
        );
        return;
    }
    let dir = format!("{out_dir}/demm-export");
    std::fs::create_dir_all(&dir).expect("create demm-export dir");

    let mut lines = Vec::new();
    for (i, item) in ctx.replay.iter().enumerate() {
        let (_, local_id, class, response) = SARC_SPEC
            .iter()
            .find(|(iri, ..)| *iri == item.policy)
            .expect("replay item cites a Sigma policy");
        let fired_response = if item.outcome == "unsatisfied" {
            *response
        } else {
            "no-action"
        };
        let guard_trace = serde_json::json!({
            "kind": "guard",
            "point": "pre-action",
            "result": if item.outcome == "satisfied" { "allow" } else { "deny" },
            "principal_chain": item.chain,
            "planner": "census-driver",
            "executor": item.writer,
            "tool": "quipu.transact_to_graph",
            "target": item.target,
            "graph": item.graph,
            "at": item.at,
            "constraints": [{
                "id": local_id,
                "class": class,
                "verification_point": "PAG",
                "outcome": item.outcome,
                "response": fired_response,
            }],
        });
        let record = serde_json::json!({
            "record_id": format!("census-dec-{i:03}"),
            "runtime": "quipu-census-gated",
            "guard_trace": guard_trace,
            "verdict_ledger": verdict_ledger(ctx, item),
            "policy_snapshot": policy_snapshot(ctx, item),
        });
        lines.push(serde_json::to_string(&record).expect("record serializes"));
    }
    std::fs::write(
        format!("{dir}/native_records.jsonl"),
        lines.join("\n") + "\n",
    )
    .expect("write native records");
    ctx.probe(
        "CEN-X2",
        6,
        "quipu-native decision evidence export for DEMM-Bench",
        &format!(
            "exported {} decisions to demm-export/native_records.jsonl \
             (guard trace + signed verdict ledger + bitemporal policy snapshot); \
             degrade/adapt/score with benchmark/demm/",
            lines.len()
        ),
        "RQ3",
    );
}

/// The signed verdict fact for one decision, queried back from the store.
/// `None` fields never happen in the gated arm; a missing verdict row is
/// exported as an explicit absence rather than invented.
fn verdict_ledger(ctx: &Ctx, item: &crate::phases::ReplayItem) -> serde_json::Value {
    let q = format!(
        "SELECT ?v ?pred ?target ?outcome ?hash ?verifier ?sig ?tier ?writer ?chain WHERE {{ \
         ?v <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{AEGIS}Verdict> . \
         ?v <{AEGIS}predicateId> ?pred . \
         ?v <{AEGIS}targetRef> ?target . \
         ?v <{AEGIS}outcome> ?outcome . \
         ?v <{AEGIS}evidenceHash> ?hash . \
         ?v <{AEGIS}verifier> ?verifier . \
         ?v <{AEGIS}signature> ?sig . \
         ?v <{AEGIS}tier> ?tier . \
         ?v <{AEGIS}attributedWriter> ?writer . \
         ?v <{AEGIS}principalChain> ?chain }}"
    );
    let Ok(QueryResult::Select { rows, .. }) = sparql::query(&ctx.store, &q) else {
        return serde_json::json!({ "present": false });
    };
    let str_of = |row: &std::collections::HashMap<String, Value>, k: &str| match row.get(k) {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let Some(row) = rows.iter().find(|row| {
        str_of(row, "pred").as_deref() == Some(item.policy.as_str())
            && str_of(row, "target").as_deref() == Some(item.target.as_str())
            && str_of(row, "outcome").as_deref() == Some(item.outcome.as_str())
    }) else {
        return serde_json::json!({ "present": false });
    };
    let s = |k: &str| match str_of(row, k) {
        Some(v) => serde_json::json!(v),
        None => serde_json::json!(null),
    };
    let id = s("v");
    let chain: Vec<String> = match str_of(row, "chain") {
        Some(joined) => joined.split(',').map(str::to_string).collect(),
        None => Vec::new(),
    };
    serde_json::json!({
        "present": true,
        "id": id,
        "predicate_id": item.policy,
        "target_ref": item.target,
        "outcome": item.outcome,
        "attributed_writer": s("writer"),
        "principal_chain": chain,
        "evidence_hash": s("hash"),
        "verifier": s("verifier"),
        "signature": s("sig"),
        "tier": s("tier"),
    })
}

/// Σ for one decision as the store serves it bitemporally: the claim as
/// of the decision instant, the claim now, and the writer's authority.
fn policy_snapshot(ctx: &Ctx, item: &crate::phases::ReplayItem) -> serde_json::Value {
    let claim_as_of = policy_field(ctx, &item.policy, "claim", Some(&item.at));
    let claim_current = policy_field(ctx, &item.policy, "claim", None);
    let targets = policy_field(ctx, &item.policy, "targets", Some(&item.at));
    let q = format!(
        "SELECT ?g WHERE {{ ?p <{AEGIS}principalId> \"{writer}\" . \
         ?p <{AEGIS}authorityOver> ?g }}",
        writer = item.writer,
    );
    let mut grants = Vec::new();
    if let Ok(QueryResult::Select { rows, .. }) = sparql::query(&ctx.store, &q) {
        for row in &rows {
            if let Some(Value::Str(g)) = row.get("g") {
                grants.push(g.clone());
            }
        }
    }
    grants.sort();
    serde_json::json!({
        "iri": item.policy,
        "as_of": item.at,
        "claim_as_of": claim_as_of,
        "claim_current": claim_current,
        "targets": targets,
        "authority_grants": grants,
    })
}

fn policy_field(ctx: &Ctx, policy: &str, field: &str, when: Option<&str>) -> Option<String> {
    let q = format!("SELECT ?x WHERE {{ <{policy}> <{AEGIS}{field}> ?x }}");
    let tc = |ts: &str| TemporalContext {
        valid_at: Some(ts.to_string()),
        ..Default::default()
    };
    let result = match when {
        Some(ts) => sparql::query_temporal(&ctx.store, &q, &tc(ts)),
        None => sparql::query(&ctx.store, &q),
    };
    match result {
        Ok(QueryResult::Select { rows, .. }) => rows.first().and_then(|r| match r.get("x") {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    }
}
