//! WAL checkpointing and size reporting (aegis-raq1ok).
//!
//! The deployed store reached a **1.06 GB** write-ahead log against a 5.7 GB
//! database, stable across samples and unchanged by a process restart, while
//! `wal_autocheckpoint` sat at SQLite's default of 1000 pages. Writes append to
//! the WAL and complete normally; reads then work against a WAL that cannot be
//! retired, and stall for 9-21 s at a time with work pending — long enough for
//! a 10 s liveness probe to read the store as dead and restart it.
//!
//! The mechanism to keep in mind when changing anything here: **SQLite cannot
//! checkpoint past the oldest live reader.** The server holds a pool of
//! long-lived read connections, so a checkpoint can be attempted, blocked, and
//! leave the WAL exactly where it was — which is what "checkpoints are running"
//! looked like from the outside while the WAL grew 250x past its threshold.
//! That is why `wal_bytes` is a metric and not an assumption.

use rusqlite::Connection;

use crate::Result;

/// Outcome of a PASSIVE checkpoint: what SQLite actually managed to do.
///
/// Reported rather than discarded because "the checkpoint ran" and "the
/// checkpoint retired the log" are different facts, and this incident is
/// entirely about the gap between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointOutcome {
    /// Pages currently in the WAL after the attempt.
    pub wal_pages: i64,
    /// Pages successfully moved into the database file.
    pub moved_pages: i64,
    /// True when SQLite reported it was blocked (busy) rather than complete.
    pub blocked: bool,
}

impl CheckpointOutcome {
    /// Did this attempt actually retire the whole log?
    #[must_use]
    pub fn fully_retired(&self) -> bool {
        !self.blocked && self.wal_pages == self.moved_pages
    }
}

/// Run a **PASSIVE** checkpoint: move what can be moved, never block.
///
/// PASSIVE is the only safe mode to call on a live serving store. `FULL`,
/// `RESTART` and `TRUNCATE` wait for readers, and waiting for readers on this
/// server is precisely the stall being fixed — a blocking checkpoint here would
/// convert an intermittent read stall into a guaranteed one.
pub(crate) fn checkpoint_passive(conn: &Connection) -> Result<CheckpointOutcome> {
    // wal_checkpoint returns (busy, wal_pages, moved_pages).
    let (busy, wal_pages, moved_pages): (i64, i64, i64) =
        conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    Ok(CheckpointOutcome {
        wal_pages,
        moved_pages,
        blocked: busy != 0,
    })
}

/// Run a **TRUNCATE** checkpoint: retire the log AND reset the file to zero.
///
/// This BLOCKS on readers, so it is safe only where no reader can exist —
/// in practice at startup, before the read pool is created. That restriction is
/// the whole point: dearing's manual repair took the deployed WAL from
/// 1,062,898,232 bytes to 78,312 in 0.0 s *with the service stopped*, and the
/// return values said `log_frames=0 checkpointed=0` — the content had already
/// been checkpointed, and only the file reset had never happened, because
/// SQLite cannot reset the WAL past the oldest live reader and the pool holds
/// four of them for the process's whole life.
///
/// So the growth was stale FILE SPACE, not unwritten data. PASSIVE can never
/// fix it; only this can, and only before the readers exist.
pub(crate) fn checkpoint_truncate(conn: &Connection) -> Result<CheckpointOutcome> {
    let (busy, wal_pages, moved_pages): (i64, i64, i64) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    Ok(CheckpointOutcome {
        wal_pages,
        moved_pages,
        blocked: busy != 0,
    })
}

#[cfg(test)]
mod tests {
    use crate::Store;

    /// A file-backed store reports a WAL size and a PASSIVE checkpoint retires
    /// the log when nothing is reading it.
    ///
    /// The anti-vacuity arm matters here: if the WAL were empty the "retired"
    /// assertion would hold for free, so the test WRITES first and asserts the
    /// log was non-empty before checkpointing it.
    #[test]
    fn passive_checkpoint_retires_the_log_when_no_reader_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let path = path.to_str().unwrap();
        let mut store = Store::open(path).unwrap();
        for i in 0..200 {
            let e = store.intern(&format!("http://ex/e{i}")).unwrap();
            let a = store.intern("http://ex/p").unwrap();
            let v = store.intern(&format!("http://ex/v{i}")).unwrap();
            store
                .transact(
                    &[crate::store::Datum {
                        entity: e,
                        attribute: a,
                        value: crate::types::Value::Ref(v),
                        valid_from: "2026-09-06T00:00:00Z".into(),
                        valid_to: None,
                        op: crate::types::Op::Assert,
                    }],
                    "2026-09-06T00:00:00Z",
                    None,
                    None,
                )
                .unwrap();
        }
        // ANTI-VACUITY: there must be something to retire.
        let before = store
            .wal_bytes()
            .expect("file-backed store reports a WAL size");
        assert!(
            before > 0,
            "fixture broken: the WAL is empty, so 'the checkpoint retired it' is free"
        );

        let out = store.checkpoint_wal_passive().unwrap();
        assert!(
            out.fully_retired(),
            "PASSIVE checkpoint did not retire the log with no reader holding it: \
             {} of {} pages moved, blocked={}",
            out.moved_pages,
            out.wal_pages,
            out.blocked
        );
    }

    /// `wal_bytes` reports UNKNOWN for an in-memory store rather than 0.
    ///
    /// This is the guard against the exact shape of the incident: a gauge that
    /// answers "0" when it cannot see the WAL lets a blind instrument assert
    /// health. Absence must stay distinguishable from emptiness.
    #[test]
    fn wal_bytes_is_unknown_not_zero_for_a_store_with_no_wal_on_disk() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.wal_bytes(),
            None,
            "an in-memory store reported a WAL SIZE; 0 here would read as 'the log is empty' \
             when the truth is 'there is no log to look at' (aegis-raq1ok)"
        );
    }
}
