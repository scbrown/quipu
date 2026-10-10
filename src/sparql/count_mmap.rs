//! Bound scalar COUNT's process working set without changing pooled-reader policy.
//!
//! A count returns no term pointers. Reading a whole index need not map every
//! touched file page into the process (a 256 MB RSS floor despite no bindings).
//! Use the existing bounded SQLite page cache for this scalar statement, then
//! restore the caller's mmap window on success, error, or unwinding. This is
//! connection-local; no persistent store setting or cache is added.
use crate::{Result, Store};
use rusqlite::OptionalExtension;

struct Mapping<'a> {
    store: &'a Store,
    previous: Option<i64>,
}
impl Mapping<'_> {
    fn restore(&mut self) -> Result<()> {
        if let Some(size) = self.previous {
            self.store.conn.pragma_update(None, "mmap_size", size)?;
            self.previous = None;
        }
        Ok(())
    }
}
impl Drop for Mapping<'_> {
    fn drop(&mut self) {
        if self.restore().is_err() {
            eprintln!("COUNT failed to restore the reader mmap window");
        }
    }
}

pub(super) fn scalar<T>(store: &Store, evaluate: impl FnOnce() -> Result<T>) -> Result<T> {
    // An in-memory SQLite database has no mmap_size result and needs no guard.
    let previous: Option<i64> = store
        .conn
        .query_row("PRAGMA mmap_size", [], |row| row.get(0))
        .optional()?;
    let mut guard = Mapping {
        store,
        previous: previous.filter(|size| *size > 0),
    };
    if guard.previous.is_some() {
        store.conn.pragma_update(None, "mmap_size", 0)?;
    }
    let result = evaluate();
    guard.restore()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapped_reader_policy_is_restored_after_success_and_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        drop(Store::open(path.to_str().unwrap()).unwrap());
        let store = Store::open_read_only(path.to_str().unwrap()).unwrap();
        let size = || {
            store
                .conn
                .query_row("PRAGMA mmap_size", [], |r| r.get::<_, i64>(0))
                .unwrap()
        };
        assert_eq!(size(), 268435456);
        scalar(&store, || {
            assert_eq!(size(), 0);
            Ok(())
        })
        .unwrap();
        assert_eq!(size(), 268435456);
        assert!(
            scalar::<()>(&store, || Err(crate::Error::InvalidValue("control".into()))).is_err()
        );
        assert_eq!(size(), 268435456);
    }
}
