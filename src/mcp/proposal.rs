//! MCP tool implementations for schema evolution proposals.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::store::Store;

/// MCP tool: `quipu_propose_schema_change` -- Submit a schema evolution proposal.
pub fn tool_propose_schema_change(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let kind_str = input
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'kind' parameter".into()))?;
    let kind = crate::proposal::ProposalKind::from_json(kind_str)?;
    let target = input
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'target' parameter".into()))?;
    let diff = input
        .get("diff")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'diff' parameter".into()))?;
    let rationale = input.get("rationale").and_then(|v| v.as_str());
    let proposer = input
        .get("proposer")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'proposer' parameter".into()))?;
    let trigger_ref = input.get("trigger_ref").and_then(|v| v.as_str());
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");

    let id = store.insert_proposal(&crate::proposal::NewProposal {
        kind: &kind,
        target,
        diff,
        rationale,
        proposer,
        trigger_ref,
        created_at: timestamp,
    })?;

    Ok(serde_json::json!({
        "proposal_id": id,
        "status": "pending"
    }))
}

/// MCP tool: `quipu_list_proposals` -- List schema evolution proposals.
pub fn tool_list_proposals(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let status = input
        .get("status")
        .and_then(|v| v.as_str())
        .map(crate::proposal::ProposalStatus::from_json)
        .transpose()?;

    let proposals = store.list_proposals(status.as_ref())?;
    let items: Vec<JsonValue> = proposals
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "kind": p.kind,
                "target": p.target,
                "diff": p.diff,
                "rationale": p.rationale,
                "proposer": p.proposer,
                "trigger_ref": p.trigger_ref,
                "status": p.status,
                "decided_by": p.decided_by,
                "decided_at": p.decided_at,
                "decision_note": p.decision_note,
                "created_at": p.created_at,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "proposals": items,
        "count": items.len()
    }))
}

/// MCP tool: `quipu_accept_proposal` -- Accept a pending schema proposal.
pub fn tool_accept_proposal(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let id = input
        .get("id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| Error::InvalidValue("missing 'id' parameter".into()))?;
    let decided_by = input
        .get("decided_by")
        .and_then(|v| v.as_str())
        .unwrap_or("aegis/crew/braino");
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");
    let note = input.get("note").and_then(|v| v.as_str());

    let proposal = store.accept_proposal(id, decided_by, timestamp, note)?;

    Ok(serde_json::json!({
        "proposal_id": proposal.id,
        "status": "accepted",
        "target": proposal.target,
        "kind": proposal.kind,
    }))
}

/// MCP tool: `quipu_reject_proposal` -- Reject a pending schema proposal.
pub fn tool_reject_proposal(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let id = input
        .get("id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| Error::InvalidValue("missing 'id' parameter".into()))?;
    let decided_by = input
        .get("decided_by")
        .and_then(|v| v.as_str())
        .unwrap_or("aegis/crew/braino");
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("1970-01-01T00:00:00Z");
    let note = input
        .get("note")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'note' (rejection reason) parameter".into()))?;

    let proposal = store.reject_proposal(id, decided_by, timestamp, note)?;

    Ok(serde_json::json!({
        "proposal_id": proposal.id,
        "status": "rejected",
        "target": proposal.target,
    }))
}
