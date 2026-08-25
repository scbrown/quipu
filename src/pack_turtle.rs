//! The `--format turtle` interop bundle — `pack::pack_turtle`'s implementation.
//!
//! Split from `src/pack.rs` for the file-size ratchet; the public path is
//! unchanged (`pack.rs` re-exports it). Export-only and file-writing by
//! nature, so the whole module is native-only.

#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;

use crate::error::{Error, Result};
use crate::pack::{Manifest, PackOptions, canonical_content, content_hash, local_name};
use crate::store::Store;

/// Export as an **interop bundle** — a directory of plain files rather than a
/// store (quipu #81, `--format turtle`).
///
/// `graph.ttl` + `shapes.ttl` + `queries.json` + `manifest.json`. Export-only:
/// there is no unpack path from this form, because its purpose is to be read by
/// something that is not Quipu.
///
/// **The content hash is the SAME as the `.qpack.db` form's**, because both are
/// computed from [`canonical_content`] rather than from the emitted bytes. So a
/// graph packs to one identity in either format, and a consumer can check a
/// bundle against a hash it was given for a store file. Hashing the emitted
/// files instead would have made the format part of the identity — two
/// renderings of the same knowledge with different hashes, which is exactly
/// what a content hash is supposed to rule out.
///
/// # Errors
/// Unknown graph, a named shape or query that does not exist, a non-zero
/// `space` (a bundle has no term ids, so there is no space to allocate from),
/// or any IO error.
pub fn pack_turtle(
    store: &Store,
    graph_iri: &str,
    out_dir: &str,
    opts: &PackOptions,
    timestamp: &str,
) -> Result<Manifest> {
    if store.lookup(graph_iri)?.is_none() {
        return Err(Error::InvalidValue(format!(
            "pack: unknown graph: {graph_iri}"
        )));
    }
    // Refused rather than ignored — an accepted-and-inert flag is the defect
    // class `--with-vectors` shipped with once already.
    if opts.space.unwrap_or(0) != 0 {
        return Err(Error::InvalidValue(
            "pack --space does not apply to --format turtle: a bundle carries \
             IRIs, not term ids, so there is no term space to allocate from."
                .into(),
        ));
    }
    let canonical = canonical_content(store, graph_iri, &opts.shapes, &opts.queries)?;
    let hash = content_hash(&canonical);

    std::fs::create_dir_all(out_dir)
        .map_err(|e| Error::Store(format!("pack: cannot create {out_dir}: {e}")))?;
    let write = |name: &str, bytes: &[u8]| -> Result<()> {
        std::fs::write(Path::new(out_dir).join(name), bytes)
            .map_err(|e| Error::Store(format!("pack: writing {name}: {e}")))
    };

    let (graph_ttl, fact_count) =
        crate::rdf::export_rdf_subset(store, oxrdfio::RdfFormat::Turtle, Some(graph_iri))?;
    write("graph.ttl", &graph_ttl)?;

    let mut shapes_ttl = String::new();
    for name in &opts.shapes {
        let (_, turtle, _) = store
            .list_shapes()?
            .into_iter()
            .find(|(n, _, _)| n == name)
            .ok_or_else(|| Error::InvalidValue(format!("pack: no such shape graph: {name}")))?;
        shapes_ttl.push_str(&format!("# --- {name} ---\n{turtle}\n\n"));
    }
    write("shapes.ttl", shapes_ttl.as_bytes())?;

    let mut queries = Vec::new();
    for name in &opts.queries {
        let q = store
            .query_get(name)?
            .ok_or_else(|| Error::InvalidValue(format!("pack: no such stored query: {name}")))?;
        let mut entry = q.to_catalog_json();
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("template".into(), serde_json::json!(q.template));
        }
        queries.push(entry);
    }
    write(
        "queries.json",
        serde_json::to_string_pretty(&serde_json::json!({ "queries": queries }))
            .map_err(|e| Error::Serialization(e.to_string()))?
            .as_bytes(),
    )?;

    let manifest = Manifest {
        pack_format: "1-turtle".into(),
        name: opts.name.clone().unwrap_or_else(|| local_name(graph_iri)),
        version: opts.version.clone().unwrap_or_else(|| "0.1.0".into()),
        term_space: 0,
        content_hash: hash,
        created_at: timestamp.to_string(),
        source_graph: graph_iri.to_string(),
        producer: serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "tool": "quipu pack --format turtle",
        })
        .to_string(),
        counts: serde_json::json!({
            "facts": fact_count,
            "shapes": opts.shapes.len(),
            "queries": opts.queries.len(),
            "embedding_model": store.embedding_config().model_path.as_ref()
                .and_then(|p| p.file_name()).map(|p| p.to_string_lossy()),
            "embedding_dimension": store.embedding_config().dimension,
        })
        .to_string(),
    };
    write(
        "manifest.json",
        serde_json::to_string_pretty(&serde_json::json!({
            "pack_format": manifest.pack_format,
            "name": manifest.name,
            "version": manifest.version,
            "term_space": manifest.term_space,
            "content_hash": manifest.content_hash,
            "created_at": manifest.created_at,
            "source_graph": manifest.source_graph,
            "producer": serde_json::from_str::<serde_json::Value>(&manifest.producer)
                .unwrap_or(serde_json::Value::Null),
            "counts": serde_json::from_str::<serde_json::Value>(&manifest.counts)
                .unwrap_or(serde_json::Value::Null),
        }))
        .map_err(|e| Error::Serialization(e.to_string()))?
        .as_bytes(),
    )?;

    Ok(manifest)
}
