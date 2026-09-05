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

use quipu::share::{ShareOptions, ShareScope};
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
    /// The share this store was built from. Carried so an exported pack can
    /// declare it as `parent_share` — an edited pack that does not say what it
    /// descends from is a fork pretending to be an original.
    parent_share: String,
    /// The parent's `graph_hash`, kept so a delta can name the payload it was
    /// computed against without re-deriving it.
    parent_graph_hash: String,
    /// The parent pack's `export.nt`, retained at load (aegis-8fdp8d).
    ///
    /// A delta is a diff against THIS, not against whatever the store would
    /// export today, so it has to be the bytes that arrived. Holding it costs
    /// the pack's graph size a second time and that is the honest price of
    /// being able to say what changed.
    parent_export_ntriples: String,
    /// Every write made in this tab, newest last. Kept in Rust rather than in
    /// the page so the count cannot drift from what the store actually did:
    /// each entry records the tx the store reported, not the request the UI
    /// sent.
    edits: Vec<serde_json::Value>,
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

        Ok(Explorer {
            store,
            load_report,
            parent_share: request.manifest.share_id.clone(),
            parent_graph_hash: request.manifest.graph_hash.clone(),
            parent_export_ntriples: request.export_ntriples.clone(),
            edits: Vec::new(),
        })
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

    /// Replace every current value of one predicate on one entity
    /// (`quipu::tool_set`, the `/set` semantics).
    ///
    /// `value` is the JSON the REST tool takes, so `{"str": "..."}` states a
    /// literal and a bare string goes through the same IRI-shape heuristic it
    /// does. Single-valued by definition: this RETRACTS what was there. To add
    /// without removing, use [`Explorer::episode`].
    ///
    /// # Errors
    /// The entity does not exist (a `/set` on a typo'd IRI must not mint an
    /// orphan), the value JSON is invalid, or the write is refused.
    pub fn set(&mut self, entity: &str, predicate: &str, value: &str) -> Result<String, JsValue> {
        let value: serde_json::Value = serde_json::from_str(value).map_err(err_js)?;
        let out = quipu::tool_set(
            &mut self.store,
            &serde_json::json!({
                "entity": entity, "predicate": predicate, "value": value, "actor": ACTOR,
            }),
        )
        .map_err(err_js)?;
        self.record("set", &out);
        serde_json::to_string(&out).map_err(err_js)
    }

    /// Retract facts on one entity (`quipu::tool_retract`).
    ///
    /// Pass `predicate` empty to retract everything on the entity; pass both
    /// `predicate` and `value` to retract exactly one statement. Retraction is
    /// LOGICAL — the fact is closed, not deleted, so a time-travel query still
    /// finds it. That is why the export below is a new share rather than an
    /// edited copy of the old one.
    ///
    /// # Errors
    /// The entity or predicate does not exist, or the write is refused.
    pub fn retract(
        &mut self,
        entity: &str,
        predicate: &str,
        value: &str,
    ) -> Result<String, JsValue> {
        let mut input = serde_json::json!({ "entity": entity, "actor": ACTOR });
        if !predicate.is_empty() {
            input["predicate"] = serde_json::Value::String(predicate.to_string());
        }
        if !value.is_empty() {
            input["value"] = serde_json::from_str(value).map_err(err_js)?;
        }
        let out = quipu::tool_retract(&mut self.store, &input).map_err(err_js)?;
        self.record("retract", &out);
        serde_json::to_string(&out).map_err(err_js)
    }

    /// Ingest an episode (`quipu::tool_episode`) — the add-a-node path.
    ///
    /// The closed-vocabulary gate applies, and that is worth seeing rather than
    /// working around: a node whose `type` no shape in this store sanctions is
    /// REFUSED and nothing is written. The store's vocabulary here is exactly
    /// the one the pack's own shapes brought with it, so what you may add is
    /// bounded by what the sender declared.
    ///
    /// # Errors
    /// The episode JSON is invalid, a node's type is ungoverned, or the write
    /// is refused.
    pub fn episode(&mut self, episode_json: &str) -> Result<String, JsValue> {
        let input: serde_json::Value = serde_json::from_str(episode_json).map_err(err_js)?;
        let out = quipu::tool_episode(&mut self.store, &input).map_err(err_js)?;
        self.record("episode", &out);
        serde_json::to_string(&out).map_err(err_js)
    }

    /// Every write made in this tab, as JSON — `[{op, tx_id, detail}, ...]`.
    #[wasm_bindgen(js_name = editLog)]
    #[must_use]
    pub fn edit_log(&self) -> String {
        serde_json::to_string(&self.edits).unwrap_or_else(|_| "[]".into())
    }

    /// The current ROOT graph as N-Triples — `export.nt` on its own.
    ///
    /// Line-oriented and canonically ordered, so `diff` against the pack you
    /// started from shows exactly what this tab changed. That is the reviewable
    /// artifact; the pack below is the shippable one.
    ///
    /// # Errors
    /// The store cannot be read or the payload exceeds the bound.
    #[wasm_bindgen(js_name = exportNtriples)]
    pub fn export_ntriples(&self) -> Result<String, JsValue> {
        let payload = self.payload()?;
        payload
            .files
            .get("export.nt")
            .cloned()
            .ok_or_else(|| JsValue::from_str("share payload has no export.nt"))
    }

    /// The edited store as `.qpack.tar.gz` bytes — a real, importable share.
    ///
    /// Built with the same `share_payload` the CLI and the REST endpoint use,
    /// then tarred and gzipped exactly as the release artifact is, so what
    /// comes out of the tab is not a browser-specific format: `quipu import` it,
    /// or drop it back into this page.
    ///
    /// It declares the pack it was derived from as `parent_share`, so the
    /// lineage survives the round trip.
    ///
    /// # Errors
    /// The store cannot be read, the payload exceeds the bound, or the archive
    /// cannot be written.
    #[wasm_bindgen(js_name = exportPack)]
    pub fn export_pack(&self) -> Result<Vec<u8>, JsValue> {
        use std::io::Write;
        let payload = self.payload()?;
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        // Sorted, because `payload.files` is a BTreeMap and a share archive
        // should be byte-stable for the same store state — the producer side of
        // the determinism `share()` gives on disk.
        for (name, contents) in &payload.files {
            let bytes = contents.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_path(name).map_err(err_js)?;
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_cksum();
            archive.append(&header, bytes).map_err(err_js)?;
        }
        let mut encoder = archive.into_inner().map_err(err_js)?;
        encoder.flush().map_err(err_js)?;
        encoder.finish().map_err(err_js)
    }

    /// The manifest the next [`Explorer::export_pack`] would carry, as JSON —
    /// so the page can show the new share id and graph hash BEFORE downloading
    /// several megabytes.
    ///
    /// # Errors
    /// The store cannot be read or the payload exceeds the bound.
    #[wasm_bindgen(js_name = exportManifest)]
    pub fn export_manifest(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.payload()?.manifest).map_err(err_js)
    }

    /// The `share-delta/v1` delta from the loaded pack to this tab's state.
    ///
    /// Returns JSON carrying the delta manifest, the `delta.ru` document, the
    /// repository directory it belongs in, and the sizes the page needs in
    /// order to choose a PR flow:
    ///
    /// ```json
    /// { "manifest": {...}, "update": "DELETE DATA {...}; INSERT DATA {...};",
    ///   "pack_dir": "qpack", "path": "qpack/deltas/<delta_id>.ru",
    ///   "empty": false, "update_bytes": 812 }
    /// ```
    ///
    /// REUSES `quipu::share_delta::build_delta` rather than diffing here
    /// (aegis-8fdp8d, ruled by sattler). quipu already had exactly one delta
    /// format — `share-delta/v1`, one file, `DELETE DATA` before `INSERT DATA`,
    /// with `delta_hash` over the whole document so the destructive half is
    /// inside the integrity envelope. A second format authored in the page
    /// would have been a fork of an artifact the CLI already reads and writes.
    ///
    /// `pack_dir` comes from the LOADED manifest, never from a constant in this
    /// bundle: the page receives its graph as a release asset and has no view of
    /// the repository, so a directory compiled in here would send a renamed
    /// repo's readers to a path that does not exist.
    ///
    /// # Errors
    /// The result share cannot be built, or the generated update is not valid
    /// SPARQL.
    pub fn delta(&self) -> Result<String, JsValue> {
        let built = quipu::share_delta::build_delta(
            &self.store,
            &self.parent_share,
            &self.parent_graph_hash,
            &self.parent_export_ntriples,
            &ShareOptions {
                scope: ShareScope::Root,
                ..ShareOptions::default()
            },
        )
        .map_err(err_js)?;
        let pack_dir = self.pack_dir();
        let dir = format!("{pack_dir}/deltas/{}", built.manifest.delta_id);
        // THE WHOLE ARTIFACT, not delta.ru alone. `materialize` verifies the
        // manifest, then the update against `delta_hash`, then reads the shapes
        // — so a lone delta.ru is a quarter of a delta share and nothing can
        // check its lineage. `files()` is the CLI's exact set, names and bytes.
        let files: Vec<serde_json::Value> = built
            .files()
            .map_err(err_js)?
            .into_iter()
            .map(|(name, contents)| {
                serde_json::json!({
                    "name": name,
                    "path": format!("{dir}/{name}"),
                    "bytes": contents.len(),
                    "contents": contents,
                })
            })
            .collect();
        let total: usize = files
            .iter()
            .map(|f| f["bytes"].as_u64().unwrap_or(0) as usize)
            .sum();
        serde_json::to_string(&serde_json::json!({
            "manifest": built.manifest,
            "update": built.update,
            "pack_dir": pack_dir,
            "dir": dir,
            "files": files,
            "total_bytes": total,
            // An empty update is the honest answer to "propose a PR" when
            // nothing was edited, and the page must say so rather than opening
            // GitHub with a blank file.
            "empty": built.update.is_empty(),
            "update_bytes": built.update.len(),
        }))
        .map_err(err_js)
    }

    /// The repository directory the loaded pack declares, or the default.
    fn pack_dir(&self) -> String {
        serde_json::from_str::<serde_json::Value>(&self.load_report)
            .ok()
            .and_then(|r| {
                r.get("manifest")?
                    .get("pack_dir")?
                    .as_str()
                    .map(str::to_string)
            })
            .map(|d| d.trim().trim_end_matches('/').to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| quipu::share::DEFAULT_PACK_DIR.to_string())
    }

    fn payload(&self) -> Result<quipu::share::SharePayload, JsValue> {
        quipu::share::share_payload(
            &self.store,
            &ShareOptions {
                scope: ShareScope::Root,
                parent_share: Some(self.parent_share.clone()),
                ..ShareOptions::default()
            },
            // The repository pack alone is ~12 MB of N-Triples, well past the
            // 8 MB default the HTTP endpoint uses — that bound exists to keep a
            // server from serialising an unbounded response into a socket, and
            // neither half of that applies to a local export in a tab.
            MAX_EXPORT_BYTES,
        )
        .map_err(err_js)
    }

    fn record(&mut self, op: &str, outcome: &serde_json::Value) {
        self.edits
            .push(serde_json::json!({ "op": op, "outcome": outcome }));
    }
}

/// Attributed to the page rather than left blank: a fact written here is not
/// the producer's, and provenance should say so before anyone exports it.
const ACTOR: &str = "browser-explorer";

/// Upper bound for an in-tab export. Generous on purpose — see
/// `Explorer::payload`.
const MAX_EXPORT_BYTES: usize = 128 * 1024 * 1024;

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
