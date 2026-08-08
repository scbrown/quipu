//! Phase 5 — Amendment: Σ and the record shape change mid-run.
//!
//! The amendment is what makes GS6 non-trivial: phase-2 decisions must
//! replay under the OLD rules (phase 6), and a post-amendment write that
//! was valid under old Σ must be judged by the new.

use quipu::Value;

use crate::phases::{
    CensusIris, Ctx, TALLY_CLAIM_V1, TALLY_CLAIM_V2, assert_datum, retract_datum, type_ref,
};

const AEGIS: &str = "http://aegis.gastown.local/ontology/";
const TALLY_POLICY: &str = "urn:census:policy:tally-label";

pub fn run(ctx: &mut Ctx, iris: &CensusIris) {
    amend(ctx);
    cen_m1(ctx, iris);
}

/// The amendment, in one keeper transaction: the tally policy's claim is
/// superseded (retract old value, assert new — Σ is ordinary bitemporal
/// facts), and the record shape is reloaded (the bitemporal registry keeps
/// v1 readable as-of).
fn amend(ctx: &mut Ctx) {
    let ts = ctx.tick();
    ctx.amendment_at = Some(ts.clone());
    let datums = vec![
        retract_datum(
            &ctx.store,
            TALLY_POLICY,
            &format!("{AEGIS}claim"),
            Value::Str(TALLY_CLAIM_V1.to_string()),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            TALLY_POLICY,
            &format!("{AEGIS}claim"),
            Value::Str(TALLY_CLAIM_V2.to_string()),
            &ts,
        ),
    ];
    ctx.store.set_principal_chain(vec!["keeper".into()]);
    let tx = ctx
        .store
        .transact(&datums, &ts, Some("keeper"), Some("census:amendment"))
        .expect("the amendment lands");
    // Shape v2: records now also need a households count. Loaded through
    // the bitemporal registry, so v1 stays answerable as-of (quipu #71).
    let shape_v2 = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix census: <urn:census:> .
@prefix vocab: <urn:census:vocab:> .

census:RecordShape a sh:NodeShape ;
    sh:targetClass census:Record ;
    sh:property [
        sh:path vocab:recordedBy ;
        sh:minCount 1 ;
        sh:message "every census record must carry vocab:recordedBy" ;
    ] ;
    sh:property [
        sh:path vocab:households ;
        sh:minCount 1 ;
        sh:message "amended: every census record must carry vocab:households" ;
    ] .
"#;
    ctx.store
        .load_shapes("census-record", shape_v2, &ts)
        .expect("shape v2 loads through the versioned registry");
    ctx.probe(
        "CEN-M0",
        5,
        "amendment: tally claim v1 superseded by v2; record shape reloaded as v2",
        &format!("tx {tx} at {ts}; claim and shape both versioned"),
        "setup",
    );
}

/// CEN-M1 — a tally with a label but no recorder: exactly what old Σ
/// accepted all through phase 2, judged by amended Σ now.
fn cen_m1(ctx: &mut Ctx, iris: &CensusIris) {
    let ts = ctx.tick();
    let subject = "urn:census:subject:m1";
    let datums = vec![
        assert_datum(
            &ctx.store,
            subject,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Tally"),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            subject,
            quipu::namespace::RDFS_LABEL,
            Value::Str("late tally".into()),
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
    let (observed, landed) = match r {
        Ok(tx) => (format!("landed: tx {tx}"), true),
        Err(e) => (format!("refused: {e}"), false),
    };
    if ctx.gated() && !landed {
        ctx.replay.push(crate::phases::ReplayItem {
            policy: "urn:census:policy:tally-label".to_string(),
            target: subject.to_string(),
            outcome: "unsatisfied".to_string(),
            at: ts.clone(),
        });
    }
    ctx.probe_defect("CEN-M1", 5, subject, &observed, landed, "RQ5");
}
