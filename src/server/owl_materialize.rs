//! Scheduled materialisation with independent admission and short apply batches.

use serde_json::{Value, json};

use super::SharedStore;
use super::admission::write_blocking;
use super::base::{AppError, blocking};
use quipu::owl::snapshot::Snapshot;

// Not WRITE_ADMISSION: holding that permit during derivation would block every
// ordinary write even with the store mutex released. Reject overlap rather than
// queueing additional uncancellable, memory-heavy derivations.
static MATERIALIZE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

pub(crate) async fn materialize(
    store: SharedStore,
    input: Value,
) -> Result<axum::Json<Value>, AppError> {
    let permit = MATERIALIZE
        .try_acquire()
        .map_err(|_| quipu::Error::InvalidValue("OWL materialisation already running".into()))?;
    if store.readers.len() == 0 {
        return Err(quipu::Error::InvalidValue(
            "scheduled OWL materialisation requires a read pool; refusing writer fallback".into(),
        )
        .into());
    }
    let timestamp = input
        .get("timestamp")
        .and_then(Value::as_str)
        .map_or_else(quipu::time::now_iso, str::to_owned);
    let started = std::time::Instant::now();
    let reader = store.clone();
    // Move the permit into blocking work: a disconnected client must not release
    // single-flight admission while its derivation still consumes resources.
    let (mut snapshot, mut permit) = blocking(move || {
        let mut snapshot = {
            let source = reader.read();
            Snapshot::capture(&source, Snapshot::temporary_store()?, &timestamp)?
        };
        snapshot.derive()?;
        Ok((snapshot, permit))
    })
    .await?;
    let mut applied = 0;
    while snapshot.remaining() > 0 {
        let writer = store.clone();
        let result = write_blocking(move || {
            let (n, work) = {
                let mut live = writer.lock();
                let n = snapshot.apply_batch(&mut live)?;
                (n, live.take_deferred_embed())
            };
            if let Some(work) = work {
                super::tools::finish_deferred_embed(&writer, &work)?;
            }
            Ok((snapshot, permit, n))
        })
        .await?;
        (snapshot, permit) = (result.0, result.1);
        applied += result.2;
    }
    write_blocking(move || {
        let _permit = permit;
        let work = {
            let mut live = store.lock();
            snapshot.finish(&mut live)?;
            live.take_deferred_embed()
        };
        if let Some(work) = work {
            super::tools::finish_deferred_embed(&store, &work)?;
        }
        let r = &snapshot.report;
        Ok(axum::Json(json!({
            "action": "materialize", "ontologies": snapshot.ontology_count(),
            "complete": true, "snapshot_tx": snapshot.premise_head,
            "elapsed_ms": started.elapsed().as_millis(), "applied_proposals": applied,
            "materialized": if snapshot.ontology_count() == 0 { Value::Null } else { json!({
                "total": r.total, "passes": r.passes,
                "subclass_inferences": r.subclass_inferences,
                "same_as_inferences": r.same_as_inferences,
                "sub_property_inferences": r.sub_property_inferences,
                "inverse_inferences": r.inverse_inferences,
                "symmetric_inferences": r.symmetric_inferences,
                "equivalent_class_inferences": r.equivalent_class_inferences,
                "domain_range_inferences": r.domain_range_inferences,
                "transitive_inferences": r.transitive_inferences,
                "equivalent_property_inferences": r.equivalent_property_inferences
            })}
        })))
    })
    .await
}

#[cfg(test)]
#[path = "owl_materialize_tests.rs"]
mod tests;
