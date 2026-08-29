//! Git-native knowledge shares: canonical RDF plus shapes and a lineage manifest.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
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
    /// Prior share id in this lineage.
    pub parent_share: Option<String>,
    /// Also emit a human-readable `export.ttl` derived view.
    pub turtle_view: bool,
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

fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    format!("sha256:{}", hex::encode(digest.as_ref()))
}

fn manifest_bytes(manifest: &ShareManifest, include_id: bool) -> Result<Vec<u8>> {
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
        ShareScope::Group(group) => crate::rdf::export_rdf_group(store, format, group).map(|v| v.0),
        ShareScope::Construct(query) => {
            crate::rdf::export_rdf_construct(store, format, query).map(|v| v.0)
        }
    }
}

fn shapes_bytes(store: &Store, names: &[String]) -> Result<Vec<u8>> {
    let requested: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let available = store.list_shapes()?;
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
        let graph = export_scope(store, &opts.scope, oxrdfio::RdfFormat::NTriples)?;
        let shapes = shapes_bytes(store, &opts.shapes)?;
        std::fs::write(build.join("export.nt"), &graph)
            .map_err(|e| Error::Store(format!("share: write export.nt: {e}")))?;
        std::fs::write(build.join("shapes.ttl"), &shapes)
            .map_err(|e| Error::Store(format!("share: write shapes.ttl: {e}")))?;

        let turtle_view = if opts.turtle_view {
            let bytes = export_scope(store, &opts.scope, oxrdfio::RdfFormat::Turtle)?;
            std::fs::write(build.join("export.ttl"), bytes)
                .map_err(|e| Error::Store(format!("share: write export.ttl: {e}")))?;
            Some("export.ttl".to_string())
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
                turtle_view,
            },
        };
        manifest.share_id = sha256(&manifest_bytes(&manifest, false)?);
        std::fs::write(
            build.join("manifest.json"),
            manifest_bytes(&manifest, true)?,
        )
        .map_err(|e| Error::Store(format!("share: write manifest.json: {e}")))?;
        Ok(manifest)
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
mod tests {
    use super::*;

    fn fixture() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        crate::rdf::ingest_rdf(
            &mut store,
            &b"<urn:z> <urn:p> \"last\" .\n<urn:a> <urn:p> \"first\" .\n"[..],
            oxrdfio::RdfFormat::NTriples,
            None,
            "2026-08-29T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        store
    }

    #[test]
    fn unchanged_state_produces_byte_identical_share_payloads() {
        let store = fixture();
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        let opts = ShareOptions {
            turtle_view: true,
            ..Default::default()
        };
        let ma = share(&store, a.to_str().unwrap(), &opts).unwrap();
        let mb = share(&store, b.to_str().unwrap(), &opts).unwrap();
        assert_eq!(ma, mb);
        for file in ["manifest.json", "export.nt", "shapes.ttl", "export.ttl"] {
            assert_eq!(
                std::fs::read(a.join(file)).unwrap(),
                std::fs::read(b.join(file)).unwrap()
            );
        }
    }

    #[test]
    fn manifest_hashes_match_exact_payload_bytes() {
        let store = fixture();
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("share");
        let manifest = share(&store, out.to_str().unwrap(), &ShareOptions::default()).unwrap();
        assert_eq!(
            manifest.graph_hash,
            sha256(&std::fs::read(out.join("export.nt")).unwrap())
        );
        assert_eq!(
            manifest.shapes_hash,
            sha256(&std::fs::read(out.join("shapes.ttl")).unwrap())
        );
        let stored: ShareManifest =
            serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(
            stored.share_id,
            sha256(&manifest_bytes(&stored, false).unwrap())
        );
        assert_eq!(stored.scope, ShareScope::Root);
    }

    #[test]
    fn parent_share_changes_envelope_identity_not_graph_identity() {
        let store = fixture();
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        let first = share(&store, a.to_str().unwrap(), &ShareOptions::default()).unwrap();
        let second = share(
            &store,
            b.to_str().unwrap(),
            &ShareOptions {
                parent_share: Some(first.share_id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first.graph_hash, second.graph_hash);
        assert_ne!(first.share_id, second.share_id);
        assert_eq!(
            second.parent_share.as_deref(),
            Some(first.share_id.as_str())
        );
    }

    #[test]
    fn shape_selection_is_sorted_and_missing_names_fail_without_output() {
        let store = fixture();
        store
            .load_shapes(
                "z-shape",
                "@prefix sh: <http://www.w3.org/ns/shacl#> .",
                "2026-08-29",
            )
            .unwrap();
        store
            .load_shapes(
                "a-shape",
                "@prefix sh: <http://www.w3.org/ns/shacl#> .",
                "2026-08-29",
            )
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("sorted");
        share(
            &store,
            out.to_str().unwrap(),
            &ShareOptions {
                shapes: vec!["z-shape".into(), "a-shape".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(out.join("shapes.ttl")).unwrap();
        assert!(text.find("a-shape").unwrap() < text.find("z-shape").unwrap());

        let missing = root.path().join("missing");
        assert!(
            share(
                &store,
                missing.to_str().unwrap(),
                &ShareOptions {
                    shapes: vec!["does-not-exist".into()],
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(
            !missing.exists(),
            "a refused share left a partial directory"
        );
    }

    #[test]
    fn store_identity_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("store.db");
        let first = Store::open(db.to_str().unwrap())
            .unwrap()
            .store_id()
            .unwrap();
        let second = Store::open(db.to_str().unwrap())
            .unwrap()
            .store_id()
            .unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with("urn:uuid:"));
    }
}
