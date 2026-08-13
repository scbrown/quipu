//! The query-budget progress guard — installs a `SQLite` progress handler
//! that interrupts statement execution past the deadline, and clears it on
//! drop. Split from `sparql/mod.rs` for the file-size ratchet.

/// Clears the `SQLite` progress handler on drop, so an early return or error
/// cannot leave a stale deadline interrupting the NEXT query on this
/// connection.
pub(super) struct ProgressGuard<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> ProgressGuard<'a> {
    /// ~4096 VM instructions between checks: coarse enough to be free on
    /// healthy queries, fine enough to stop a grinding scan within
    /// milliseconds of the deadline.
    pub(super) fn install(
        conn: &'a rusqlite::Connection,
        deadline: crate::time::Deadline,
    ) -> rusqlite::Result<Self> {
        conn.progress_handler(4096, Some(move || deadline.passed()))?;
        Ok(Self { conn })
    }
}

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        // Nothing to do if clearing fails — the handler only fires between VM
        // instructions of a running statement, and this connection isn't
        // running one during drop.
        let _ = self.conn.progress_handler(0, None::<fn() -> bool>);
    }
}
