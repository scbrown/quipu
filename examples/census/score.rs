//! The RQ scorers. Every scorer reads the manifest (the injector's declared
//! ground truth) and asks the STORE what actually landed — never the phase
//! scripts. Metrics files carry real numbers for the scored RQs and honest
//! `pending` markers for the rest.

use crate::manifest::Manifest;
use crate::phases::Ctx;

pub fn write_metrics(ctx: &Ctx, manifest: &Manifest, out: &str) {
    let dir = format!("{out}/metrics/{}", ctx.arm.as_str());
    std::fs::create_dir_all(&dir).expect("create metrics directory");
    rq1(ctx, &dir);
    rq2(ctx, manifest, &dir);
    rq3(ctx, &dir);
    rq4(manifest, &dir);
    rq5_pending(&dir);
}

fn stats(lat: &[u128]) -> serde_json::Value {
    if lat.is_empty() {
        return serde_json::json!(null);
    }
    let mut sorted = lat.to_vec();
    sorted.sort_unstable();
    let mean = sorted.iter().sum::<u128>() as f64 / sorted.len() as f64;
    serde_json::json!({
        "n": sorted.len(),
        "mean_us": mean,
        "p50_us": sorted[sorted.len() / 2],
        "p95_us": sorted[(sorted.len() * 95) / 100],
        "max_us": sorted[sorted.len() - 1],
    })
}

/// RQ1 — gate cost. Real distributions from this run; the cross-arm delta
/// is computed by comparing the two arms' files (single-machine,
/// single-run: the determinism note governs how these are aggregated).
fn rq1(ctx: &Ctx, dir: &str) {
    let body = serde_json::json!({
        "rq": "rq1",
        "title": "gate cost: per-write latency, this arm",
        "status": "measured",
        "arm": ctx.arm.as_str(),
        "ungoverned_writes": stats(&ctx.lat_ungoverned),
        "governed_writes": stats(&ctx.lat_governed),
        "note": "compare across arms and repeats; single-run numbers are not results (BUILD_REPORT.md). \
                 Abstention applies to the POLICY gate only: authority intersection runs on every \
                 graph-scoped write by design (GS3), so gated-arm ungoverned writes are not free.",
    });
    write(dir, "rq1", &body);
}

/// RQ2 — strictness value: which planted defects are present in the final
/// graph? Presence is asked of the store, per defect subject from the
/// manifest.
fn rq2(ctx: &Ctx, manifest: &Manifest, dir: &str) {
    let mut planted = 0usize;
    let mut present = Vec::new();
    for entry in manifest.entries.iter().filter(|e| e.scored_by == "RQ2") {
        let Some(subject) = &entry.defect_subject else {
            continue;
        };
        planted += 1;
        // CEN-P2's defect is the SECOND placement value, not the subject.
        let landed = if entry.id == "CEN-P2" {
            crate::phases::value_present(
                &ctx.store,
                subject,
                "urn:census:vocab:placedIn",
                "dwelling-2",
            )
        } else {
            crate::phases::has_facts(&ctx.store, subject)
        };
        if landed {
            present.push(entry.id.clone());
        }
    }
    let body = serde_json::json!({
        "rq": "rq2",
        "title": "strictness value: planted defects present in the final graph",
        "status": "measured",
        "arm": ctx.arm.as_str(),
        "defects_planted": planted,
        "defects_present": present.len(),
        "present_ids": present,
        "expectation": if ctx.gated() { "0 present" } else { "all present" },
    });
    write(dir, "rq2", &body);
}

/// RQ3 — the in-store half so far: signed verdicts recorded as facts.
/// The audit passes and the external-checker arm land with phases 5-6
/// (quipu-tj0, quipu-4mi).
fn rq3(ctx: &Ctx, dir: &str) {
    let q = "PREFIX a: <http://aegis.gastown.local/ontology/> \
             SELECT ?v ?o WHERE { ?v a a:Verdict ; a:outcome ?o }";
    let mut satisfied = 0usize;
    let mut violated = 0usize;
    if let Ok(quipu::sparql::QueryResult::Select { rows, .. }) = quipu::sparql::query(&ctx.store, q)
    {
        for row in &rows {
            match row.get("o") {
                Some(quipu::Value::Str(s)) if s == "satisfied" => satisfied += 1,
                Some(quipu::Value::Str(_)) => violated += 1,
                _ => {}
            }
        }
    }
    let body = serde_json::json!({
        "rq": "rq3",
        "title": "audit: signed verdicts as facts (in-store half)",
        "status": "partial",
        "arm": ctx.arm.as_str(),
        "verdicts_satisfied": satisfied,
        "verdicts_other": violated,
        "pending_on": "quipu-tj0 (audit passes), quipu-4mi (external checker arm)",
    });
    write(dir, "rq3", &body);
}

/// RQ4 — composition: read straight off the phase-4 manifest entries'
/// `contract_upheld` markers.
fn rq4(manifest: &Manifest, dir: &str) {
    let entries: Vec<_> = manifest
        .entries
        .iter()
        .filter(|e| e.phase == 4 && e.status == "executed")
        .collect();
    let upheld = entries
        .iter()
        .filter(|e| {
            e.observed
                .as_deref()
                .is_some_and(|o| o.contains("contract_upheld=true"))
        })
        .count();
    let body = serde_json::json!({
        "rq": "rq4",
        "title": "composition: lattice contract upheld across probes",
        "status": "measured",
        "probes": entries.len(),
        "contract_upheld": upheld,
        "expectation": "all probes upheld (refusals where widening, admissions where clean)",
    });
    write(dir, "rq4", &body);
}

fn rq5_pending(dir: &str) {
    let body = serde_json::json!({
        "rq": "rq5",
        "title": "replay: as-of fidelity across the amendment boundary",
        "status": "pending",
        "pending_on": "quipu-krv (shape versioning), quipu-tj0 (replay scoring)",
    });
    write(dir, "rq5", &body);
}

fn write(dir: &str, rq: &str, body: &serde_json::Value) {
    let path = format!("{dir}/{rq}.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(body).expect("metric serializes"),
    )
    .expect("write metric");
}
