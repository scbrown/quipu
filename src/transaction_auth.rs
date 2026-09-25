//! Authenticated request provenance, separate from caller-declared actors.
//!
//! HTTP adapters capture an owned identity before dispatch and enter this scope
//! only inside synchronous store work. No identity is inferred from input JSON.

use std::cell::RefCell;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// The credential evidence attached to a locally committed transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub principal: String,
    pub credential_id: Option<String>,
    pub auth_class: String,
}

thread_local! {
    static IDENTITY: RefCell<Option<Identity>> = const { RefCell::new(None) };
}

/// Run synchronous work with owned authentication evidence. Nested scopes,
/// errors, and unwinding restore the previous value before a worker is reused.
/// Library/CLI callers that do not enter a scope remain unattributed.
pub fn with_identity<T>(identity: Option<Identity>, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Identity>);
    impl Drop for Restore {
        fn drop(&mut self) {
            IDENTITY.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(IDENTITY.with(|slot| slot.replace(identity)));
    work()
}

/// Create a locally authored transaction inside the caller's savepoint.
pub(crate) fn begin(
    conn: &Connection,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
) -> crate::Result<i64> {
    conn.execute(
        "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
        params![timestamp, actor, source],
    )?;
    let tx_id = conn.last_insert_rowid();
    record(conn, tx_id)?;
    Ok(tx_id)
}

/// Called immediately after creating a transaction, inside its savepoint.
/// The evidence therefore commits or rolls back with the exact transaction.
pub(crate) fn record(conn: &Connection, tx_id: i64) -> crate::Result<()> {
    IDENTITY.with(|slot| {
        if let Some(identity) = slot.borrow().as_ref() {
            conn.execute(
                "INSERT INTO transaction_auth (tx, principal, credential_id, auth_class) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    tx_id,
                    identity.principal,
                    identity.credential_id,
                    identity.auth_class
                ],
            )?;
        }
        Ok(())
    })
}

impl crate::Store {
    /// Read authenticated evidence, never substituting the declared actor.
    /// Historical and foreign copied transactions have no local credential proof.
    pub fn transaction_auth(&self, tx_id: i64) -> crate::Result<Option<Identity>> {
        // Read-only opens of older stores do not perform schema migrations.
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='transaction_auth')",
            [], |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT principal, credential_id, auth_class FROM transaction_auth WHERE tx=?1",
                [tx_id],
                |row| {
                    Ok(Identity {
                        principal: row.get(0)?,
                        credential_id: row.get(1)?,
                        auth_class: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn identity(name: &str) -> Identity {
        Identity {
            principal: format!("urn:crew:{name}"),
            credential_id: Some(name.into()),
            auth_class: "named_bearer".into(),
        }
    }

    #[test]
    fn credential_evidence_is_separate_and_scope_does_not_leak() {
        let mut store = Store::open_in_memory().unwrap();
        let id = with_identity(Some(identity("alice")), || {
            store
                .transact(&[], "2026-01-01T00:00:00Z", Some("spoofed"), Some("source"))
                .unwrap()
        });
        assert_eq!(store.transaction_auth(id).unwrap(), Some(identity("alice")));
        let tx = store.get_transaction(id).unwrap().unwrap();
        assert_eq!(tx.actor.as_deref(), Some("spoofed"));
        assert_eq!(tx.source.as_deref(), Some("source"));
        let plain = store
            .transact(&[], "2026-01-01T00:00:00Z", None, None)
            .unwrap();
        assert_eq!(store.transaction_auth(plain).unwrap(), None);
    }

    #[test]
    fn rollback_removes_evidence_and_unwind_restores_worker() {
        let mut store = Store::open_in_memory().unwrap();
        store.conn.execute_batch("SAVEPOINT outer_test").unwrap();
        let id = with_identity(Some(identity("alice")), || {
            store
                .transact(&[], "2026-01-01T00:00:00Z", None, None)
                .unwrap()
        });
        assert!(store.transaction_auth(id).unwrap().is_some());
        store
            .conn
            .execute_batch("ROLLBACK TO outer_test; RELEASE outer_test")
            .unwrap();
        assert!(store.get_transaction(id).unwrap().is_none());
        assert!(store.transaction_auth(id).unwrap().is_none());
        let _ =
            std::panic::catch_unwind(|| with_identity(Some(identity("bob")), || panic!("fixture")));
        let plain = store
            .transact(&[], "2026-01-01T00:00:00Z", None, None)
            .unwrap();
        assert!(store.transaction_auth(plain).unwrap().is_none());
    }

    #[test]
    fn concurrent_workers_and_nested_scopes_are_isolated() {
        let workers: Vec<_> = ["alice", "bob"]
            .into_iter()
            .map(|name| {
                std::thread::spawn(move || {
                    let mut store = Store::open_in_memory().unwrap();
                    with_identity(Some(identity(name)), || {
                        with_identity(None, || {
                            let tx = store
                                .transact(&[], "2026-01-01T00:00:00Z", None, None)
                                .unwrap();
                            assert!(store.transaction_auth(tx).unwrap().is_none());
                        });
                        let tx = store
                            .transact(&[], "2026-01-01T00:00:00Z", None, None)
                            .unwrap();
                        assert_eq!(store.transaction_auth(tx).unwrap(), Some(identity(name)));
                    });
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    }
}
