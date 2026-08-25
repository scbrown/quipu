//! The one store-open path for the `quipu` binary (quipu-at2).
//!
//! Every subcommand used to call `Store::open` directly, which is why
//! `[[quipu.attachments]]` had nowhere to land: there were two dozen opens and
//! no single place a declared layer could be mounted for all of them. Routing
//! them through here means a declared attachment composes for `read`, `cord`,
//! `impact` and the rest alike, rather than for whichever command someone
//! remembered to wire.
//!
//! The config is read once per process and cached. `--db` is NOT re-read here:
//! `main` has already resolved it (flag over config file) and hands the
//! resulting path down, so this module only supplies the attachment half.

use std::sync::OnceLock;

use quipu::{QuipuConfig, Store};

/// The process's config, loaded on first open.
pub fn config() -> &'static QuipuConfig {
    static CONFIG: OnceLock<QuipuConfig> = OnceLock::new();
    CONFIG.get_or_init(|| QuipuConfig::load(std::path::Path::new(".")))
}

/// Open the store at `db_path` with the configured attachments mounted, or
/// exit naming the failure.
///
/// Exits rather than returning a `Result` because every call site did exactly
/// that already, in twenty-four slightly different spellings. A refusal here —
/// a declared file that is missing, an alias that collides, a layer whose
/// schema cannot be composed — is a startup error, not a query that returns
/// fewer rows.
pub fn open_store(db_path: &str) -> Store {
    let mut store =
        quipu::open_with_configured_attachments(db_path, config()).unwrap_or_else(|e| {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        });
    // quipu-lv7: `vector.backend` selects the backend for THIS store, so it is
    // installed per open rather than once per process. `main` has already
    // entered a Tokio runtime when the configured backend needs one.
    if let Err(e) = quipu::config::install_vector_backend(&mut store, config()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    store
}
