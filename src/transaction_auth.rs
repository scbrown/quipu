//! Authenticated request provenance, separate from caller-declared actors.
//!
//! HTTP adapters capture an owned identity before dispatch and enter this scope
//! only inside synchronous store work. No identity is inferred from input JSON.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// The credential evidence attached to a locally committed transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub principal: String,
    pub credential_id: Option<String>,
    pub auth_class: String,
    /// A signed write's replay token, carried INSIDE the identity so every
    /// dispatch site that scopes an identity also scopes it (aegis-bys8d1).
    #[serde(skip)]
    pub attestation: AttestationSlot,
}

/// Where a signed write's nonce stands for the request that carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttestState {
    Unspent,
    Spent,
    /// Refused at settle time; `verdict` is the wire code (`revoked`, ...).
    Refused {
        verdict: &'static str,
        message: String,
    },
}

/// A verified signed write whose nonce must be spent before it may mutate.
///
/// The HTTP layer verifies the signature and binding up front WITHOUT
/// spending; the spend and a binding re-check happen when the request takes
/// the store's writer lock ([`crate::Store::settle_attestation`]), in the same
/// lock hold as its work, and `record` refuses to open a transaction for a
/// refused request. So a replay is refused even when the first use changed
/// nothing, and a revocation that lands while the write is queued is honoured.
#[derive(Debug)]
pub struct PendingAttestation {
    pub session: String,
    pub nonce: String,
    pub key_id: String,
    pub introducer: String,
    state: Mutex<AttestState>,
}

impl PendingAttestation {
    #[must_use]
    pub fn new(session: String, nonce: String, key_id: String, introducer: String) -> Arc<Self> {
        Arc::new(Self {
            session,
            nonce,
            key_id,
            introducer,
            state: Mutex::new(AttestState::Unspent),
        })
    }

    #[must_use]
    pub fn state(&self) -> AttestState {
        self.state
            .lock()
            .expect("attestation state poisoned")
            .clone()
    }
}

/// Carries the pending attestation through `Identity` without affecting its
/// equality: two identities are the same principal whatever their tokens.
#[derive(Clone, Debug, Default)]
pub struct AttestationSlot(pub Option<Arc<PendingAttestation>>);

impl PartialEq for AttestationSlot {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for AttestationSlot {}

/// Refuse NOW if this thread's signed write has been refused. For store work
/// that mutates without opening a transaction (so `record` never runs): call it
/// right after taking the writer lock, where the settle has just happened.
pub fn refuse_if_refused() -> crate::Result<()> {
    match current_attestation().map(|p| p.state()) {
        Some(AttestState::Refused { verdict, message }) => Err(crate::Error::InvalidValue(
            format!("attestation refused ({verdict}): {message}"),
        )),
        _ => Ok(()),
    }
}

/// The pending attestation of the identity scoped on this thread, if any.
#[must_use]
pub fn current_attestation() -> Option<Arc<PendingAttestation>> {
    IDENTITY.with(|slot| slot.borrow().as_ref().and_then(|i| i.attestation.0.clone()))
}

/// Re-check the binding and spend the nonce, once per request, on `conn`.
///
/// Idempotent within a request: once `Spent` it returns `Ok`, once `Refused`
/// it returns the same refusal. The binding is re-read on `conn`, so the check
/// sees every revocation committed before this point.
pub(crate) fn settle_on(
    conn: &Connection,
    pending: &PendingAttestation,
    now: u64,
) -> crate::Result<()> {
    let mut state = pending.state.lock().expect("attestation state poisoned");
    let refuse = |state: &mut AttestState, verdict: &'static str, message: String| {
        *state = AttestState::Refused {
            verdict,
            message: message.clone(),
        };
        crate::Error::InvalidValue(format!("attestation refused ({verdict}): {message}"))
    };
    match &*state {
        AttestState::Spent => return Ok(()),
        AttestState::Refused { verdict, message } => {
            return Err(crate::Error::InvalidValue(format!(
                "attestation refused ({verdict}): {message}"
            )));
        }
        AttestState::Unspent => {}
    }
    let binding = crate::store::attestation::binding_on(conn, &pending.session)?;
    let problem = match &binding {
        None => Some(("unbound", "the session is no longer registered")),
        Some(b) if b.revoked => Some(("revoked", "the session binding was revoked")),
        Some(b) if !b.allow_write => Some(("scope", "the session binding is not granted write")),
        Some(b) if now > b.expires_at_epoch || now < b.issued_at_epoch => {
            Some(("expired", "the session binding is expired or not yet valid"))
        }
        Some(b) if b.key_id != pending.key_id || b.introducer != pending.introducer => Some((
            "invalid",
            "the session binding changed after the signature was checked",
        )),
        Some(_) => None,
    };
    if let Some((verdict, message)) = problem {
        return Err(refuse(&mut state, verdict, message.to_owned()));
    }
    match crate::store::attestation::consume_nonce_on(conn, &pending.session, &pending.nonce, now) {
        Ok(true) => {
            *state = AttestState::Spent;
            Ok(())
        }
        Ok(false) => Err(refuse(
            &mut state,
            "replay",
            "the nonce was already spent".to_owned(),
        )),
        Err(e) => Err(refuse(&mut state, "error", e.to_string())),
    }
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
            // A signed write mutates only once its nonce is spent and its
            // binding re-checked. Normally the writer-lock hook has already
            // settled it; this is the backstop that makes a refused request
            // unable to open a transaction at all.
            if let Some(pending) = identity.attestation.0.as_deref() {
                settle_on(conn, pending, crate::time::epoch_secs())?;
            }
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
                        attestation: Default::default(),
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
            attestation: Default::default(),
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

#[cfg(test)]
#[path = "transaction_auth_attest_tests.rs"]
mod attest_tests;
