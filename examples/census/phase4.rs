//! Phase 4 — Composition: the lattice probes (RQ4) and the pack import.
//!
//! Read-time behavior, identical in both arms: labels and folds are not
//! gated writes, they are how composed reads are judged.

use quipu::store::labels::GraphLabel;
use quipu::{Value, lattice};

use crate::phases::{CensusIris, Ctx, DISTRICTS, assert_datum, type_ref};

pub fn run(ctx: &mut Ctx, iris: &CensusIris, out_dir: &str) {
    // Composition is administered by the keeper: labelling a graph needs
    // authority over the META-graph, which no recorder chain holds — the
    // meta-partition rule (GS3) that stops a tenant promoting itself.
    ctx.store.set_principal_chain(vec!["keeper".into()]);
    // One floor for the whole phase: composed reads must be fresh.
    ctx.store.labels_config_mut().min_freshness = Some("fresh".into());

    cen_c1(ctx, iris);
    cen_c2(ctx, iris);
    cen_c3(ctx, iris);
    cen_c4(ctx, iris);
    cen_c5(ctx, iris);
    cen_c6(ctx, iris);
    cen_c7(ctx, out_dir);
}

fn label_fresh_with_rank(ctx: &mut Ctx, iri: &str, rank: i64, chain: &str) -> i64 {
    let g = ctx.store.graph_create(iri).expect("graph registers");
    let ts = ctx.tick();
    let label = GraphLabel {
        freshness: Some(lattice::Freshness::Fresh),
        durability: None,
        trust: Some(lattice::Trust {
            iri: format!("{iri}#trust"),
            chain: chain.to_string(),
            rank,
        }),
        policy: None,
        kind: None,
    };
    ctx.store
        .set_graph_label(iri, &label, &ts, Some("census:composition"))
        .expect("label lands");
    g
}

/// CEN-C1 — an undeclared graph degrades Coverage and fails the floor.
fn cen_c1(ctx: &mut Ctx, iris: &CensusIris) {
    let ghost = ctx
        .store
        .graph_create("urn:census:graph:ghost")
        .expect("ghost graph registers (but is never labelled)");
    let set = [iris.district_g[0], ghost];
    let labels = ctx.store.dataset_labels(&set).expect("fold computes");
    let floored = ctx.store.check_label_floor(&set);
    let refused = floored.is_err();
    let observed = format!(
        "coverage={:?}; floor={}",
        labels.freshness.coverage,
        match &floored {
            Ok(()) => "passed (WRONG)".to_string(),
            Err(e) => format!("refused: {e}"),
        }
    );
    ctx.probe_refusal("CEN-C1", &observed, refused);
}

/// CEN-C2 — cross-chain trust refuses comparison by name.
fn cen_c2(ctx: &mut Ctx, iris: &CensusIris) {
    let foreign = label_fresh_with_rank(
        ctx,
        "urn:census:graph:foreign-province",
        5,
        "urn:census:chain:foreign",
    );
    let r = ctx.store.dataset_labels(&[iris.district_g[0], foreign]);
    let (observed, refused) = match r {
        Ok(_) => (
            "cross-chain fold computed (WRONG: silent ordering)".to_string(),
            false,
        ),
        Err(e) => (format!("refused: {e}"), true),
    };
    ctx.probe_refusal("CEN-C2", &observed, refused);
}

/// CEN-C3 — an expired label is ABSENT, not false: coverage degrades.
fn cen_c3(ctx: &mut Ctx, iris: &CensusIris) {
    let iri = "urn:census:graph:seasonal";
    let g = ctx
        .store
        .graph_create(iri)
        .expect("seasonal graph registers");
    let ts = ctx.tick();
    let label = GraphLabel {
        freshness: Some(lattice::Freshness::Fresh),
        ..Default::default()
    };
    // Expires long before any real 'now': the declaration reads as absent.
    ctx.store
        .set_graph_label_until(
            iri,
            &label,
            &ts,
            Some("2026-02-01T00:00:00Z"),
            Some("census:composition"),
        )
        .expect("expiring label lands");
    let labels = ctx
        .store
        .dataset_labels(&[iris.district_g[0], g])
        .expect("fold computes");
    let floored = ctx.store.check_label_floor(&[iris.district_g[0], g]);
    let degraded = format!("{:?}", labels.freshness.coverage) != "Full";
    let observed = format!(
        "coverage={:?}; floor={}",
        labels.freshness.coverage,
        if floored.is_err() {
            "refused"
        } else {
            "passed (WRONG)"
        }
    );
    ctx.probe_refusal("CEN-C3", &observed, degraded && floored.is_err());
}

/// CEN-C4 — obligations JOIN: one no-export graph taints the set.
fn cen_c4(ctx: &mut Ctx, iris: &CensusIris) {
    let iri = "urn:census:graph:sealed";
    let g = ctx.store.graph_create(iri).expect("sealed graph registers");
    let ts = ctx.tick();
    let label = GraphLabel {
        freshness: Some(lattice::Freshness::Fresh),
        policy: Some(lattice::PolicyClass::new(["no-export"])),
        ..Default::default()
    };
    ctx.store
        .set_graph_label(iri, &label, &ts, Some("census:composition"))
        .expect("sealed label lands");
    let labels = ctx
        .store
        .dataset_labels(&[iris.district_g[0], g])
        .expect("fold computes");
    let tokens = format!("{:?}", labels.policy);
    let joined = tokens.contains("no-export");
    let observed = format!("composed policy fold: {tokens}");
    ctx.probe_refusal("CEN-C4", &observed, joined);
}

/// CEN-C5 — the specificity half: clean compositions must pass.
fn cen_c5(ctx: &mut Ctx, iris: &CensusIris) {
    let mut passed = 0usize;
    let sets: [&[i64]; 4] = [
        &[iris.district_g[0]],
        &[iris.district_g[0], iris.district_g[1]],
        &[iris.district_g[1], iris.district_g[2]],
        &[iris.district_g[0], iris.district_g[1], iris.district_g[2]],
    ];
    for set in sets {
        if ctx.store.check_label_floor(set).is_ok() {
            passed += 1;
        }
    }
    let observed = format!(
        "{passed}/{} clean compositions passed the floor",
        sets.len()
    );
    ctx.probe_refusal("CEN-C5", &observed, passed == sets.len());
}

/// CEN-C6 — bind-once: an overlay cannot rebind to a different base.
fn cen_c6(ctx: &mut Ctx, iris: &CensusIris) {
    let iri = "urn:census:graph:annex-overlay";
    ctx.store
        .overlay_create(iri, iris.district_g[0])
        .expect("overlay binds to north");
    let rebind = ctx.store.overlay_create(iri, iris.district_g[1]);
    let (observed, refused) = match rebind {
        Ok(_) => ("rebind accepted (WRONG)".to_string(), false),
        Err(e) => (format!("refused: {e}"), true),
    };
    ctx.probe_refusal("CEN-C6", &observed, refused);
}

/// CEN-C7 — the provincial pack: build a second store, pack a graph,
/// unpack it here; identity is re-interned, the content hash is the pack's
/// identity across id spaces.
fn cen_c7(ctx: &mut Ctx, out_dir: &str) {
    let ts = ctx.tick();
    let province_db = format!("{out_dir}/province-{}.db", ctx.arm.as_str());
    let pack_path = format!("{out_dir}/province-{}.qpack.db", ctx.arm.as_str());
    let _ = std::fs::remove_file(&province_db);
    let _ = std::fs::remove_file(&pack_path);

    let observed = (|| -> Result<String, quipu::Error> {
        let mut province = quipu::Store::open(&province_db)?;
        let graph_iri = "urn:census:graph:province";
        let g = province.graph_create(graph_iri)?;
        let datums = vec![
            assert_datum(
                &province,
                "urn:census:province:count",
                quipu::namespace::RDF_TYPE,
                type_ref(&province, "urn:census:Tally"),
                &ts,
            ),
            assert_datum(
                &province,
                "urn:census:province:count",
                quipu::namespace::RDFS_LABEL,
                Value::Str("province tally".into()),
                &ts,
            ),
        ];
        province.transact_to_graph(&datums, &ts, Some("province"), Some("census"), g)?;
        let manifest = quipu::pack::pack(
            &province,
            graph_iri,
            &pack_path,
            &quipu::pack::PackOptions::default(),
            &ts,
        )?;
        let r = quipu::pack::unpack(&pack_path, ctx.store_db_path(), None, &ts)?;
        Ok(format!(
            "packed (hash {}); unpacked {} facts into {}",
            manifest.content_hash, r.facts, r.graph
        ))
    })();
    match observed {
        Ok(obs) => {
            let imported = crate::phases::has_facts(&ctx.store, "urn:census:province:count");
            ctx.probe_refusal(
                "CEN-C7",
                &format!("{obs}; visible here={imported}"),
                imported,
            );
        }
        Err(e) => ctx.probe_refusal("CEN-C7", &format!("failed: {e}"), false),
    }
    let _ = DISTRICTS;
}
