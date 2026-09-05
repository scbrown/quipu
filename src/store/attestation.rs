//! Durable session bindings and replay state for the common attestation
//! verifier (aegis-c9c44, unit A).
//!
//! # Why this exists at all
//!
//! [`crate::session_attestation::BindingRegistry`] keeps bindings in a
//! `HashMap` and spent nonces in a `HashSet`. Both are correct and both are
//! forgotten on restart, which makes the replay defence a property of process
//! uptime rather than of the system: every nonce a crashed server had consumed
//! becomes spendable again the moment it comes back. The stand-down note on
//! aegis-c9c44 put it plainly — an in-memory replay set is not production
//! enforcement.
//!
//! # The property this file is FOR
//!
//! Nonce consumption must succeed or fail *with the mutation it authorises*.
//! Because the spend is an ordinary SQL insert on the store's own connection,
//! it lands inside whatever `SAVEPOINT` the caller already holds, and so:
//!
//! * a mutation that is rolled back takes its nonce spend with it, and the
//!   legitimate holder can retry — a rejected write must not burn credentials
//! * a mutation that is released keeps its nonce spend, and the replay is
//!   refused for good, across restarts
//!
//! Those two are opposite failure directions, so both are asserted by tests
//! rather than one being inferred from the other: "the nonce is gone" and "the
//! nonce was never written" look identical from the outside, and only forcing
//! a successful accept as the control tells them apart.
//!
//! # What this file deliberately does NOT do
//!
//! It adds no call sites. At the time of writing `session_attestation` is
//! reachable from nothing in the crate (one hit repo-wide: its own `pub mod`
//! line in `lib.rs`), which makes this change behaviourally inert to deploy —
//! and that is the reason it ships before the `/import` wiring rather than
//! with it.

#![cfg(not(target_arch = "wasm32"))]

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::error::{Error, Result};
use crate::session_attestation::{AttestationBindings, SessionBinding, nonce_horizon_secs};

impl Store {
    /// Install a protected binding, as a trusted introducer.
    ///
    /// Semantics match the in-memory registry exactly, because the two must be
    /// interchangeable behind [`AttestationBindings`]: re-registering a
    /// byte-identical binding is a no-op, a different binding for a live
    /// session is refused, and a public key already bound to another session
    /// is refused. The last of those is the one worth stating out loud — two
    /// sessions sharing a key means a nonce spent by one is not spent by the
    /// other, and the replay window reopens through the second door.
    pub fn attestation_register(&self, binding: &SessionBinding) -> Result<()> {
        if let Some(existing) = self.attestation_binding(&binding.session)? {
            return if existing == *binding {
                Ok(())
            } else {
                Err(Error::InvalidValue(format!(
                    "conflicting session binding: {}",
                    binding.session
                )))
            };
        }
        let key_owner: Option<String> = self
            .conn
            .query_row(
                "SELECT session FROM attestation_bindings WHERE key_id = ?1",
                params![binding.key_id],
                |row| row.get(0),
            )
            .optional()?;
        if key_owner.is_some() {
            return Err(Error::InvalidValue(format!(
                "session public key already bound: {}",
                binding.key_id
            )));
        }
        self.conn.execute(
            "INSERT INTO attestation_bindings
                 (session, agent, public_key, key_id, introducer,
                  issued_at_epoch, expires_at_epoch, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                binding.session,
                binding.agent,
                binding.public_key,
                binding.key_id,
                binding.introducer,
                // SQLite integers are signed. These are epoch seconds, so the
                // saturation below is unreachable for any real clock; it is
                // here so the conversion cannot panic on a nonsense binding
                // rather than because it is expected to fire.
                i64::try_from(binding.issued_at_epoch).unwrap_or(i64::MAX),
                i64::try_from(binding.expires_at_epoch).unwrap_or(i64::MAX),
                i64::from(binding.revoked),
            ],
        )?;
        Ok(())
    }

    /// Mark a session revoked. Rows are never deleted: a binding that existed
    /// is a fact about the past, and a revoked row refuses where a missing row
    /// would merely be unbound — the two are different findings for whoever is
    /// reading the refusal.
    pub fn attestation_revoke(&self, session: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE attestation_bindings SET revoked = 1 WHERE session = ?1",
            params![session],
        )?;
        if changed == 0 {
            return Err(Error::InvalidValue(format!("unbound session: {session}")));
        }
        Ok(())
    }

    /// Read one protected binding.
    ///
    /// `key_id` is RECOMPUTED from the stored public key rather than trusted
    /// from its own column, so the invariant `SessionBinding::new` establishes
    /// holds on every read as well as every write. The column remains, because
    /// the uniqueness constraint above needs something to index.
    pub fn attestation_binding(&self, session: &str) -> Result<Option<SessionBinding>> {
        let row = self
            .conn
            .query_row(
                "SELECT agent, public_key, introducer, issued_at_epoch,
                        expires_at_epoch, revoked
                   FROM attestation_bindings WHERE session = ?1",
                params![session],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((agent, public_key, introducer, issued, expires, revoked)) = row else {
            return Ok(None);
        };
        let mut binding = SessionBinding::new(
            agent,
            session,
            public_key,
            introducer,
            issued.unsigned_abs(),
            expires.unsigned_abs(),
        )?;
        binding.revoked = revoked != 0;
        Ok(Some(binding))
    }

    /// Forget nonces that can no longer be replayed.
    ///
    /// The horizon comes from [`nonce_horizon_secs`] and therefore from the
    /// verifier's own skew constant — an attestation older than the skew window
    /// is refused before the nonce is consulted, so remembering it longer
    /// protects nothing and the table would grow without bound. Returns the
    /// number of rows forgotten so a caller can report it rather than assume
    /// it.
    pub fn attestation_prune_nonces(
        &self,
        now_epoch: u64,
        allowed_skew_secs: u64,
    ) -> Result<usize> {
        let horizon = nonce_horizon_secs(allowed_skew_secs);
        let cutoff = i64::try_from(now_epoch.saturating_sub(horizon)).unwrap_or(i64::MAX);
        Ok(self.conn.execute(
            "DELETE FROM attestation_nonces WHERE consumed_at_epoch < ?1",
            params![cutoff],
        )?)
    }
}

impl AttestationBindings for Store {
    fn binding(&self, session: &str) -> Result<Option<SessionBinding>> {
        self.attestation_binding(session)
    }

    /// Spend a nonce inside the caller's savepoint.
    ///
    /// `INSERT OR IGNORE` swallows every constraint violation, not only the
    /// primary-key collision that means "replay". So a zero row count is not
    /// taken at face value: the row is read back, and its ABSENCE after an
    /// ignored insert is reported as an error rather than as a replay. Those
    /// two answers send a reader to opposite places — one to an attacker, one
    /// to the schema — and a check that cannot tell them apart would always
    /// give the reassuring one.
    fn consume_nonce(&self, session: &str, nonce: &str, now_epoch: u64) -> Result<bool> {
        let consumed_at = i64::try_from(now_epoch).unwrap_or(i64::MAX);
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO attestation_nonces (session, nonce, consumed_at_epoch)
             VALUES (?1, ?2, ?3)",
            params![session, nonce, consumed_at],
        )?;
        if inserted == 1 {
            return Ok(true);
        }
        let present: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM attestation_nonces WHERE session = ?1 AND nonce = ?2",
                params![session, nonce],
                |row| row.get(0),
            )
            .optional()?;
        if present.is_some() {
            Ok(false)
        } else {
            Err(Error::InvalidValue(format!(
                "attestation nonce could not be recorded for session {session}"
            )))
        }
    }
}
