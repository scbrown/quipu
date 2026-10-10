//! Durable share review state and events. Notifications never adopt shapes.
//! # arming: library — import/promote call directly; age notices require an explicit consumer policy.
use crate::share_import::{
    PromoteImportRequest, PromoteImportResult, ShareImportRequest, ShareImportResult,
};
use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

fn atomic<T>(store: &mut Store, run: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
    store
        .conn
        .execute_batch("SAVEPOINT share_review_transition")?;
    match run(store) {
        Ok(value) => {
            store
                .conn
                .execute_batch("RELEASE share_review_transition")?;
            Ok(value)
        }
        Err(error) => {
            store.conn.execute_batch(
                "ROLLBACK TO share_review_transition; RELEASE share_review_transition",
            )?;
            store.read_model.borrow_mut().clear();
            store.term_cache.borrow_mut().clear_persistent();
            Err(error)
        }
    }
}
fn state(store: &Store, id: &str) -> Result<Option<String>> {
    Ok(store
        .conn
        .query_row(
            "SELECT state FROM import_reviews WHERE share_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?)
}
fn open(store: &Store, id: &str) -> Result<()> {
    if state(store, id)?.is_some_and(|s| matches!(s.as_str(), "rejected" | "expired")) {
        return Err(Error::InvalidValue(
            "import review is closed; explicitly reopen before importing or promoting".into(),
        ));
    }
    Ok(())
}
fn emit(store: &Store, kind: &str, id: &str, ts: &str, payload: &Value) -> Result<()> {
    store.conn.execute(
        "INSERT INTO events(type,ts,subject,group_id,tx_id,payload) VALUES(?1,?2,?3,NULL,?4,?5)",
        params![kind, ts, id, store.latest_tx_id()?, payload.to_string()],
    )?;
    Ok(())
}
fn transition(
    store: &Store,
    id: &str,
    next: &str,
    kind: &str,
    ts: &str,
    payload: &Value,
) -> Result<()> {
    if state(store, id)?.as_deref() == Some(next) {
        return Ok(());
    }
    let previous: Option<(String, String)> = store
        .conn
        .query_row(
            "SELECT first_seen,payload FROM import_reviews WHERE share_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let mut details = if let Some((_, body)) = &previous {
        serde_json::from_str::<Value>(body).map_err(|e| Error::Serialization(e.to_string()))?
    } else {
        json!({})
    };
    for (key, value) in payload.as_object().unwrap() {
        details[key] = value.clone();
    }
    if let Some((first, _)) = previous {
        let age: Option<i64> = store.conn.query_row(
            "SELECT MAX(0,unixepoch(?1)-unixepoch(?2))",
            params![ts, first],
            |r| r.get(0),
        )?;
        details["waiting_seconds"] = json!(age);
    }
    store.conn.execute("INSERT INTO import_reviews(share_id,state,first_seen,updated_at,payload) VALUES(?1,?2,?3,?3,?4) ON CONFLICT(share_id) DO UPDATE SET state=excluded.state,updated_at=excluded.updated_at,notice_policy=NULL,payload=excluded.payload",params![id,next,ts,details.to_string()])?;
    emit(store, kind, id, ts, &details)
}
/// Verify/stage a share and record its review transition in the same savepoint.
pub fn import_share(
    store: &mut Store,
    request: &ShareImportRequest,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<ShareImportResult> {
    atomic(store, |store| {
        open(store, &request.manifest.share_id)?;
        let old = state(store, &request.manifest.share_id)?;
        let result = crate::share_import::import_inner(store, request, timestamp, actor)?;
        let next = if result.promotion.eligible {
            "staged"
        } else {
            "quarantined"
        };
        // Replaying a promoted pack is not a new unresolved decision.
        if old.as_deref() != Some("promoted") {
            let kind = if next == "staged" && old.as_deref() == Some("quarantined") {
                "import.adopted"
            } else if next == "staged" {
                "import.staged"
            } else {
                "import.quarantined"
            };
            let payload = json!({"share_id":result.share_id,"source":request.source,"actor":actor,"claimed_actor":request.actor,"unknown_types":result.validation.off_vocabulary,"blockers":result.promotion.blockers,"shapes_hash":request.manifest.shapes_hash,"triples":result.triples,"size_bytes":request.export_ntriples.len()+request.shapes_turtle.len()+request.queries_turtle.as_ref().map_or(0,String::len),"waiting_seconds":0});
            transition(store, &result.share_id, next, kind, timestamp, &payload)?;
        }
        Ok(result)
    })
}
/// Explicit promotion; success and resolution event commit atomically.
pub fn promote_import(
    store: &mut Store,
    request: &PromoteImportRequest,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<PromoteImportResult> {
    atomic(store, |store| {
        open(store, &request.share_id)?;
        let result = crate::share_import::promote_inner(store, request, timestamp, actor)?;
        transition(
            store,
            &request.share_id,
            "promoted",
            "import.promoted",
            timestamp,
            &json!({"share_id":request.share_id,"actor":actor,"claimed_actor":request.actor,"triples":result.triples}),
        )?;
        Ok(result)
    })
}
/// Bounded pending-review page. Read-only, including on WAL reader connections.
/// Cursor is the last share ID, not an event offset; state survives log retention.
pub fn pending(store: &Store, after: &str, limit: usize, timestamp: &str) -> Result<Value> {
    let mut stmt=store.conn.prepare("SELECT share_id,first_seen,payload,MAX(0,unixepoch(?1)-unixepoch(first_seen)) FROM import_reviews WHERE state='quarantined' AND share_id>?2 ORDER BY share_id LIMIT ?3")?;
    let rows = stmt.query_map(
        params![timestamp, after, limit.clamp(1, 1000) as i64 + 1],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    let mut items = Vec::new();
    for row in rows {
        let (id, first, payload, age) = row?;
        let mut value: Value =
            serde_json::from_str(&payload).map_err(|e| Error::Serialization(e.to_string()))?;
        value["share_id"] = id.into();
        value["first_seen"] = first.into();
        value["waiting_seconds"] = age.into();
        items.push(value);
    }
    let has_more = items.len() > limit.clamp(1, 1000);
    items.truncate(limit.clamp(1, 1000));
    let cursor = items
        .last()
        .and_then(|v| v["share_id"].as_str())
        .unwrap_or(after);
    Ok(json!({"next_share_id":cursor,"has_more":has_more,"reviews":items}))
}
/// Emit one age escalation per share for the current policy (threshold + route).
/// Invoke from a consumer's scheduled path; this function installs no timer.
pub fn notify_due(
    store: &mut Store,
    after: &str,
    limit: usize,
    timestamp: &str,
    age_seconds: u64,
    route: &str,
) -> Result<Value> {
    if age_seconds == 0 || route.trim().is_empty() {
        return Err(Error::InvalidValue(
            "review policy requires positive age_seconds and a route".into(),
        ));
    }
    atomic(store, |store| {
        let page = pending(store, after, limit, timestamp)?;
        let policy = json!([age_seconds, route]).to_string();
        for item in page["reviews"].as_array().unwrap() {
            if item["waiting_seconds"].as_u64().unwrap_or(0) < age_seconds {
                continue;
            }
            let id = item["share_id"].as_str().unwrap();
            let changed=store.conn.execute("UPDATE import_reviews SET notice_policy=?1 WHERE share_id=?2 AND (notice_policy IS NULL OR notice_policy!=?1)",params![policy,id])?;
            if changed > 0 {
                let mut payload = item.clone();
                payload["route"] = route.into();
                payload["age_seconds"] = age_seconds.into();
                emit(store, "import.review_due", id, timestamp, &payload)?;
            }
        }
        Ok(page)
    })
}
/// Explicit rejection/expiry or reopening by a named local actor; retain bytes.
/// These are review decisions, never shape adoption or deletion of pack content.
pub fn decide(
    store: &mut Store,
    id: &str,
    decision: &str,
    actor: &str,
    reason: &str,
    timestamp: &str,
) -> Result<()> {
    if actor.trim().is_empty()
        || reason.trim().is_empty()
        || !matches!(decision, "rejected" | "expired" | "reopen")
    {
        return Err(Error::InvalidValue(
            "review decision requires actor, reason and rejected|expired|reopen".into(),
        ));
    }
    atomic(store, |store| {
        let old =
            state(store, id)?.ok_or_else(|| Error::InvalidValue("unknown import review".into()))?;
        if old == "staged" || old == "promoted" {
            return Err(Error::InvalidValue(
                "only quarantine reviews can be closed or reopened".into(),
            ));
        }
        let payload: String = store.conn.query_row(
            "SELECT payload FROM import_reviews WHERE share_id=?1",
            [id],
            |r| r.get(0),
        )?;
        let mut payload: Value =
            serde_json::from_str(&payload).map_err(|e| Error::Serialization(e.to_string()))?;
        payload["decision_actor"] = actor.into();
        payload["decision_reason"] = reason.into();
        let next = if decision == "reopen" {
            "quarantined"
        } else {
            decision
        };
        transition(
            store,
            id,
            next,
            &format!(
                "import.{}",
                if decision == "reopen" {
                    "reopened"
                } else {
                    decision
                }
            ),
            timestamp,
            &payload,
        )
    })
}

#[cfg(test)]
#[path = "share_review_tests.rs"]
mod tests;
