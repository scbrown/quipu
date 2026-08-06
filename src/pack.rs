//! Knowledge packs — export a graph as a single attachable artifact (quipu #81).
//!
//! Design: `docs/design/knowledge-packs.md` §1. A pack is an ordinary Quipu
//! store file carrying one graph's facts plus the shapes, stored queries and
//! labels that make it usable, and a `pack_manifest` row describing itself.
//!
//! ## Why re-interning, rather than copying rows
//!
//! Term ids are per-database rowids, and they appear in `e`, `a`, `g` **and**
//! inside the opaque `Value::Ref` BLOB in `v`. Copying rows would carry the
//! producer's id assignment into a file that will be opened somewhere else.
//! Writing every fact through `transact_to_graph` on a fresh store re-interns
//! everything, so ids and `Ref` payloads are correct **by construction** rather
//! than by remapping.
//!
//! That is also why the content hash is over sorted N-Triples and not over the
//! file: two stores holding the same triples with different id assignment must
//! hash the SAME, or the hash would describe the producer rather than the
//! content.
//!
//! ## Scope note (quipu #74)
//!
//! `--space <term-space>` is deliberately **not implemented here**: term-space
//! allocation is #74, which is gated. Nothing in this module's acceptance needs
//! it — re-interning is what makes a standalone pack correct — so the manifest
//! records `term_space: 0` and the flag lands with #74.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::{Error, Result};
use crate::store::Store;

/// What a pack declares about itself. Mirrored as meta-graph facts inside the
/// pack as well as stored in `pack_manifest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Format version of the pack itself.
    pub pack_format: String,
    /// Producer-chosen pack name.
    pub name: String,
    /// Producer-chosen version.
    pub version: String,
    /// Term space the pack allocates from. Always `0` until quipu #74.
    pub term_space: i64,
    /// `sha256:<hex>` over the canonical content (see [`content_hash`]).
    pub content_hash: String,
    /// When the pack was produced.
    pub created_at: String,
    /// The graph IRI this pack was cut from.
    pub source_graph: String,
    /// Producer identification, mirroring `GET /version`.
    pub producer: String,
    /// Row counts, as JSON.
    pub counts: String,
}

/// Options for [`pack`].
#[derive(Debug, Clone, Default)]
pub struct PackOptions {
    /// Pack name; defaults to the graph's local name.
    pub name: Option<String>,
    /// Pack version; defaults to `0.1.0`.
    pub version: Option<String>,
    /// Shape graphs to include. Shapes are global — they carry no graph
    /// linkage — so there is nothing to infer and they must be named.
    pub shapes: Vec<String>,
    /// Stored queries to include.
    pub queries: Vec<String>,
    /// Include embeddings, re-keyed by IRI.
    pub with_vectors: bool,
}

/// The canonical content of a pack, as the exact bytes that get hashed.
///
/// **Lexically sorted N-Triples**, plus the named shapes and queries and the
/// graph's labels, each section introduced by a marker line.
///
/// Sorting is load-bearing. `current_facts_in_graph` orders by term id
/// (`ORDER BY e, a`), which is neither total nor stable across stores — two
/// stores with the same triples assign ids in whatever order they happened to
/// intern them. Hashing emission order would therefore make the hash a
/// property of the PRODUCER, not of the content, and the "same triples,
/// different ids, same hash" acceptance would be unmeetable.
///
/// # Errors
/// Store and serialization errors.
pub fn canonical_content(
    store: &Store,
    graph_iri: &str,
    shapes: &[String],
    queries: &[String],
) -> Result<String> {
    let (bytes, _) =
        crate::rdf::export_rdf_subset(store, oxrdfio::RdfFormat::NTriples, Some(graph_iri))?;
    let text = String::from_utf8(bytes)
        .map_err(|e| Error::Serialization(format!("pack: non-UTF8 N-Triples: {e}")))?;

    // BTreeSet: sorted AND deduplicated. A triple emitted twice (the same
    // (e,a,v) re-asserted across transactions leaves multiple current rows)
    // must not change the hash — it is the same content.
    let triples: BTreeSet<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    let mut out = String::new();
    out.push_str("# quipu-pack-content v1\n## triples\n");
    for t in &triples {
        out.push_str(t);
        out.push('\n');
    }

    out.push_str("## shapes\n");
    for name in shapes {
        let found = store
            .list_shapes()?
            .into_iter()
            .find(|(n, _, _)| n == name)
            .ok_or_else(|| Error::InvalidValue(format!("pack: no such shape graph: {name}")))?;
        out.push_str(&format!("### {name}\n{}\n", found.1));
    }

    out.push_str("## queries\n");
    for name in queries {
        let q = store
            .query_get(name)?
            .ok_or_else(|| Error::InvalidValue(format!("pack: no such stored query: {name}")))?;
        // Field-by-field rather than a JSON dump: a serializer's key order is
        // not part of the content, and letting it into the hash would make the
        // hash depend on serde rather than on the query.
        out.push_str(&format!(
            "### {}\n{}\n{}\n",
            q.name, q.description, q.template
        ));
        if let Some(ds) = &q.dataset {
            out.push_str(&format!("dataset={ds}\n"));
        }
        for p in &q.params {
            out.push_str(&format!(
                "param {} {} {} {}\n",
                p.name,
                p.kind,
                p.required,
                p.default.as_deref().unwrap_or("-")
            ));
        }
    }

    out.push_str("## labels\n");
    let l = store.label_of(graph_iri)?;
    if let Some(f) = l.freshness.value {
        out.push_str(&format!("freshness={f}\n"));
    }
    if let Some(t) = &l.trust.value {
        out.push_str(&format!(
            "trust={} chain={} rank={}\n",
            t.iri, t.chain, t.rank
        ));
    }
    if let Some(p) = &l.policy.value {
        out.push_str(&format!("policy={}\n", p.tokens().join(" ")));
    }
    Ok(out)
}

/// `sha256:<hex>` over [`canonical_content`].
///
/// `ring`, not `sha2`: the signing path already depends on it, and the
/// verdict-evidence hash uses the same pairing. A second hashing crate for one
/// call is a dependency nobody needs.
#[must_use]
pub fn content_hash(canonical: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes());
    format!("sha256:{}", hex::encode(digest.as_ref()))
}

/// The `pack_manifest` DDL. One row, by construction.
const MANIFEST_SQL: &str = "CREATE TABLE IF NOT EXISTS pack_manifest (
     id           INTEGER PRIMARY KEY CHECK (id = 1),
     pack_format  TEXT NOT NULL,
     name         TEXT NOT NULL,
     version      TEXT NOT NULL,
     term_space   INTEGER NOT NULL,
     content_hash TEXT NOT NULL,
     created_at   TEXT NOT NULL,
     source_graph TEXT NOT NULL,
     producer     TEXT NOT NULL,
     counts       TEXT NOT NULL
 );";

fn local_name(iri: &str) -> String {
    iri.rsplit(['#', '/', ':'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(iri)
        .to_string()
}

/// Export `graph_iri` from `store` as a pack at `out_path`.
///
/// # Errors
/// Unknown graph, a named shape or query that does not exist, `--with-vectors`
/// against a non-SQLite backend, or any store/IO error.
pub fn pack(
    store: &Store,
    graph_iri: &str,
    out_path: &str,
    opts: &PackOptions,
    timestamp: &str,
) -> Result<Manifest> {
    if store.lookup(graph_iri)?.is_none() {
        return Err(Error::InvalidValue(format!(
            "pack: unknown graph: {graph_iri}"
        )));
    }
    // v1 restriction, refused up front rather than half-way through a build.
    // A delegate/Lance backend has no enumerate, so there is nothing to re-key
    // by IRI — and silently shipping a pack with no vectors when they were
    // asked for is the worse outcome.
    if opts.with_vectors && !store.has_sqlite_vector_backend() {
        return Err(Error::InvalidValue(
            "pack --with-vectors requires the built-in SQLite vector backend; a \
             delegated or LanceDB backend cannot be enumerated, so embeddings \
             cannot be re-keyed by IRI. Pack without vectors, or export from a \
             SQLite-backed store."
                .into(),
        ));
    }

    let canonical = canonical_content(store, graph_iri, &opts.shapes, &opts.queries)?;
    let hash = content_hash(&canonical);

    // Build into a sibling, then VACUUM INTO the shipped path. WAL would
    // otherwise leave `-wal`/`-shm` next to the file people actually copy, and
    // a pack that needs three files is not "a single attachable artifact".
    let build_path = format!("{out_path}.building");
    for p in [&build_path, out_path] {
        if Path::new(p).exists() {
            std::fs::remove_file(p)
                .map_err(|e| Error::Store(format!("pack: cannot replace {p}: {e}")))?;
        }
    }

    let facts = store.current_facts_in_graph(store.lookup(graph_iri)?.unwrap_or(0))?;
    let fact_count = facts.len();
    {
        let mut out = Store::open(&build_path)?;
        // Re-intern through the ordinary write path: ids and `Ref` BLOBs come
        // out correct by construction rather than by remapping.
        let g = out.overlay_create(graph_iri, 0)?;
        let mut datums = Vec::with_capacity(facts.len());
        for f in &facts {
            let e_iri = store.resolve(f.entity)?;
            let a_iri = store.resolve(f.attribute)?;
            let value = match &f.value {
                crate::types::Value::Ref(id) => {
                    crate::types::Value::Ref(out.intern(&store.resolve(*id)?)?)
                }
                other => other.clone(),
            };
            datums.push(crate::store::Datum {
                entity: out.intern(&e_iri)?,
                attribute: out.intern(&a_iri)?,
                value,
                valid_from: f.valid_from.clone(),
                valid_to: f.valid_to.clone(),
                op: crate::types::Op::Assert,
            });
        }
        out.transact_to_graph(&datums, timestamp, None, Some("pack"), g)?;

        for name in &opts.shapes {
            if let Some((_, turtle, _)) =
                store.list_shapes()?.into_iter().find(|(n, _, _)| n == name)
            {
                out.load_shapes(name, &turtle, timestamp)?;
            }
        }
        for name in &opts.queries {
            if let Some(q) = store.query_get(name)? {
                out.query_load(&q, timestamp)?;
            }
        }
        // Embeddings, RE-KEYED BY IRI. `vectors.entity_id` is a local term id
        // and does not travel, so the join is through the IRI on both sides.
        //
        // ⚠️ This was MISSING when #81 first landed: `--with_vectors` was
        // checked (it refuses a delegated backend) and then never acted on, so
        // asking for vectors on the ordinary SQLite path produced a pack with
        // none and said nothing. The refusal above exists precisely to avoid
        // "silently missing the vectors that were asked for" — and the SQLite
        // path did exactly that. A flag that is accepted and inert is the same
        // class of defect as an unwired config key.
        let mut vector_count = 0usize;
        if opts.with_vectors {
            let mut stmt = store.conn.prepare(
                "SELECT entity_id, text, embedding, valid_from, valid_to FROM vectors \
                 WHERE valid_to IS NULL",
            )?;
            // One `vectors` row as packed: entity id, text, embedding blob, and
            // the bitemporal window. Named because clippy reads the bare 5-tuple
            // as a complex type, and a name is better than an allow.
            type VectorRow = (i64, String, Vec<u8>, String, Option<String>);
            let rows: Vec<VectorRow> = stmt
                .query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (entity_id, text, embedding, valid_from, valid_to) in rows {
                // An embedding whose entity did not travel with this graph is
                // skipped rather than carried: a vector pointing at an IRI the
                // pack does not contain is a dangling row, and re-interning it
                // would silently widen the pack's term table.
                let Ok(iri) = store.resolve(entity_id) else {
                    continue;
                };
                let Some(local) = out.lookup(&iri)? else {
                    continue;
                };
                out.conn.execute(
                    "INSERT OR REPLACE INTO vectors \
                     (entity_id, text, embedding, valid_from, valid_to) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![local, text, embedding, valid_from, valid_to],
                )?;
                vector_count += 1;
            }
        }

        // Carry the graph's label so a consumer can compose it (#67) without
        // taking the producer's word for it out of band.
        let l = store.label_of(graph_iri)?;
        let label = crate::store::labels::GraphLabel {
            freshness: l.freshness.value,
            trust: l.trust.value.clone(),
            policy: l.policy.value.clone(),
        };
        if !label.is_empty() {
            out.set_graph_label(graph_iri, &label, timestamp, None)?;
        }

        let manifest = Manifest {
            pack_format: "1".into(),
            name: opts.name.clone().unwrap_or_else(|| local_name(graph_iri)),
            version: opts.version.clone().unwrap_or_else(|| "0.1.0".into()),
            term_space: 0,
            content_hash: hash.clone(),
            created_at: timestamp.to_string(),
            source_graph: graph_iri.to_string(),
            producer: serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "tool": "quipu pack",
            })
            .to_string(),
            counts: serde_json::json!({
                "facts": fact_count,
                "shapes": opts.shapes.len(),
                "queries": opts.queries.len(),
                "vectors": vector_count,
            })
            .to_string(),
        };
        out.conn.execute_batch(MANIFEST_SQL)?;
        out.conn.execute(
            "INSERT OR REPLACE INTO pack_manifest \
             (id, pack_format, name, version, term_space, content_hash, created_at, \
              source_graph, producer, counts) \
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                manifest.pack_format,
                manifest.name,
                manifest.version,
                manifest.term_space,
                manifest.content_hash,
                manifest.created_at,
                manifest.source_graph,
                manifest.producer,
                manifest.counts,
            ],
        )?;

        // VACUUM INTO produces a single clean file with no WAL siblings.
        out.conn
            .execute("VACUUM INTO ?1", rusqlite::params![out_path])?;
        drop(out);
    }
    // Remove the build artifact and ITS wal/shm siblings, so only the shipped
    // file remains.
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{build_path}{suffix}");
        if Path::new(&p).exists() {
            let _ = std::fs::remove_file(&p);
        }
    }

    read_manifest(out_path)
}

/// Read a pack's manifest.
///
/// # Errors
/// The file is not a pack, or cannot be opened.
pub fn read_manifest(path: &str) -> Result<Manifest> {
    let conn = rusqlite::Connection::open(path)?;
    conn.query_row(
        "SELECT pack_format, name, version, term_space, content_hash, created_at, \
                source_graph, producer, counts FROM pack_manifest WHERE id = 1",
        [],
        |r| {
            Ok(Manifest {
                pack_format: r.get(0)?,
                name: r.get(1)?,
                version: r.get(2)?,
                term_space: r.get(3)?,
                content_hash: r.get(4)?,
                created_at: r.get(5)?,
                source_graph: r.get(6)?,
                producer: r.get(7)?,
                counts: r.get(8)?,
            })
        },
    )
    .map_err(|e| Error::InvalidValue(format!("pack: {path} has no readable manifest: {e}")))
}

/// Recompute a pack's content hash and compare it to the manifest.
///
/// Returns `(stored, recomputed, matches)`.
///
/// # Errors
/// The file is not a pack, or cannot be opened as a store.
pub fn verify(path: &str) -> Result<(String, String, bool)> {
    let manifest = read_manifest(path)?;
    let store = Store::open(path)?;
    let shapes: Vec<String> = store
        .list_shapes()?
        .into_iter()
        .map(|(n, _, _)| n)
        .collect();
    let queries: Vec<String> = store.query_list()?.into_iter().map(|q| q.name).collect();
    let canonical = canonical_content(&store, &manifest.source_graph, &shapes, &queries)?;
    let recomputed = content_hash(&canonical);
    let matches = recomputed == manifest.content_hash;
    Ok((manifest.content_hash, recomputed, matches))
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;

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
/// Unknown graph, a named shape or query that does not exist, or any IO error.
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
