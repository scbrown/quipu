//! Phase 2 — Recording: the defect probes and the clean traffic (RQ1, RQ2).
//!
//! Each probe sets the writer's principal chain, attempts the write, and
//! records the observed outcome in the manifest next to the planted ground
//! truth. In the gated arm every defect must be refused; in the control arm
//! every defect lands — that contrast is RQ2.

use std::time::Instant;

use quipu::Value;

use crate::phases::{CensusIris, Ctx, DISTRICTS, assert_datum, type_ref};

/// Clean writes per class (ungoverned / governed) for RQ1's distributions.
pub const CLEAN_WRITES: usize = 50;

pub fn run(ctx: &mut Ctx, iris: &CensusIris) {
    cen_u1(ctx);
    cen_a1(ctx, iris);
    cen_a2(ctx, iris);
    cen_p1(ctx, iris);
    cen_p2(ctx, iris);
    cen_v1(ctx, iris);
    cen_n1(ctx, iris);
    cen_n2(ctx, iris);
}

/// CEN-U1 — episode ingress missing the required provenance property.
/// The episode path is quipu's real ingress seam; the shape loaded at
/// founding requires `census:recordedBy` on every `census:Record`.
fn cen_u1(ctx: &mut Ctx) {
    let ts = ctx.tick();
    let episode: quipu::Episode = serde_json::from_str(
        r#"{
            "name": "u1-untagged-record",
            "episode_body": "A record with no recordedBy tag.",
            "source": "census",
            "nodes": [
                {"name": "record-u1", "type": "Record",
                 "description": "household tally, provenance missing"}
            ],
            "edges": []
        }"#,
    )
    .expect("episode json parses");
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    let result = quipu::ingest_episode(&mut ctx.store, &episode, &ts, "urn:census:");
    let (observed, landed) = match result {
        Ok((tx, n)) => (format!("landed: tx {tx}, {n} triples"), true),
        Err(e) => (format!("refused: {e}"), false),
    };
    ctx.probe_defect(
        "CEN-U1",
        2,
        "urn:census:record-u1",
        &observed,
        landed,
        "RQ2",
    );
}

/// CEN-A1 — a write into a district the writer holds no authority over.
fn cen_a1(ctx: &mut Ctx, iris: &CensusIris) {
    let ts = ctx.tick();
    let subject = "urn:census:subject:a1";
    let datums = vec![
        assert_datum(
            &ctx.store,
            subject,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Note"),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            subject,
            "urn:census:vocab:notes",
            Value::Str("out of area".into()),
            &ts,
        ),
    ];
    // amaru holds ROOT + north; the write targets south.
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    let r = ctx.store.transact_to_graph(
        &datums,
        &ts,
        Some("amaru"),
        Some("census"),
        iris.district_g[1],
    );
    let (observed, landed) = outcome(r);
    ctx.probe_defect("CEN-A1", 2, subject, &observed, landed, "RQ2");
}

/// CEN-A2 — delegation narrows: chaski holds south+east, the delegate
/// scribe holds north+south; the chain [chaski, scribe] writes to east,
/// which chaski alone could reach but the intersection cannot.
fn cen_a2(ctx: &mut Ctx, iris: &CensusIris) {
    let ts = ctx.tick();
    let subject = "urn:census:subject:a2";
    let datums = vec![assert_datum(
        &ctx.store,
        subject,
        "urn:census:vocab:notes",
        Value::Str("delegated overreach".into()),
        &ts,
    )];
    ctx.store
        .set_principal_chain(vec!["chaski".into(), "scribe".into()]);
    let r = ctx.store.transact_to_graph(
        &datums,
        &ts,
        Some("scribe"),
        Some("census"),
        iris.district_g[2],
    );
    let (observed, landed) = outcome(r);
    ctx.probe_defect("CEN-A2", 2, subject, &observed, landed, "RQ2");
}

/// CEN-P1 — a Tally without the label its policy claim requires.
fn cen_p1(ctx: &mut Ctx, iris: &CensusIris) {
    let ts = ctx.tick();
    let subject = "urn:census:subject:p1";
    let datums = vec![assert_datum(
        &ctx.store,
        subject,
        quipu::namespace::RDF_TYPE,
        type_ref(&ctx.store, "urn:census:Tally"),
        &ts,
    )];
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    let r = ctx.store.transact_to_graph(
        &datums,
        &ts,
        Some("amaru"),
        Some("census"),
        iris.district_g[0],
    );
    let (observed, landed) = outcome(r);
    if ctx.gated() && !landed {
        ctx.replay.push(crate::phases::ReplayItem {
            policy: "urn:census:policy:tally-label".to_string(),
            target: subject.to_string(),
            outcome: "unsatisfied".to_string(),
            at: ts.clone(),
            writer: "amaru".to_string(),
            chain: vec!["amaru".to_string()],
            graph: crate::phases::DISTRICTS[0].to_string(),
        });
    }
    ctx.probe_defect("CEN-P1", 2, subject, &observed, landed, "RQ2");
}

/// CEN-P2 — the post-state discriminator. The first placement is legal;
/// the second is legal against the PRE-state (the resident exists, the
/// datum is well-formed) and only the combined post-state — two
/// placements — violates the claim. A pre-state gate passes this write.
fn cen_p2(ctx: &mut Ctx, iris: &CensusIris) {
    let ts = ctx.tick();
    let subject = "urn:census:subject:p2";
    ctx.store.set_principal_chain(vec!["amaru".into()]);
    let first = vec![
        assert_datum(
            &ctx.store,
            subject,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Resident"),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            subject,
            "urn:census:vocab:placedIn",
            Value::Str("dwelling-1".into()),
            &ts,
        ),
    ];
    ctx.store
        .transact_to_graph(
            &first,
            &ts,
            Some("amaru"),
            Some("census"),
            iris.district_g[0],
        )
        .expect("one placement is legal in both arms");
    let ts2 = ctx.tick();
    let second = vec![assert_datum(
        &ctx.store,
        subject,
        "urn:census:vocab:placedIn",
        Value::Str("dwelling-2".into()),
        &ts2,
    )];
    let r = ctx.store.transact_to_graph(
        &second,
        &ts2,
        Some("amaru"),
        Some("census"),
        iris.district_g[0],
    );
    let (observed, landed) = outcome(r);
    // The defect is the SECOND placement value; score.rs special-cases the
    // presence check for this probe.
    if ctx.gated() && !landed {
        ctx.replay.push(crate::phases::ReplayItem {
            policy: "urn:census:policy:single-placement".to_string(),
            target: subject.to_string(),
            outcome: "unsatisfied".to_string(),
            at: ts2.clone(),
            writer: "amaru".to_string(),
            chain: vec!["amaru".to_string()],
            graph: crate::phases::DISTRICTS[0].to_string(),
        });
    }
    ctx.probe_defect("CEN-P2", 2, subject, &observed, landed, "RQ2");
}

/// CEN-V1 — the closed-world vocabulary probe (bead quipu-64q): a record
/// using a fabricated predicate in the policed namespace. Open-world SHACL
/// alone passes this; the vocabulary policy in Σ refuses it.
fn cen_v1(ctx: &mut Ctx, iris: &CensusIris) {
    let ts = ctx.tick();
    let subject = "urn:census:subject:v1";
    let datums = vec![
        assert_datum(
            &ctx.store,
            subject,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Record"),
            &ts,
        ),
        assert_datum(
            &ctx.store,
            subject,
            "urn:census:vocab:recordedBy",
            Value::Str("amaru".into()),
            &ts,
        ),
        // The fabrication: plausible, well-formed, undeclared.
        assert_datum(
            &ctx.store,
            subject,
            "urn:census:vocab:hasQuota",
            Value::Int(3),
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
    let (observed, landed) = outcome(r);
    if ctx.gated() && !landed {
        ctx.replay.push(crate::phases::ReplayItem {
            policy: "urn:census:policy:closed-vocabulary".to_string(),
            target: subject.to_string(),
            outcome: "unsatisfied".to_string(),
            at: ts.clone(),
            writer: "amaru".to_string(),
            chain: vec!["amaru".to_string()],
            graph: crate::phases::DISTRICTS[0].to_string(),
        });
    }
    ctx.probe_defect("CEN-V1", 2, subject, &observed, landed, "RQ2");
}

/// CEN-N1 — clean writes touching no governed type: RQ1's zero-cost
/// abstention distribution.
fn cen_n1(ctx: &mut Ctx, iris: &CensusIris) {
    for i in 0..CLEAN_WRITES {
        let ts = ctx.tick();
        let d = (ctx.rng.below(DISTRICTS.len() as u64)) as usize;
        let subject = format!("urn:census:note:{i}");
        let datums = vec![
            assert_datum(
                &ctx.store,
                &subject,
                quipu::namespace::RDF_TYPE,
                type_ref(&ctx.store, "urn:census:Note"),
                &ts,
            ),
            assert_datum(
                &ctx.store,
                &subject,
                "urn:census:vocab:notes",
                Value::Str(format!("note {i}")),
                &ts,
            ),
        ];
        ctx.store.set_principal_chain(vec![owner_of(d).into()]);
        let start = Instant::now();
        ctx.store
            .transact_to_graph(
                &datums,
                &ts,
                Some(owner_of(d)),
                Some("census"),
                iris.district_g[d],
            )
            .expect("ungoverned clean writes land in both arms");
        ctx.lat_ungoverned.push(start.elapsed().as_micros());
    }
    ctx.probe(
        "CEN-N1",
        2,
        &format!("{CLEAN_WRITES} clean ungoverned writes"),
        "landed; latencies in metrics/rq1.json",
        "RQ1",
    );
}

/// CEN-N2 — clean writes touching a governed type: RQ1's enforcement-cost
/// distribution.
fn cen_n2(ctx: &mut Ctx, iris: &CensusIris) {
    for i in 0..CLEAN_WRITES {
        let ts = ctx.tick();
        let d = (ctx.rng.below(DISTRICTS.len() as u64)) as usize;
        let subject = format!("urn:census:tally:{i}");
        let datums = vec![
            assert_datum(
                &ctx.store,
                &subject,
                quipu::namespace::RDF_TYPE,
                type_ref(&ctx.store, "urn:census:Tally"),
                &ts,
            ),
            assert_datum(
                &ctx.store,
                &subject,
                quipu::namespace::RDFS_LABEL,
                Value::Str(format!("tally {i}")),
                &ts,
            ),
        ];
        ctx.store.set_principal_chain(vec![owner_of(d).into()]);
        let start = Instant::now();
        ctx.store
            .transact_to_graph(
                &datums,
                &ts,
                Some(owner_of(d)),
                Some("census"),
                iris.district_g[d],
            )
            .expect("compliant governed writes land in both arms");
        ctx.lat_governed.push(start.elapsed().as_micros());
        if ctx.gated() {
            ctx.replay.push(crate::phases::ReplayItem {
                policy: "urn:census:policy:tally-label".to_string(),
                target: subject.clone(),
                outcome: "satisfied".to_string(),
                at: ts.clone(),
                writer: owner_of(d).to_string(),
                chain: vec![owner_of(d).to_string()],
                graph: crate::phases::DISTRICTS[d].to_string(),
            });
        }
    }
    ctx.probe(
        "CEN-N2",
        2,
        &format!("{CLEAN_WRITES} clean governed writes"),
        "landed; latencies in metrics/rq1.json",
        "RQ1",
    );
}

/// Which recorder holds authority over district `d` (see founding).
pub fn owner_of(d: usize) -> &'static str {
    match d {
        0 => "amaru",
        _ => "chaski",
    }
}

fn outcome(r: quipu::Result<i64>) -> (String, bool) {
    match r {
        Ok(tx) => (format!("landed: tx {tx}"), true),
        Err(e) => (format!("refused: {e}"), false),
    }
}
