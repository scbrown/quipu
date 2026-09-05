//! Parent-bound SPARQL Update deltas for portable shares.

use std::collections::BTreeSet;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use spargebra::GraphUpdateOperation;

use crate::error::{Error, Result};
use crate::share::{ShareManifest, ShareOptions, sha256, share_payload};
use crate::share_import::ShareImportRequest;

const SCHEMA: &str = "https://github.com/scbrown/quipu/share-delta/v1";

/// Files named by a delta share.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaFiles {
    /// Restricted SPARQL 1.1 Update document.
    pub update: String,
    /// Shapes for the resulting share.
    pub shapes: String,
}

/// A verified delta from one full share to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaManifest {
    /// Delta schema identifier.
    pub schema: String,
    /// Hash of this manifest with the id omitted.
    pub delta_id: String,
    /// Immediate parent share identity.
    pub parent_share: String,
    /// Required parent payload identity.
    pub parent_graph_hash: String,
    /// Manifest of the materialized result.
    pub result: ShareManifest,
    /// Hash of exact `delta.ru` bytes.
    pub delta_hash: String,
    /// Delta payload names.
    pub files: DeltaFiles,
}

fn lines(input: &str) -> BTreeSet<String> {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn update_text(parent: &str, result: &str) -> String {
    let parent = lines(parent);
    let result = lines(result);
    let mut out = String::new();
    let deleted = parent.difference(&result).collect::<Vec<_>>();
    if !deleted.is_empty() {
        out.push_str("DELETE DATA {\n");
        for line in deleted {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("};\n");
    }
    let inserted = result.difference(&parent).collect::<Vec<_>>();
    if !inserted.is_empty() {
        out.push_str("INSERT DATA {\n");
        for line in inserted {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("};\n");
    }
    out
}

fn manifest_bytes(manifest: &DeltaManifest, include_id: bool) -> Result<Vec<u8>> {
    let mut value = serde_json::to_value(manifest)
        .map_err(|e| Error::Serialization(format!("delta manifest: {e}")))?;
    if !include_id {
        value.as_object_mut().expect("object").remove("delta_id");
    }
    let mut bytes = serde_json::to_vec(&value)
        .map_err(|e| Error::Serialization(format!("delta manifest: {e}")))?;
    if include_id {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

/// The delta manifest as RDF — the derived view `write_delta` writes alongside
/// `manifest.json`.
///
/// Public so the wasm page can emit the SAME four files the CLI does
/// (aegis-8fdp8d). `materialize` does not read it; it exists for humans and for
/// graph tooling, and omitting it would make a browser-produced delta share a
/// near-miss of the CLI's rather than the same artifact.
#[must_use]
pub fn manifest_turtle(manifest: &DeltaManifest) -> String {
    let literal = |value: &str| serde_json::to_string(value).expect("string JSON cannot fail");
    format!(
        r#"@prefix dcat: <http://www.w3.org/ns/dcat#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix spdx: <http://spdx.org/rdf/terms#> .
@prefix quipu: <https://quipu.dev/ontology/> .

<urn:{delta_id}> a dcat:Dataset, prov:Entity ;
  dct:identifier {delta_id_literal} ;
  prov:wasRevisionOf <urn:{parent}> ;
  quipu:parentPayloadChecksum {parent_hash} ;
  quipu:resultShare <urn:{result_id}> ;
  quipu:resultPayloadChecksum {result_hash} ;
  dcat:distribution [ a dcat:Distribution ;
    dcat:mediaType "application/sparql-update" ;
    dcat:downloadURL <payload:delta.ru> ;
    spdx:checksum [ a spdx:Checksum ;
      spdx:algorithm spdx:checksumAlgorithm_sha256 ;
      spdx:checksumValue {delta_hash} ] ] .
"#,
        delta_id = manifest.delta_id,
        delta_id_literal = literal(&manifest.delta_id),
        parent = manifest.parent_share,
        parent_hash = literal(&manifest.parent_graph_hash),
        result_id = manifest.result.share_id,
        result_hash = literal(&manifest.result.graph_hash),
        delta_hash = literal(&manifest.delta_hash),
    )
}

/// Write a delta share against a verified local parent directory.
/// A delta computed in memory: the manifest, the update document and the shapes.
///
/// The filesystem-free half of [`write_delta`], extracted (aegis-8fdp8d) so the
/// wasm explorer can produce the SAME `share-delta/v1` artifact the CLI does.
/// The page has no filesystem and no parent directory to read, but it does hold
/// the parent's `export.nt` and manifest from the pack it loaded, which is all
/// this needs. Sharing the function rather than reimplementing the diff is the
/// point: two producers of one format is how the format acquires two meanings.
#[derive(Debug, Clone)]
pub struct DeltaPayload {
    /// Verified delta manifest, `delta_id` already computed.
    pub manifest: DeltaManifest,
    /// `delta.ru` contents — empty when parent and result agree.
    pub update: String,
    /// `shapes.ttl` contents for the resulting share.
    pub shapes: String,
}

impl DeltaPayload {
    /// The four files a delta share directory contains, as (name, contents).
    ///
    /// The same set, with the same names, that [`write_delta`] puts on disk —
    /// so a consumer cannot tell whether a delta came from the CLI or from a
    /// browser tab, which is the point.
    ///
    /// # Errors
    /// The manifest cannot be serialized.
    pub fn files(&self) -> Result<Vec<(String, String)>> {
        Ok(vec![
            (
                "manifest.json".to_string(),
                String::from_utf8(manifest_bytes(&self.manifest, true)?)
                    .map_err(|e| Error::Serialization(format!("delta manifest utf8: {e}")))?,
            ),
            ("manifest.ttl".to_string(), manifest_turtle(&self.manifest)),
            (self.manifest.files.update.clone(), self.update.clone()),
            (self.manifest.files.shapes.clone(), self.shapes.clone()),
        ])
    }
}

/// Build a delta from an in-memory parent, without touching the filesystem.
///
/// The caller supplies the parent's identity and normative graph; verification
/// of the parent against its own manifest is the caller's job, because the two
/// callers verify at different moments — the CLI reads and verifies a directory,
/// while the page verified the pack when it loaded it and has been editing the
/// resulting store ever since.
///
/// # Errors
/// The result share cannot be built, or the generated update is not valid
/// SPARQL — the latter is a guard against emitting a document no receiver could
/// apply, not a formality.
pub fn build_delta(
    store: &crate::Store,
    parent_share_id: &str,
    parent_graph_hash: &str,
    parent_export_ntriples: &str,
    opts: &ShareOptions,
) -> Result<DeltaPayload> {
    build_delta_with_limit(
        store,
        parent_share_id,
        parent_graph_hash,
        parent_export_ntriples,
        opts,
        crate::share::SHARE_PAYLOAD_MAX_BYTES,
    )
}

/// Build a delta with an explicit payload budget for an embedded caller.
///
/// Browser exports can exceed the remote response ceiling. Remote callers
/// should use [`build_delta`], which keeps the fixed server budget.
///
/// # Errors
/// The materialized share exceeds `max_bytes`, cannot be built, or produces
/// an invalid SPARQL update.
pub fn build_delta_with_limit(
    store: &crate::Store,
    parent_share_id: &str,
    parent_graph_hash: &str,
    parent_export_ntriples: &str,
    opts: &ShareOptions,
    max_bytes: usize,
) -> Result<DeltaPayload> {
    let mut opts = opts.clone();
    opts.parent_share = Some(parent_share_id.to_string());
    let result = share_payload(store, &opts, max_bytes)?;
    let update = update_text(parent_export_ntriples, &result.files["export.nt"]);
    if !update.is_empty() {
        spargebra::SparqlParser::new()
            .parse_update(&update)
            .map_err(|e| {
                Error::InvalidValue(format!("generated delta is not SPARQL Update: {e}"))
            })?;
    }
    let shapes = result.files["shapes.ttl"].clone();
    let mut manifest = DeltaManifest {
        schema: SCHEMA.into(),
        delta_id: String::new(),
        parent_share: parent_share_id.to_string(),
        parent_graph_hash: parent_graph_hash.to_string(),
        result: result.manifest,
        delta_hash: sha256(update.as_bytes()),
        files: DeltaFiles {
            update: "delta.ru".into(),
            shapes: "shapes.ttl".into(),
        },
    };
    manifest.delta_id = sha256(&manifest_bytes(&manifest, false)?);
    Ok(DeltaPayload {
        manifest,
        update,
        shapes,
    })
}

/// Write a delta share against a verified local parent directory.
///
/// Filesystem entry point, so not built for wasm — the page uses
/// [`build_delta`] and hands the bytes to the reader instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_delta(
    store: &crate::Store,
    parent_dir: &str,
    out_dir: &str,
    opts: &ShareOptions,
) -> Result<DeltaManifest> {
    let parent = crate::share_transport::read_reference(parent_dir)?;
    crate::share_import::verify_share(&parent)?;
    let built = build_delta(
        store,
        &parent.manifest.share_id,
        &parent.manifest.graph_hash,
        &parent.export_ntriples,
        opts,
    )?;
    let DeltaPayload {
        manifest,
        update,
        shapes,
    } = built;
    let out = Path::new(out_dir);
    if out.exists() {
        return Err(Error::InvalidValue(format!(
            "delta destination exists: {out_dir}"
        )));
    }
    let build = PathBuf::from(format!("{out_dir}.building"));
    std::fs::create_dir_all(&build).map_err(|e| Error::Store(format!("delta create: {e}")))?;
    let written = (|| -> Result<()> {
        std::fs::write(
            build.join("manifest.json"),
            manifest_bytes(&manifest, true)?,
        )
        .map_err(|e| Error::Store(format!("delta manifest write: {e}")))?;
        std::fs::write(build.join("manifest.ttl"), manifest_turtle(&manifest))
            .map_err(|e| Error::Store(format!("delta RDF manifest write: {e}")))?;
        std::fs::write(build.join("delta.ru"), update)
            .map_err(|e| Error::Store(format!("delta update write: {e}")))?;
        std::fs::write(build.join("shapes.ttl"), &shapes)
            .map_err(|e| Error::Store(format!("delta shapes write: {e}")))?;
        Ok(())
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_dir_all(&build);
        return Err(error);
    }
    std::fs::rename(build, out).map_err(|e| Error::Store(format!("delta publish: {e}")))?;
    Ok(manifest)
}

/// Verify and materialize a local delta against its full parent without mutating either input.
///
/// Filesystem entry point, so not built for wasm.
#[cfg(not(target_arch = "wasm32"))]
pub fn materialize(parent_dir: &str, delta_dir: &str) -> Result<ShareImportRequest> {
    let parent = crate::share_transport::read_local(parent_dir)?;
    crate::share_import::verify_share(&parent)?;
    let root = Path::new(delta_dir);
    let bytes = std::fs::read(root.join("manifest.json"))
        .map_err(|e| Error::InvalidValue(format!("delta manifest read: {e}")))?;
    let manifest: DeltaManifest = serde_json::from_slice(&bytes)
        .map_err(|e| Error::InvalidValue(format!("delta manifest parse: {e}")))?;
    if manifest.schema != SCHEMA
        || manifest.parent_share != parent.manifest.share_id
        || manifest.parent_graph_hash != parent.manifest.graph_hash
    {
        return Err(Error::InvalidValue(
            "delta parent precondition failed".into(),
        ));
    }
    if manifest.delta_id != sha256(&manifest_bytes(&manifest, false)?) {
        return Err(Error::InvalidValue(
            "delta manifest identity mismatch".into(),
        ));
    }
    let update = std::fs::read_to_string(root.join(&manifest.files.update))
        .map_err(|e| Error::InvalidValue(format!("delta update read: {e}")))?;
    if manifest.delta_hash != sha256(update.as_bytes()) {
        return Err(Error::InvalidValue("delta update hash mismatch".into()));
    }
    let mut result = lines(&parent.export_ntriples);
    if !update.is_empty() {
        let parsed = spargebra::SparqlParser::new()
            .parse_update(&update)
            .map_err(|e| Error::InvalidValue(format!("delta update parse: {e}")))?;
        for operation in parsed.operations {
            match operation {
                GraphUpdateOperation::DeleteData { data } => {
                    for quad in data {
                        result.remove(&format!("{quad} ."));
                    }
                }
                GraphUpdateOperation::InsertData { data } => {
                    for quad in data {
                        result.insert(format!("{quad} ."));
                    }
                }
                _ => {
                    return Err(Error::InvalidValue(
                        "delta contains unrestricted update".into(),
                    ));
                }
            }
        }
    }
    let mut export = result.into_iter().collect::<Vec<_>>().join("\n");
    if !export.is_empty() {
        export.push('\n');
    }
    let export = crate::share::canonicalize_ntriples(export.as_bytes())?;
    if sha256(&export) != manifest.result.graph_hash {
        return Err(Error::InvalidValue("delta result hash mismatch".into()));
    }
    Ok(ShareImportRequest {
        // Local construction carries no envelope: these paths build a request from
        // bytes already in hand, so there is no producer to attest. The result then
        // reports tier "transport", which is the truth (aegis-c9c44).
        attestation: None,
        manifest: manifest.result,
        export_ntriples: String::from_utf8(export)
            .map_err(|e| Error::Serialization(format!("delta result UTF-8: {e}")))?,
        shapes_turtle: std::fs::read_to_string(root.join(&manifest.files.shapes))
            .map_err(|e| Error::InvalidValue(format!("delta shapes read: {e}")))?,
        source: delta_dir.into(),
        actor: None,
        accept_exact: false,
    })
}

#[cfg(test)]
#[path = "share_delta_tests.rs"]
mod tests;
