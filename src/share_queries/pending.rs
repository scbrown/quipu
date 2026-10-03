//! A staged import's stored queries wait for `import promote` (aegis-9ofqqs).
//!
//! The data of a clean import is staged and reaches ROOT only when a human
//! promotes it. Its queries used to be installed at import, so an import
//! nobody promoted still left them live in the registry. Now a staged import
//! records its sealed `queries.ttl` here and installs nothing; promotion
//! re-vets that member against the store as it is then and installs it.

use rusqlite::{OptionalExtension, params};

use super::import::{QueryImport, install, prepare, withheld};
use crate::error::Result;
use crate::store::Store;

/// Hold a staged share's member until promotion, replacing an earlier hold
/// for the same share. Installs nothing.
///
/// # Errors
/// Store errors.
pub fn hold(
    store: &Store,
    share_id: &str,
    member: &str,
    store_id: &str,
    namespace: &str,
    replace: bool,
    timestamp: &str,
) -> Result<()> {
    store.conn.execute(
        "INSERT OR REPLACE INTO pending_share_queries \
         (share_id, member, store_id, namespace, replace, staged_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            share_id,
            member,
            store_id,
            namespace,
            i64::from(replace),
            timestamp
        ],
    )?;
    Ok(())
}

/// Install the queries held for `share_id`, then drop the hold.
///
/// Re-vetted against the CURRENT vocabulary and registry. A collision created
/// since the import is reported, not overwritten. If any query now targets a
/// class the store does not sanction, NONE is installed (as at import, where
/// one such query quarantines the whole pack) and the hold is KEPT, so
/// promoting again after the vocabulary is loaded installs them. `None` when
/// the share held no queries.
///
/// # Errors
/// Store errors, or a held member that no longer parses.
pub fn release(store: &Store, share_id: &str, timestamp: &str) -> Result<Option<QueryImport>> {
    let row: Option<(String, String, String, i64)> = store
        .conn
        .query_row(
            "SELECT member, store_id, namespace, replace \
             FROM pending_share_queries WHERE share_id = ?1",
            params![share_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((member, store_id, namespace, replace)) = row else {
        return Ok(None);
    };
    let Some(pending) = prepare(
        store,
        Some(&member),
        &store_id,
        Some(&namespace),
        replace != 0,
    )?
    else {
        return Ok(None);
    };
    if pending.off_vocabulary() {
        return Ok(Some(withheld(pending)));
    }
    let report = install(
        store,
        &pending.queries,
        &pending.namespace,
        pending.replace,
        timestamp,
    )?;
    store.conn.execute(
        "DELETE FROM pending_share_queries WHERE share_id = ?1",
        params![share_id],
    )?;
    Ok(Some(report))
}
