//! Git-native knowledge shares: canonical RDF plus shapes and a lineage manifest.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::store::Store;

/// The graph slice written into a share.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ShareScope {
    /// ROOT/default graph.
    #[default]
    Root,
    /// One named graph IRI.
    Graph(String),
    /// Facts attributed to one episode provenance group.
    Group(String),
    /// A SPARQL CONSTRUCT or DESCRIBE result.
    Construct(String),
}

impl Serialize for ShareScope {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let (kind, value) = match self {
            Self::Root => ("root", None),
            Self::Graph(value) => ("graph", Some(value.as_str())),
            Self::Group(value) => ("group", Some(value.as_str())),
            Self::Construct(value) => ("construct", Some(value.as_str())),
        };
        let mut state = serializer.serialize_struct("ShareScope", 2)?;
        state.serialize_field("kind", kind)?;
        state.serialize_field("value", &value)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ShareScope {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            kind: String,
            value: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        match (wire.kind.as_str(), wire.value) {
            ("root", None) => Ok(Self::Root),
            ("graph", Some(value)) => Ok(Self::Graph(value)),
            ("group", Some(value)) => Ok(Self::Group(value)),
            ("construct", Some(value)) => Ok(Self::Construct(value)),
            (kind, value) => Err(serde::de::Error::custom(format!(
                "invalid share scope kind={kind:?} value={value:?}"
            ))),
        }
    }
}

/// Options for [`share`].
#[derive(Debug, Clone, Default)]
pub struct ShareOptions {
    /// Scope to export.
    pub scope: ShareScope,
    /// Shape registry entries to concatenate into `shapes.ttl`.
    pub shapes: Vec<String>,
    /// Explicitly produce a shapes-free share.
    pub no_shapes: bool,
    /// Prior share id in this lineage.
    pub parent_share: Option<String>,
    /// Also emit a human-readable `export.ttl` derived view.
    pub turtle_view: bool,
}

/// Default upper bound for a payload-returning share response.
pub const SHARE_PAYLOAD_MAX_BYTES: usize = 8 * 1024 * 1024;

/// HTTP-friendly options for building a share payload in memory.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SharePayloadRequest {
    /// Scope to export.
    #[serde(default)]
    pub scope: ShareScope,
    /// Shape registry entries to concatenate into `shapes.ttl`.
    #[serde(default)]
    pub shapes: Vec<String>,
    /// Explicitly produce a shapes-free share.
    #[serde(default)]
    pub no_shapes: bool,
    /// Prior share id in this lineage.
    pub parent_share: Option<String>,
    /// Also include a human-readable `export.ttl` derived view.
    #[serde(default)]
    pub turtle_view: bool,
    /// Optional lower response limit; callers cannot raise the server cap.
    pub max_bytes: Option<usize>,
}

impl SharePayloadRequest {
    /// Convert the wire request to the canonical producer options.
    pub fn options(&self) -> ShareOptions {
        ShareOptions {
            scope: self.scope.clone(),
            shapes: self.shapes.clone(),
            no_shapes: self.no_shapes,
            parent_share: self.parent_share.clone(),
            turtle_view: self.turtle_view,
        }
    }

    /// Apply the fixed server ceiling to the optional caller limit.
    pub fn effective_max_bytes(&self) -> usize {
        self.max_bytes
            .unwrap_or(SHARE_PAYLOAD_MAX_BYTES)
            .min(SHARE_PAYLOAD_MAX_BYTES)
    }
}

/// A complete share returned to a remote caller without server-local paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharePayload {
    /// Canonical manifest, also present byte-for-byte as `files["manifest.json"]`.
    pub manifest: ShareManifest,
    /// Exact UTF-8 file contents keyed by canonical share filename.
    pub files: BTreeMap<String, String>,
}

/// Producer identity recorded in a share manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareProducer {
    /// Producer name.
    pub name: String,
    /// Producer version.
    pub version: String,
}

/// Fixed payload names in a share directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareFiles {
    /// Normative graph payload.
    pub graph: String,
    /// Shape payload.
    pub shapes: String,
    /// Optional derived Turtle view.
    pub turtle_view: Option<String>,
}

/// Versioned, hash-checked share envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareManifest {
    /// Manifest schema identifier.
    pub schema: String,
    /// Hash of this manifest with `share_id` omitted.
    pub share_id: String,
    /// Stable producer-store identity.
    pub store_id: String,
    /// Latest transaction included in the snapshot.
    pub tx_anchor: i64,
    /// Hash of exact `export.nt` bytes.
    pub graph_hash: String,
    /// Hash of exact `shapes.ttl` bytes.
    pub shapes_hash: String,
    /// Export scope.
    pub scope: ShareScope,
    /// Prior share in this lineage.
    pub parent_share: Option<String>,
    /// Timestamp of the anchored transaction, stable for unchanged state.
    pub created_at: String,
    /// Producer identity.
    pub producer: ShareProducer,
    /// Payload names.
    pub files: ShareFiles,
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    format!("sha256:{}", hex::encode(digest.as_ref()))
}

pub(crate) fn manifest_bytes(manifest: &ShareManifest, include_id: bool) -> Result<Vec<u8>> {
    let mut value = serde_json::to_value(manifest)
        .map_err(|e| Error::Serialization(format!("share manifest: {e}")))?;
    if !include_id {
        value
            .as_object_mut()
            .expect("serialized manifest is an object")
            .remove("share_id");
    }
    let mut bytes = serde_json::to_vec(&value)
        .map_err(|e| Error::Serialization(format!("share manifest: {e}")))?;
    if include_id {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn export_scope(store: &Store, scope: &ShareScope, format: oxrdfio::RdfFormat) -> Result<Vec<u8>> {
    match scope {
        ShareScope::Root => crate::rdf::export_rdf_subset(store, format, None).map(|v| v.0),
        ShareScope::Graph(iri) => {
            crate::rdf::export_rdf_subset(store, format, Some(iri)).map(|v| v.0)
        }
        ShareScope::Group(group) => crate::export_rdf_group(store, format, group).map(|v| v.0),
        ShareScope::Construct(query) => {
            crate::export_rdf_construct(store, format, query).map(|v| v.0)
        }
    }
}

fn shapes_bytes(store: &Store, names: &[String], no_shapes: bool) -> Result<Vec<u8>> {
    let available = store.list_shapes()?;
    if no_shapes {
        return Ok(Vec::new());
    }
    if available.is_empty() {
        return Err(Error::InvalidValue(
            "share: no loaded shape sets; load shapes first or explicitly pass --no-shapes".into(),
        ));
    }
    let requested: BTreeSet<&str> = if names.is_empty() {
        available.iter().map(|(name, _, _)| name.as_str()).collect()
    } else {
        names.iter().map(String::as_str).collect()
    };
    let mut out = String::new();
    for name in requested {
        let (_, turtle, _) = available
            .iter()
            .find(|(candidate, _, _)| candidate == name)
            .ok_or_else(|| Error::InvalidValue(format!("share: no such shape set: {name}")))?;
        out.push_str(&format!("# --- {name} ---\n{}\n", turtle.trim_end()));
    }
    Ok(out.into_bytes())
}

fn outward_scrub_patterns(store: &Store) -> Result<Vec<(String, regex::Regex)>> {
    const QUERY: &str = "PREFIX aegis: <http://aegis.gastown.local/ontology/> \
        PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
        SELECT ?label ?regex WHERE { \
          ?rule a aegis:InternalIdentifierPattern ; \
                rdfs:label ?label ; \
                aegis:regex ?regex ; \
                aegis:enforcementTier \"block\" . \
        } ORDER BY ?label ?regex";
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(store, QUERY)?
    else {
        return Err(Error::Store(
            "share scrub: InternalIdentifierPattern query did not return rows".into(),
        ));
    };
    rows.into_iter()
        .map(|row| {
            let label = match row.get("label") {
                Some(crate::types::Value::Str(value)) => value.clone(),
                _ => {
                    return Err(Error::Store(
                        "share scrub: pattern has no string label".into(),
                    ));
                }
            };
            let Some(crate::types::Value::Str(source)) = row.get("regex") else {
                return Err(Error::Store(format!(
                    "share scrub: pattern {label:?} has no string regex"
                )));
            };
            let compiled = regex::Regex::new(source).map_err(|error| {
                Error::InvalidValue(format!(
                    "share scrub: pattern {label:?} has invalid regex: {error}"
                ))
            })?;
            Ok((label, compiled))
        })
        .collect()
}

fn scrub_outward_payload(store: &Store, files: &BTreeMap<String, String>) -> Result<()> {
    for (label, pattern) in outward_scrub_patterns(store)? {
        for (name, contents) in files {
            if name == "manifest.json" {
                continue;
            }
            if let Some(hit) = pattern.find(contents) {
                return Err(Error::PolicyDenied(format!(
                    "share scrub refused {name}: InternalIdentifierPattern {label:?} matched bytes {}..{}; \
                     identifiers are entity identity and are never rewritten at this boundary",
                    hit.start(),
                    hit.end()
                )));
            }
        }
    }
    Ok(())
}

fn build_share_payload(store: &Store, opts: &ShareOptions) -> Result<SharePayload> {
    let graph = export_scope(store, &opts.scope, oxrdfio::RdfFormat::NTriples)?;
    let shapes = shapes_bytes(store, &opts.shapes, opts.no_shapes)?;
    let turtle = if opts.turtle_view {
        Some(export_scope(
            store,
            &opts.scope,
            oxrdfio::RdfFormat::Turtle,
        )?)
    } else {
        None
    };

    let tx_anchor = store.latest_tx_id()?;
    let created_at = if tx_anchor == 0 {
        "1970-01-01T00:00:00Z".to_string()
    } else {
        store
            .get_transaction(tx_anchor)?
            .ok_or_else(|| Error::Store(format!("share: missing anchor tx {tx_anchor}")))?
            .timestamp
    };
    let mut manifest = ShareManifest {
        schema: "https://github.com/scbrown/quipu/share-manifest/v1".into(),
        share_id: String::new(),
        store_id: store.store_id()?,
        tx_anchor,
        graph_hash: sha256(&graph),
        shapes_hash: sha256(&shapes),
        scope: opts.scope.clone(),
        parent_share: opts.parent_share.clone(),
        created_at,
        producer: ShareProducer {
            name: "quipu".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        files: ShareFiles {
            graph: "export.nt".into(),
            shapes: "shapes.ttl".into(),
            turtle_view: turtle.as_ref().map(|_| "export.ttl".to_string()),
        },
    };
    manifest.share_id = sha256(&manifest_bytes(&manifest, false)?);

    let mut files = BTreeMap::new();
    files.insert(
        "manifest.json".into(),
        String::from_utf8(manifest_bytes(&manifest, true)?)
            .map_err(|e| Error::Serialization(format!("share manifest UTF-8: {e}")))?,
    );
    files.insert(
        "export.nt".into(),
        String::from_utf8(graph)
            .map_err(|e| Error::Serialization(format!("share graph UTF-8: {e}")))?,
    );
    files.insert(
        "shapes.ttl".into(),
        String::from_utf8(shapes)
            .map_err(|e| Error::Serialization(format!("share shapes UTF-8: {e}")))?,
    );
    if let Some(turtle) = turtle {
        files.insert(
            "export.ttl".into(),
            String::from_utf8(turtle)
                .map_err(|e| Error::Serialization(format!("share Turtle UTF-8: {e}")))?,
        );
    }
    scrub_outward_payload(store, &files)?;
    Ok(SharePayload { manifest, files })
}

/// Build a complete share response with the same canonical producer as [`share`].
pub fn share_payload(store: &Store, opts: &ShareOptions, max_bytes: usize) -> Result<SharePayload> {
    let payload = build_share_payload(store, opts)?;
    let encoded_len = serde_json::to_vec(&payload)
        .map_err(|e| Error::Serialization(format!("share response: {e}")))?
        .len();
    if encoded_len > max_bytes {
        return Err(Error::InvalidValue(format!(
            "share: response is {encoded_len} bytes, exceeding max_bytes {max_bytes}"
        )));
    }
    Ok(payload)
}

/// Write a deterministic share directory.
///
/// The timestamp is derived from `tx_anchor`, rather than wall time, so two
/// exports of unchanged state with the same options are byte-identical.
///
/// # Errors
/// The scope is invalid, a requested shape does not exist, the destination
/// exists, or a payload cannot be written.
pub fn share(store: &Store, out_dir: &str, opts: &ShareOptions) -> Result<ShareManifest> {
    let out = Path::new(out_dir);
    if out.exists() {
        return Err(Error::InvalidValue(format!(
            "share: destination already exists: {out_dir}"
        )));
    }
    let build = PathBuf::from(format!("{out_dir}.building"));
    if build.exists() {
        std::fs::remove_dir_all(&build)
            .map_err(|e| Error::Store(format!("share: clearing stale build: {e}")))?;
    }
    std::fs::create_dir_all(&build)
        .map_err(|e| Error::Store(format!("share: create build directory: {e}")))?;

    let built = (|| -> Result<ShareManifest> {
        let payload = build_share_payload(store, opts)?;
        for (name, contents) in &payload.files {
            std::fs::write(build.join(name), contents.as_bytes())
                .map_err(|e| Error::Store(format!("share: write {name}: {e}")))?;
        }
        Ok(payload.manifest)
    })();

    match built {
        Ok(manifest) => {
            std::fs::rename(&build, out)
                .map_err(|e| Error::Store(format!("share: publish directory: {e}")))?;
            Ok(manifest)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&build);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests;
