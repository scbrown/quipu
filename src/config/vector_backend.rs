//! Installing the configured vector backend (quipu-lv7).
//!
//! `vector.backend = "lancedb"` was set-but-not-read for as long as the
//! `LanceDB` implementation existed: `Store::set_local_vector_backend` had no
//! non-test caller, so `quipu migrate-vectors` moved embeddings into a store
//! that nothing then selected. The key warned about itself, which kept it
//! honest but left the capability unreachable.
//!
//! This is the reader. The dispatch below it was never the missing part —
//! `Store::vector_store()` already prefers a delegate, then a local backend,
//! then the built-in `SQLite` table, and every consumer goes through it — so
//! selecting the backend at open is all that was needed for search, resolution,
//! auto-embed and the MCP/REST search tools to use it at once.
//!
//! ## Why a hard error rather than a warning when the feature is absent
//!
//! `LanceDB` is deliberately not in the `full` feature the release binaries are
//! built with: it drags in protoc and the whole datafusion tree. So a shipped
//! binary can be asked for a backend it cannot construct. That is not a case
//! for a warning — the operator asking for `lancedb` has usually just migrated
//! their embeddings there, and continuing against the `SQLite` table would
//! answer their searches out of a store they believe they stopped using. It
//! refuses, and names the rebuild.

use crate::config::{QuipuConfig, VectorBackend};
use crate::error::{Error, Result};
use crate::store::Store;

/// Install `config.vector.backend` on `store`.
///
/// `sqlite` (the default) is a no-op: the built-in table is what a store
/// already uses. Returns what was installed, for the caller to announce.
///
/// # Errors
/// - `vector.backend = "lancedb"` on a binary built without the `lancedb`
///   feature: refuses, naming the rebuild. Silently querying the `SQLite`
///   table instead would answer from a store the operator believes they have
///   migrated away from.
/// - A `LanceDB` database that cannot be opened at `vector.lancedb_path`.
/// - No Tokio runtime in context. `LanceDB` is async; the caller must be
///   inside a multi-threaded runtime (`quipu-server` is `#[tokio::main]`; the
///   CLI enters one before dispatch when the backend is selected).
pub fn install_vector_backend(store: &mut Store, config: &QuipuConfig) -> Result<Option<String>> {
    match config.vector.backend {
        VectorBackend::Sqlite => Ok(None),
        VectorBackend::Lancedb => install_lancedb(store, config),
    }
}

#[cfg(feature = "lancedb")]
fn install_lancedb(store: &mut Store, config: &QuipuConfig) -> Result<Option<String>> {
    let path = config.vector.lancedb_path.to_string_lossy().to_string();
    let handle = tokio::runtime::Handle::try_current().map_err(|_| {
        Error::Store(format!(
            "vector.backend = \"lancedb\" needs a Tokio runtime to open {path:?}, and none is \
             in context. This is a wiring bug in the calling binary, not a config error."
        ))
    })?;
    let backend = tokio::task::block_in_place(|| {
        handle.block_on(crate::vector_lance::LanceVectorStore::open_or_create(&path))
    })?;
    store.set_local_vector_backend(Box::new(backend));
    Ok(Some(path))
}

#[cfg(not(feature = "lancedb"))]
fn install_lancedb(_store: &mut Store, config: &QuipuConfig) -> Result<Option<String>> {
    Err(Error::Store(format!(
        "vector.backend = \"lancedb\" is configured (path {:?}), but this binary was built \
         without the `lancedb` feature, so that backend cannot be constructed. Refusing rather \
         than falling back to the SQLite vectors table: if you have run `quipu migrate-vectors`, \
         a silent fallback would answer every search out of the store you migrated away from. \
         Rebuild with `--features lancedb`, or set `[quipu.vector] backend = \"sqlite\"`.",
        config.vector.lancedb_path.display().to_string()
    )))
}

#[cfg(test)]
#[path = "vector_backend_tests.rs"]
mod tests;
