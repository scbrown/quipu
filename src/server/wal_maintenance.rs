//! WAL reset at startup and passive checkpointing thereafter (aegis-raq1ok).
//!
//! Split out of `server.rs` rather than inlined: the placement rule below is
//! the entire fix and it deserves somewhere to be stated, and `server.rs` is at
//! its size limit.

use quipu::Store;

use super::SharedStore;

/// Reset the WAL **before the read pool opens**.
///
/// The ordering is the fix. `TRUNCATE` blocks on readers, and the pool's read
/// connections live for the whole process, so the identical call placed a few
/// lines later is permanently blocked — which is precisely the deployed
/// condition this repairs. Called from `serve` before `read_pool` is built.
///
/// What was measured: the deployed WAL stood at 1,062,898,232 bytes and
/// `wal_checkpoint(TRUNCATE)` with the service stopped returned
/// `log_frames=0 checkpointed=0`, taking it to 78,312 bytes in 0.0 s. The
/// content had already been checkpointed; only the file reset had never
/// happened. So this reclaims stale FILE SPACE, and a PASSIVE checkpoint —
/// which never resets the file — cannot do it, however often it runs.
pub(crate) fn reset_at_startup(store: &Store) {
    match store.checkpoint_wal_truncate() {
        Ok(o) if o.fully_retired() => eprintln!("wal: reset at startup (log truncated)"),
        Ok(o) => eprintln!(
            "wal: startup TRUNCATE did not reset the log — {} of {} pages{}; \
             something already holds a read connection this early",
            o.moved_pages,
            o.wal_pages,
            if o.blocked { ", reported BUSY" } else { "" }
        ),
        Err(e) => eprintln!("wal: startup checkpoint failed: {e}"),
    }
    match store.wal_bytes() {
        Some(b) => eprintln!("wal: {b} bytes on disk after the startup reset"),
        None => eprintln!("wal: size UNKNOWN (not a file-backed store, or unreadable)"),
    }
}

/// A PASSIVE checkpoint every 60 s for the life of the process.
///
/// Deliberately periodic rather than a call after each bulk write: a per-site
/// list has to enumerate promote, every ingest path and every bulk writer added
/// later, and the one it misses is the one that grows the log.
///
/// PASSIVE never waits for a reader, so this cannot itself stall the read path
/// — the failure it exists near. It also cannot reset the FILE (see
/// [`reset_at_startup`]); it keeps frames moving into the database, and reports
/// when it is blocked instead of logging a success it did not achieve.
pub(crate) fn spawn_periodic_checkpoint(store: SharedStore) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let store = store.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let store = store.lock();
                match store.checkpoint_wal_passive() {
                    Ok(o) if o.fully_retired() => {}
                    Ok(o) => eprintln!(
                        "wal: checkpoint did NOT retire the log — {} of {} pages moved{} \
                         (a live reader holds it open; watch quipu_wal_bytes)",
                        o.moved_pages,
                        o.wal_pages,
                        if o.blocked { ", reported BUSY" } else { "" }
                    ),
                    Err(e) => eprintln!("wal: checkpoint failed: {e}"),
                }
            })
            .await;
        }
    });
}
