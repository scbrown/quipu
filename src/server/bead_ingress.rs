//! Native ingress adapter for authoritative `br` work-item lifecycle events.

use axum::extract::State;
use serde_json::{Value as JsonValue, json};

use super::SharedStore;
use super::admission::write_blocking;
use super::base::AppError;
use super::tools::finish_deferred_embed;

pub(crate) fn tool_bead_ingress(
    store: &mut quipu::Store,
    input: &JsonValue,
) -> quipu::Result<JsonValue> {
    let bead_id = input
        .get("id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| quipu::Error::InvalidValue("missing bead 'id'".into()))?;
    let closed_at = input
        .get("closed_at")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| quipu::Error::InvalidValue("missing bead 'closed_at'".into()))?;
    let title = input.get("title").and_then(JsonValue::as_str).unwrap_or("");
    let observation = format!("br-closed-{bead_id}-{closed_at}");
    let episode = json!({
        "name": observation,
        "episode_body": format!("{bead_id} closed in the authoritative br store: {title}"),
        "source": format!("br lifecycle poll {bead_id}"),
        "group_id": "aegis-ontology",
        "nodes": [
            {"name": bead_id, "type": "Bead", "properties": {"beadId": bead_id}},
            {"name": observation, "type": "Observation",
             "description": format!("Observed close of {bead_id} at {closed_at}.")}
        ],
        "edges": [{"source": observation, "target": bead_id, "relation": "observes"}]
    });
    quipu::tool_episode(store, &episode)
}

pub(crate) async fn bead_ingress(
    State(s): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    write_blocking(move || {
        let (result, deferred) = {
            let mut store = s.lock();
            let result = tool_bead_ingress(&mut store, &input)?;
            (result, store.take_deferred_embed())
        };
        if let Some(deferred) = deferred {
            finish_deferred_embed(&s, &deferred)?;
        }
        Ok(axum::Json(result))
    })
    .await
}
