//! The store as bytes — `sqlite3_serialize` / `sqlite3_deserialize`
//! (quipu-2l5, `docs/design/wasm-support.md` §6).
//!
//! The `.db` file stays the interchange format everywhere quipu runs. On
//! native the file IS the store; in a browser there is no file to hand
//! anyone, so export means "the exact bytes a `.db` file would contain" —
//! `Store::serialize_db` — and import means opening a store from those bytes
//! — `Store::open_from_bytes`. Both are plain `SQLite` serialization, so the
//! bytes round-trip through `sqlite3`, the `quipu` CLI, and a browser tab
//! without conversion in any direction.

use rusqlite::Connection;

use super::Store;
use crate::error::Result;

impl Store {
    /// The exact bytes of this store's main database, as a `.db` file image
    /// (`sqlite3_serialize`). Writing them to disk yields a file that opens
    /// in `sqlite3` and in `quipu` unchanged; in a browser they are what a
    /// `Blob` download ships.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if serialization fails (OOM, or a database
    /// name that does not exist).
    pub fn serialize_db(&self) -> Result<Vec<u8>> {
        let data = self.conn.serialize(rusqlite::MAIN_DB)?;
        Ok(data.to_vec())
    }

    /// Open a store from the bytes of a `.db` file (`sqlite3_deserialize`).
    ///
    /// The bytes become an in-memory database and then go through the same
    /// `init` — schema, migrations — as any other open, so a store exported
    /// from an older quipu migrates on import exactly as a file would on
    /// open.
    ///
    /// A WAL-format image is normalized before deserializing: `SQLite` refuses
    /// to deserialize WAL (the in-memory database has no shared memory), and
    /// native quipu stores run WAL, so the bytes of a native `.db` file
    /// would otherwise be un-importable — found the hard way, in the
    /// browser, on the first native→tab round-trip. A **checkpointed** WAL
    /// database differs from a rollback one only in header bytes 18/19 (the
    /// read/write format versions: 2 = WAL, 1 = legacy), so this makes the
    /// same edit `PRAGMA journal_mode=DELETE` would. The caller must hand
    /// over a checkpointed database — the bytes of a `.db` whose `-wal`
    /// sidecar still holds frames would silently lose those frames, on this
    /// path exactly as on a plain file copy that forgets the sidecar.
    ///
    /// # Errors
    /// [`crate::Error::Sqlite`] if the bytes are not a `SQLite` database, or
    /// any `init`/migration error.
    pub fn open_from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        let bytes: std::borrow::Cow<'_, [u8]> =
            if bytes.len() > 19 && (bytes[18] == 2 || bytes[19] == 2) {
                let mut v = bytes.to_vec();
                v[18] = 1;
                v[19] = 1;
                std::borrow::Cow::Owned(v)
            } else {
                std::borrow::Cow::Borrowed(bytes)
            };
        conn.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), false)?;
        Self::init(conn)
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    fn count_facts(store: &Store) -> usize {
        match crate::sparql::query(store, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }").unwrap() {
            crate::sparql::QueryResult::Select { rows, .. } => rows.len(),
            _ => panic!("expected SELECT"),
        }
    }

    fn seeded_store() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        let episode: crate::episode::Episode = serde_json::from_str(
            r#"{
                "name": "ep-1",
                "episode_body": "serialize round-trip seed",
                "source": "test",
                "group_id": "g1",
                "nodes": [
                    {"name": "svc-1", "type": "Service", "description": "a service"},
                    {"name": "dep-1", "type": "Deployment", "description": "a deploy"}
                ],
                "edges": [
                    {"source": "dep-1", "target": "svc-1", "relation": "targets"}
                ]
            }"#,
        )
        .unwrap();
        crate::episode::ingest_episode_outcome(
            &mut store,
            &episode,
            "2026-08-13T00:00:00Z",
            "http://test.example/",
        )
        .unwrap();
        store
    }

    #[test]
    fn serialize_then_open_from_bytes_preserves_facts() {
        let store = seeded_store();
        let want = count_facts(&store);
        assert!(want > 0, "seed must produce facts");

        let bytes = store.serialize_db().unwrap();
        let reopened = Store::open_from_bytes(&bytes).unwrap();
        assert_eq!(count_facts(&reopened), want);
    }

    #[test]
    fn serialized_bytes_are_a_db_file_that_opens_in_quipu_and_sqlite() {
        let store = seeded_store();
        let want = count_facts(&store);
        let bytes = store.serialize_db().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exported.db");
        std::fs::write(&path, &bytes).unwrap();

        // "Opens in sqlite3 unchanged": same engine as the CLI, integrity
        // included.
        let raw = rusqlite::Connection::open(&path).unwrap();
        let ok: String = raw
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ok, "ok");
        drop(raw);

        // Opens as a quipu store, facts intact.
        let reopened = Store::open(path.to_str().unwrap()).unwrap();
        assert_eq!(count_facts(&reopened), want);
    }

    #[test]
    fn open_from_bytes_rejects_garbage() {
        assert!(Store::open_from_bytes(b"not a sqlite database at all").is_err());
    }

    #[test]
    fn the_bytes_of_a_wal_mode_file_import_cleanly() {
        // The native→browser direction: quipu file stores run WAL, and a
        // WAL-format image is un-deserializable without the header
        // normalization in open_from_bytes. This is the exact failure the
        // first browser round-trip hit; keep it pinned natively.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal-store.db");
        {
            let mut store = Store::open(path.to_str().unwrap()).unwrap();
            let mode: String = store
                .conn
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal", "precondition: native file stores run WAL");
            let episode: crate::episode::Episode = serde_json::from_str(
                r#"{
                    "name": "ep-wal", "episode_body": "wal import seed",
                    "source": "test", "group_id": "g1",
                    "nodes": [{"name": "svc-9", "type": "Service", "description": "d"}],
                    "edges": []
                }"#,
            )
            .unwrap();
            crate::episode::ingest_episode_outcome(
                &mut store,
                &episode,
                "2026-08-13T00:00:00Z",
                "http://test.example/",
            )
            .unwrap();
        } // close checkpoints the WAL; the header stays WAL-format

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes[18] == 2 || bytes[19] == 2,
            "precondition: the file image is WAL-format"
        );
        let imported = Store::open_from_bytes(&bytes).unwrap();
        assert!(count_facts(&imported) > 0, "the WAL image's facts imported");
    }
}
