//! The wasm bundle the book's "Explore this repository's graph" page runs
//! (aegis-tpqccc).
//!
//! It is a thin shell on purpose. Loading a pack goes through the SAME
//! `share_transport` -> `share_import` -> `promote_import` path the `quipu
//! import` CLI takes, and every read goes through `quipu::tool_query`, the
//! same entry point the REST API and the MCP server use. A reader watching the
//! page is therefore watching the real import and the real query engine, not a
//! browser-shaped reimplementation of them — which is the whole point of
//! putting it in front of them.
//!
//! Everything the page displays derives from a SPARQL query it could have
//! typed itself, so there is no privileged view here that a reader cannot
//! reproduce in the query box.

use quipu::share_import::{PromoteImportRequest, promote_import};
use quipu::share_transport::read_archive_bytes;
use wasm_bindgen::prelude::*;

fn err_js(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// A loaded repository pack: an in-memory store plus the provenance a reader
/// needs to check that what they are exploring is the artifact they think.
#[wasm_bindgen]
pub struct Explorer {
    store: quipu::Store,
    load_report: String,
}

#[wasm_bindgen]
impl Explorer {
    /// Load a `.qpack.tar.gz` into a fresh in-memory store.
    ///
    /// Runs the full receiving ceremony rather than shortcutting to an RDF
    /// parse, because the ceremony is the thing worth showing:
    ///
    /// 1. verify the manifest against the exact payload bytes (RDFC-1.0
    ///    canonical hash),
    /// 2. ADOPT the bundled shapes explicitly — bundled shapes are evidence,
    ///    never authority, so without this step the pack's own vocabulary is
    ///    off-vocabulary here and the import quarantines itself,
    /// 3. import into a staging graph, then
    /// 4. promote that staging graph into ROOT.
    ///
    /// The returned JSON carries the manifest, the import result and the
    /// promotion result, so the page can show the artifact's identity and what
    /// the receiver decided about it.
    ///
    /// # Errors
    /// The archive is malformed or oversized, the manifest does not match its
    /// payload, or the import is refused.
    #[wasm_bindgen(js_name = loadQpack)]
    pub fn load_qpack(bytes: &[u8], source: &str, timestamp: &str) -> Result<Explorer, JsValue> {
        let mut request = read_archive_bytes(bytes, source, true).map_err(err_js)?;
        request.actor = None;
        let mut store = quipu::Store::open_in_memory().map_err(err_js)?;

        // Step 2. Adopting the pack's shapes is a DECISION the receiver makes,
        // and skipping it is not a silent no-op: `import_share` would find the
        // pack's classes ungoverned and quarantine every triple. The page shows
        // this as its own line for that reason.
        store
            .load_shapes("repository-share", &request.shapes_turtle, timestamp)
            .map_err(err_js)?;

        let import = quipu::share_import::import_share(&mut store, &request, timestamp, None)
            .map_err(err_js)?;

        let promotion = if import.promotion.eligible {
            Some(
                promote_import(
                    &mut store,
                    &PromoteImportRequest {
                        share_id: import.share_id.clone(),
                        actor: None,
                    },
                    timestamp,
                    None,
                )
                .map_err(err_js)?,
            )
        } else {
            None
        };

        let load_report = serde_json::to_string(&serde_json::json!({
            "manifest": request.manifest,
            "import": import,
            "promotion": promotion,
            "shapes_bytes": request.shapes_turtle.len(),
            "export_bytes": request.export_ntriples.len(),
            // Named rather than inferred: this build has no SHACL engine in it,
            // so `import.validation.conforms` is a default and not a finding.
            "shacl_compiled": cfg!(feature = "shacl"),
        }))
        .map_err(err_js)?;

        Ok(Explorer { store, load_report })
    }

    /// Manifest, import decision and promotion for the loaded pack, as JSON.
    #[wasm_bindgen(js_name = loadReport)]
    #[must_use]
    pub fn load_report(&self) -> String {
        self.load_report.clone()
    }

    /// Run one SPARQL query through `quipu::tool_query` and return its JSON.
    ///
    /// # Errors
    /// The query is invalid or the engine refuses it.
    pub fn query(&self, sparql: &str) -> Result<String, JsValue> {
        let out = quipu::tool_query(&self.store, &serde_json::json!({ "query": sparql }))
            .map_err(err_js)?;
        serde_json::to_string(&out).map_err(err_js)
    }

    /// Store-level counts (`quipu stats`), as JSON.
    ///
    /// # Errors
    /// The store cannot be read.
    pub fn stats(&self) -> Result<String, JsValue> {
        let out = quipu::tool_report(&self.store, &serde_json::json!({ "kind": "stats" }))
            .map_err(err_js)?;
        serde_json::to_string(&out).map_err(err_js)
    }
}

/// The `quipu` version and commit this bundle was built from, as JSON.
///
/// Deliberately `quipu::VERSION` and NOT this crate's own `CARGO_PKG_VERSION`:
/// the explorer crate is unversioned (`0.0.0`, `publish = false`), so reporting
/// its own version would put a meaningless number on the page directly beside
/// the pack's real `producer.version` and invite the reader to compare them.
/// What a reader wants to know is whether the ENGINE reading the artifact is
/// the engine that wrote it, and that is quipu's version.
#[wasm_bindgen(js_name = explorerVersion)]
#[must_use]
pub fn explorer_version() -> String {
    serde_json::json!({ "version": quipu::VERSION, "git_sha": quipu::GIT_SHA }).to_string()
}
