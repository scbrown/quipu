//! Phase 3 — Correction: escalation, supersession, promotion.

use quipu::{Value, sparql};

use crate::phases::{CensusIris, Ctx, DISTRICTS, assert_datum, retract_datum, type_ref};

const AEGIS: &str = "http://aegis.gastown.local/ontology/";

pub fn run(ctx: &mut Ctx, iris: &CensusIris) {
    cen_e1(ctx, iris);
    cen_e2(ctx, iris);
    cen_r1(ctx, iris);
    cen_r2(ctx, iris);
}

/// The evidence hash the router minted for `subject`, read back from the
/// open `DecisionRequest` — the public surface, not a private helper.
fn minted_evidence_hash(ctx: &Ctx, subject: &str) -> Option<String> {
    let q = format!(
        "PREFIX a: <{AEGIS}> SELECT ?h WHERE {{ ?r a a:DecisionRequest ; \
         a:forTarget \"{subject}\" ; a:evidenceHash ?h }}"
    );
    match sparql::query(&ctx.store, &q) {
        Ok(sparql::QueryResult::Select { rows, .. }) => rows.first().and_then(|row| {
            row.get("h").and_then(|v| match v {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
        }),
        _ => None,
    }
}

/// A scripted human decision bound to the request's evidence.
fn decide(ctx: &mut Ctx, subject: &str, outcome: &str) {
    let Some(hash) = minted_evidence_hash(ctx, subject) else {
        return; // control arm: nothing escalated, nothing to decide
    };
    let ts = ctx.tick();
    let iri = format!(
        "urn:census:decision:{}-{outcome}",
        subject.rsplit(':').next().unwrap_or("x")
    );
    let datums = vec![
        assert_datum(
            &ctx.store,
            &iri,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, &format!("{AEGIS}Decision")),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            &iri,
            &format!("{AEGIS}outcome"),
            Value::Str(outcome.to_string()),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            &iri,
            &format!("{AEGIS}by"),
            Value::Str("keeper".to_string()),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            &iri,
            &format!("{AEGIS}evidenceHash"),
            Value::Str(hash),
            &ts,
        ),
    ];
    ctx.store.set_principal_chain(vec!["keeper".into()]);
    ctx.store
        .transact(&datums, &ts, Some("keeper"), Some("census"))
        .expect("the human decision lands");
}

fn annex_write(ctx: &mut Ctx, subject: &str, iris: &CensusIris) -> Result<i64, quipu::Error> {
    let ts = ctx.tick();
    let datums = vec![assert_datum(
        &ctx.store,
        subject,
        quipu::namespace::RDF_TYPE,
        type_ref(&ctx.store, "urn:census:Annex"),
        &ts,
    )];
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    ctx.store.transact_to_graph(
        &datums,
        &ts,
        Some("amaru"),
        Some("census"),
        iris.district_g[0],
    )
}

/// CEN-E1 — escalation approved: refusal mints a `DecisionRequest`; the
/// scripted human approves against the same evidence; the retry lands.
fn cen_e1(ctx: &mut Ctx, iris: &CensusIris) {
    let subject = "urn:census:subject:e1";
    let first = annex_write(ctx, subject, iris);
    if ctx.gated() && first.is_err() {
        ctx.replay.push(crate::phases::ReplayItem {
            policy: "urn:census:policy:annex-approval".to_string(),
            target: subject.to_string(),
            outcome: "unsatisfied".to_string(),
            at: ctx.last_ts(),
        });
    }
    decide(ctx, subject, "approve");
    let retry = annex_write(ctx, subject, iris);
    let observed = format!(
        "first={} retry={}",
        outcome_word(&first),
        outcome_word(&retry)
    );
    let landed = retry.is_ok();
    ctx.probe_defect("CEN-E1", 3, subject, &observed, landed, "RQ3");
}

/// CEN-E2 — escalation rejected: the retry stays refused.
fn cen_e2(ctx: &mut Ctx, iris: &CensusIris) {
    let subject = "urn:census:subject:e2";
    let first = annex_write(ctx, subject, iris);
    if ctx.gated() && first.is_err() {
        ctx.replay.push(crate::phases::ReplayItem {
            policy: "urn:census:policy:annex-approval".to_string(),
            target: subject.to_string(),
            outcome: "unsatisfied".to_string(),
            at: ctx.last_ts(),
        });
    }
    decide(ctx, subject, "reject");
    let retry = annex_write(ctx, subject, iris);
    let observed = format!(
        "first={} retry={}",
        outcome_word(&first),
        outcome_word(&retry)
    );
    // In the gated arm the DEFECT would be this subject landing at all.
    let landed = first.is_ok() || retry.is_ok();
    ctx.probe_defect("CEN-E2", 3, subject, &observed, landed, "RQ3");
}

/// CEN-R1 — supersession: retract the old placement and assert the new one
/// in ONE transaction. The single-placement claim sees the combined
/// post-state — one placement — and passes; history keeps both values.
fn cen_r1(ctx: &mut Ctx, iris: &CensusIris) {
    let ts = ctx.tick();
    let subject = "urn:census:subject:p2";
    let datums = vec![
        retract_datum(
            &ctx.store,
            subject,
            "urn:census:vocab:placedIn",
            Value::Str("dwelling-1".into()),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            subject,
            "urn:census:vocab:placedIn",
            Value::Str("dwelling-3".into()),
            &ts,
        ),
    ];
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    let r = ctx.store.transact_to_graph(
        &datums,
        &ts,
        Some("amaru"),
        Some("census"),
        iris.district_g[0],
    );
    let observed = match &r {
        Ok(tx) => format!("supersession landed: tx {tx}"),
        Err(e) => format!("refused: {e}"),
    };
    ctx.probe(
        "CEN-R1",
        3,
        "retraction plus supersession of a placement",
        &observed,
        "RQ5",
    );
}

/// CEN-R2 — promotion: the tally moves from north (rank 0) to east
/// (rank 2). The move is two graph-scoped writes — a retraction in the
/// lower plane, an assertion in the higher — and both survive as history.
fn cen_r2(ctx: &mut Ctx, iris: &CensusIris) {
    let subject = "urn:census:tally:0";
    let label = Value::Str("tally 0".to_string());
    let ts = ctx.tick();
    // Retract from wherever CEN-N2's rng placed it is fiddly; promote a
    // dedicated fact instead: assert in north, then move to east.
    let promoted = "urn:census:tally:promoted";
    let datums = vec![
        assert_datum(
            &ctx.store,
            promoted,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Tally"),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            promoted,
            quipu::namespace::RDFS_LABEL,
            label.clone(),
            &ts,
        ),
    ];
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    ctx.store
        .transact_to_graph(
            &datums,
            &ts,
            Some("amaru"),
            Some("census"),
            iris.district_g[0],
        )
        .expect("the tally to promote lands in north");

    let ts2 = ctx.tick();
    let out = vec![
        retract_datum(
            &ctx.store,
            promoted,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Tally"),
            &ts2,
        ),
        retract_datum(
            &ctx.store,
            promoted,
            quipu::namespace::RDFS_LABEL,
            label.clone(),
            &ts2,
        ),
    ];
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    ctx.store
        .transact_to_graph(
            &out,
            &ts2,
            Some("amaru"),
            Some("census"),
            iris.district_g[0],
        )
        .expect("retraction from the lower plane lands");
    let ts3 = ctx.tick();
    let into = vec![
        assert_datum(
            &ctx.store,
            promoted,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Tally"),
            &ts3,
        ),
        assert_datum(
            &ctx.store,
            promoted,
            quipu::namespace::RDFS_LABEL,
            label,
            &ts3,
        ),
    ];
    ctx.store.set_principal_chain(vec!["chaski".into()]);
    let r = ctx.store.transact_to_graph(
        &into,
        &ts3,
        Some("chaski"),
        Some("census"),
        iris.district_g[2],
    );
    let observed = match &r {
        Ok(tx) => format!(
            "promoted {promoted} from {} to {} (tx {tx}); both writes kept as history",
            DISTRICTS[0], DISTRICTS[2]
        ),
        Err(e) => format!("refused: {e}"),
    };
    ctx.probe("CEN-R2", 3, "trust-plane promotion", &observed, "RQ5");
    let _ = subject;
}

fn outcome_word(r: &Result<i64, quipu::Error>) -> String {
    match r {
        Ok(tx) => format!("landed(tx {tx})"),
        Err(e) => {
            let s = e.to_string();
            let head: String = s.chars().take(60).collect();
            format!("refused({head})")
        }
    }
}
