//! Cooperative cancellation for an atomic staged snapshot promotion.
//!
//! A lease is renewed by the producer's existing promotion polls. Cancellation
//! and the final commit claim share one mutex: an accepted cancellation cannot
//! race a successful claim. Once commit starts, cancellation is too late.

use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Running,
    Cancelled,
    Committing,
}

struct State {
    phase: Phase,
    expires: crate::time::Deadline,
}

/// Native HTTP promotion lease; no control is installed for ordinary writes.
pub struct PromotionControl {
    state: Mutex<State>,
    lease: Duration,
}

impl PromotionControl {
    /// Start a renewable lease.
    #[must_use]
    pub fn new(lease: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                phase: Phase::Running,
                expires: deadline(lease),
            }),
            lease,
        })
    }

    fn expire(state: &mut State) {
        if state.phase == Phase::Running && state.expires.passed() {
            state.phase = Phase::Cancelled;
        }
    }

    /// Renew only a still-live lease. An expired lease cannot be resurrected.
    pub fn renew(&self) -> bool {
        let mut state = self.state.lock().expect("promotion lease lock");
        Self::expire(&mut state);
        if state.phase != Phase::Running {
            return false;
        }
        state.expires = deadline(self.lease);
        true
    }

    /// Request cancellation. False means the commit boundary already won.
    pub fn cancel(&self) -> bool {
        let mut state = self.state.lock().expect("promotion lease lock");
        if state.phase == Phase::Committing {
            return false;
        }
        state.phase = Phase::Cancelled;
        true
    }

    /// Whether cancellation has been requested (not proof of worker exit).
    #[must_use]
    pub fn cancelled(&self) -> bool {
        let mut state = self.state.lock().expect("promotion lease lock");
        Self::expire(&mut state);
        state.phase == Phase::Cancelled
    }

    /// Check whether this worker must stop before changing facts.
    pub fn check(&self) -> Result<()> {
        if self.cancelled() {
            Err(cancelled_error())
        } else {
            Ok(())
        }
    }

    fn claim_commit(&self) -> Result<()> {
        let mut state = self.state.lock().expect("promotion lease lock");
        Self::expire(&mut state);
        if state.phase == Phase::Cancelled {
            return Err(cancelled_error());
        }
        state.phase = Phase::Committing;
        Ok(())
    }
}

fn deadline(lease: Duration) -> crate::time::Deadline {
    crate::time::Deadline::after_millis(lease.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn cancelled_error() -> Error {
    Error::InvalidValue("snapshot promotion cancelled before commit".into())
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<PromotionControl>>> = const { RefCell::new(None) };
}

struct Scope(Option<Arc<PromotionControl>>);
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

/// Apply a promotion lease only to this synchronous worker, restoring on panic.
pub fn scoped<T>(control: Arc<PromotionControl>, work: impl FnOnce() -> T) -> T {
    let _scope = Scope(CURRENT.with(|slot| slot.replace(Some(control))));
    work()
}

/// Cancellation checkpoint. No-op outside a leased promotion.
pub(crate) fn check() -> Result<()> {
    // Engine audit bookkeeping must outlive an aborted request. The HTTP
    // adapter selects the request kind; caller-declared actors cannot opt in.
    if crate::write_kind::current() == Some(crate::write_kind::WriteKind::Verdict) {
        return Ok(());
    }
    CURRENT.with(|slot| slot.borrow().as_ref().map_or(Ok(()), |c| c.check()))
}

/// Linearize cancellation against the snapshot's own transaction commit only.
/// Audit/refusal transactions before it must not consume the commit claim.
pub(crate) fn claim_commit(source: Option<&str>) -> Result<()> {
    if source.is_some_and(|s| s.starts_with("snapshot:")) {
        CURRENT.with(|slot| slot.borrow().as_ref().map_or(Ok(()), |c| c.claim_commit()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_and_commit_have_one_winner() {
        let cancelled = PromotionControl::new(Duration::from_secs(30));
        assert!(cancelled.cancel());
        assert!(!cancelled.renew());
        assert!(cancelled.claim_commit().is_err());
        let committed = PromotionControl::new(Duration::from_secs(30));
        committed.claim_commit().unwrap();
        assert!(!committed.cancel());
        committed.check().unwrap();
    }

    #[test]
    fn concurrent_cancel_and_commit_have_exactly_one_winner() {
        for _ in 0..32 {
            let control = PromotionControl::new(Duration::from_secs(30));
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let worker = control.clone();
            let ready = barrier.clone();
            let thread = std::thread::spawn(move || {
                ready.wait();
                worker.claim_commit().is_ok()
            });
            barrier.wait();
            let cancelled = control.cancel();
            let committed = thread.join().unwrap();
            assert_ne!(cancelled, committed);
        }
    }

    #[test]
    fn cancelled_transaction_preserves_signed_verdict_and_discards_advisories() {
        let control = PromotionControl::new(Duration::from_secs(30));
        control.cancel();
        let mut store = crate::Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let signer =
            crate::signing::SigningIdentity::load(&dir.path().join("audit.pk8"), "quipu").unwrap();
        store.set_signing_identity(Arc::new(signer));
        let entity = store.intern("http://ex/cancelled").unwrap();
        let attribute = store
            .intern("http://www.w3.org/2000/01/rdf-schema#label")
            .unwrap();
        let datum = crate::store::Datum {
            entity,
            attribute,
            value: crate::Value::Str("cancelled".into()),
            valid_from: "2026-10-17T00:00:00Z".into(),
            valid_to: None,
            op: crate::Op::Assert,
        };
        store
            .pending_verdicts
            .push(crate::governance::verdict_facts::PendingVerdict {
                predicate_id: "http://ex/policy".into(),
                target_ref: "http://ex/cancelled".into(),
                outcome: "unsatisfied".into(),
            });
        store.queue_write_event(crate::store::PendingWriteEvent {
            event_type: "test.cancelled-upload".into(),
            subject: None,
            payload: serde_json::json!({"publication":"cancelled"}),
        });
        scoped(control, || {
            assert!(
                store
                    .transact(
                        &[datum],
                        "2026-10-17T00:00:00Z",
                        Some("test"),
                        Some("snapshot:cancelled-audit-test")
                    )
                    .is_err()
            );
            assert!(
                check().is_err(),
                "audit scope must not uncancel the publication"
            );
        });
        assert!(store.entity_facts(entity).unwrap().is_empty());
        let audit: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE source='write-gate verdict'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            audit, 1,
            "the cancelled request lost its signed audit verdict"
        );
        let leaked: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type='test.cancelled-upload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            leaked, 0,
            "aborted validation advisories must not ride the audit writer"
        );
    }

    #[test]
    fn cancelled_publication_does_not_suppress_its_durable_audit_writer() {
        let control = PromotionControl::new(Duration::from_secs(30));
        control.cancel();
        let mut store = crate::Store::open_in_memory().unwrap();
        scoped(control, || {
            assert!(check().is_err());
            crate::write_kind::scoped(Some(crate::write_kind::WriteKind::Verdict), || {
                crate::tool_knot(&mut store, &serde_json::json!({
                    "turtle":"<http://ex/audit> <http://www.w3.org/2000/01/rdf-schema#label> \"audit\" .",
                    "source":"verdict:cancelled-publication",
                })).unwrap();
            });
            assert!(
                check().is_err(),
                "audit scope must not uncancel the publication"
            );
        });
        let entity = store.lookup("http://ex/audit").unwrap().unwrap();
        assert!(!store.entity_facts(entity).unwrap().is_empty());
    }

    #[test]
    fn expired_lease_never_revives_and_scope_is_restored() {
        let control = PromotionControl::new(Duration::ZERO);
        assert!(!control.renew());
        scoped(control, || assert!(check().is_err()));
        check().unwrap();
    }
}
