//! Phase 6 — Audit: as-of replay (RQ5) and the in-store audit passes (RQ3).
//!
//! Replay semantics, stated honestly: a SATISFIED verdict re-derives fully
//! — its evidence (the facts) persisted, and both the data and the rules
//! are bitemporal, so `query_temporal` reproduces the decision. A DENIED
//! verdict cannot be re-derived from the store alone: the staged delta was
//! rolled back — GS2 keeps the verdict, deliberately not the attempt — so
//! replay for denials verifies the rules in force at the time instead.
//! That asymmetry is a finding, not a bug (`BUILD_REPORT.md`).

use quipu::sparql::{self, QueryResult, TemporalContext};
use quipu::{Value, governance};

use crate::phases::{Ctx, assert_datum, type_ref};

const AEGIS: &str = "http://aegis.gastown.local/ontology/";

pub fn run(ctx: &mut Ctx) {
    // Phase 4's enforcement floor was a composition device; the audit must
    // see the whole store, not refuse its own evidence queries.
    ctx.store.labels_config_mut().min_freshness = None;
    cen_m2_replay(ctx);
    cen_g1_g2_inventory(ctx);
    cen_t1_attribution(ctx);
}

fn at(ts: &str) -> TemporalContext {
    TemporalContext {
        valid_at: Some(ts.to_string()),
        ..Default::default()
    }
}

fn claim_of(ctx: &Ctx, policy: &str, when: Option<&str>) -> Option<String> {
    let q = format!("SELECT ?c WHERE {{ <{policy}> <{AEGIS}claim> ?c }}");
    let result = match when {
        Some(ts) => sparql::query_temporal(&ctx.store, &q, &at(ts)),
        None => sparql::query(&ctx.store, &q),
    };
    match result {
        Ok(QueryResult::Select { rows, .. }) => rows.first().and_then(|r| match r.get("c") {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    }
}

fn eval_claim(ctx: &Ctx, claim: &str, target: &str, when: Option<&str>) -> &'static str {
    let ask = claim.replace("$target", &format!("<{target}>"));
    let result = match when {
        Some(ts) => sparql::query_temporal(&ctx.store, &ask, &at(ts)),
        None => sparql::query(&ctx.store, &ask),
    };
    match result {
        Ok(QueryResult::Ask(true)) => "satisfied",
        Ok(QueryResult::Ask(false)) => "unsatisfied",
        _ => "error",
    }
}

/// CEN-M2 — replay every recorded decision as of its transaction.
fn cen_m2_replay(ctx: &mut Ctx) {
    let items = std::mem::take(&mut ctx.replay);
    if items.is_empty() {
        ctx.probe(
            "CEN-M2",
            6,
            "replay of recorded decisions across the amendment",
            "n/a: no verdicts recorded in this arm (gates off)",
            "RQ5",
        );
        return;
    }
    let mut satisfied_replayed = 0usize;
    let mut satisfied_faithful = 0usize;
    let mut drifted = 0usize;
    let mut denials_rules_verified = 0usize;
    let mut denials = 0usize;
    for item in &items {
        let claim_then = claim_of(ctx, &item.policy, Some(&item.at));
        let claim_now = claim_of(ctx, &item.policy, None);
        let moved = claim_then.is_some() && claim_then != claim_now;
        if item.outcome == "satisfied" {
            satisfied_replayed += 1;
            if let Some(c) = &claim_then {
                // Full re-derivation: rules-as-of over data-as-of.
                if eval_claim(ctx, c, &item.target, Some(&item.at)) == "satisfied" {
                    satisfied_faithful += 1;
                }
                // The drift a latest-only replay would misreport: the same
                // decision under TODAY's claim.
                if moved
                    && eval_claim(ctx, claim_now.as_deref().unwrap_or(c), &item.target, None)
                        != "satisfied"
                {
                    drifted += 1;
                }
            }
        } else {
            denials += 1;
            // The attempt's delta was rolled back; what replay CAN verify
            // is that the policy and claim it cites were in force then.
            if claim_then.is_some() {
                denials_rules_verified += 1;
            }
        }
    }
    let summary = serde_json::json!({
        "replayed": items.len(),
        "satisfied_verdicts": satisfied_replayed,
        "satisfied_rederived_faithfully": satisfied_faithful,
        "satisfied_that_would_misreport_under_latest_only_sigma": drifted,
        "denials": denials,
        "denials_rules_in_force_verified": denials_rules_verified,
    });
    let observed = format!(
        "{satisfied_faithful}/{satisfied_replayed} satisfied verdicts re-derived faithfully \
         as-of; {drifted} would misreport under latest-only Sigma; \
         {denials_rules_verified}/{denials} denials verified against rules-in-force (delta \
         rolled back by design)"
    );
    ctx.replay_summary = Some(summary);
    ctx.probe(
        "CEN-M2",
        6,
        "replay of recorded decisions across the amendment",
        &observed,
        "RQ5",
    );
}

/// CEN-G1 / CEN-G2 — the dispatch inventory's two severities.
fn cen_g1_g2_inventory(ctx: &mut Ctx) {
    let ts = ctx.tick();
    let datums = vec![
        // G1: executable, ungoverned, no reason — the unknown hole.
        assert_datum(
            &ctx.store,
            "urn:census:tool:shadow-export",
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, &format!("{AEGIS}ToolClass")),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            "urn:census:tool:shadow-export",
            &format!("{AEGIS}executable"),
            Value::Bool(true),
            &ts,
        ),
        // G2: executable, ungoverned with a declared reason — acknowledged.
        assert_datum(
            &ctx.store,
            "urn:census:tool:manual-adjustment",
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, &format!("{AEGIS}ToolClass")),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            "urn:census:tool:manual-adjustment",
            &format!("{AEGIS}executable"),
            Value::Bool(true),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            "urn:census:tool:manual-adjustment",
            &format!("{AEGIS}ungovernedReason"),
            Value::Str("quarterly manual correction, operator-run".into()),
            &ts,
        ),
    ];
    ctx.store.set_principal_chain(vec!["keeper".into()]);
    ctx.store
        .transact(&datums, &ts, Some("keeper"), Some("census:audit"))
        .expect("tool-class declarations land");
    match governance::inventory::check(&ctx.store) {
        Ok(report) => {
            let violations = report.of(governance::audit::Severity::Violation).len();
            let incomplete = report.of(governance::audit::Severity::Incompleteness).len();
            ctx.inventory_counts = Some((violations, incomplete));
            let observed = format!(
                "inventory: {violations} violation(s) [shadow-export, no reason], \
                 {incomplete} incompleteness [manual-adjustment, declared reason]"
            );
            ctx.probe(
                "CEN-G1",
                6,
                "ungoverned tool class, no reason",
                &observed,
                "RQ3",
            );
            ctx.probe(
                "CEN-G2",
                6,
                "ungoverned tool class with declared reason",
                &format!("counted as incompleteness, not violation ({incomplete} entries)"),
                "RQ3",
            );
        }
        Err(e) => {
            ctx.probe(
                "CEN-G1",
                6,
                "ungoverned tool class, no reason",
                &format!("inventory failed: {e}"),
                "RQ3",
            );
            ctx.probe(
                "CEN-G2",
                6,
                "ungoverned tool class with declared reason",
                &format!("inventory failed: {e}"),
                "RQ3",
            );
        }
    }
}

/// CEN-T1 — an unattributed trace record is incompleteness, not a
/// violation, and never lands at the attribution root. The trace here is
/// synthesized from the run's own decisions plus one deliberately
/// unattributed record.
fn cen_t1_attribution(ctx: &mut Ctx) {
    // A well-formed window over the run's own decisions — constraint ids
    // are Σ's local names, refusals name the unsatisfied constraint, and
    // attribution is complete — except ONE record with no attribution at
    // all: that record is the probe.
    let trace = r#"{"kind":"guard","point":"pre-action","result":"deny","principal_chain":["amaru"],"planner":"census-driver","executor":"census","tool":"transact","constraints":[{"id":"tally-label","class":"hard","verification_point":"PAG","outcome":"unsatisfied","response":"blocked"}]}
{"kind":"guard","point":"pre-action","result":"allow","principal_chain":["chaski"],"planner":"census-driver","executor":"census","tool":"transact","constraints":[{"id":"tally-label","class":"hard","verification_point":"PAG","outcome":"satisfied","response":"no-action"}]}
{"kind":"guard","point":"pre-action","result":"deny","principal_chain":["amaru"],"planner":"census-driver","executor":"census","tool":"transact","constraints":[{"id":"closed-vocabulary","class":"hard","verification_point":"PAG","outcome":"unsatisfied","response":"blocked"}]}
{"kind":"guard","point":"pre-action","result":"deny","principal_chain":["amaru"],"planner":"census-driver","executor":"census","tool":"transact","constraints":[{"id":"annex-approval","class":"escalation","verification_point":"PAG","outcome":"unsatisfied","response":"escalated"}]}
{"kind":"guard","point":"pre-action","result":"deny","principal_chain":[],"constraints":[{"id":"single-placement","class":"hard","verification_point":"PAG","outcome":"unsatisfied","response":"blocked"}]}
"#;
    match governance::audit::check_jsonl(&ctx.store, trace) {
        Ok(report) => {
            let violations = report.of(governance::audit::Severity::Violation).len();
            let incomplete = report.of(governance::audit::Severity::Incompleteness).len();
            ctx.audit_counts = Some((violations, incomplete));
            let observed = format!(
                "audit over 3-record trace (one unattributed): {violations} violation(s), \
                 {incomplete} incompleteness finding(s)"
            );
            ctx.probe(
                "CEN-T1",
                6,
                "trace record with no attribution",
                &observed,
                "RQ3",
            );
        }
        Err(e) => {
            ctx.probe(
                "CEN-T1",
                6,
                "trace record with no attribution",
                &format!("audit failed: {e}"),
                "RQ3",
            );
        }
    }
}
