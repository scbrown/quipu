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
//! ## Term spaces (quipu #74)
//!
//! `--space <n>` ships the pack in term space `n`, so it can be attached to a
//! consumer without their ids colliding. The pack is always **built** in
//! space 0 — `Store::open` interns the meta-graph before any caller gets a
//! say, so a fresh build store necessarily allocates from space 0 — and then
//! moved by the same machinery `quipu db respace` uses
//! ([`crate::store::respace::respace_file`]), which already rewrites `Ref`
//! BLOBs, updates `pack_manifest.term_space`, and asserts its own
//! post-conditions. The content hash is unaffected: it is computed over
//! IRIs, and a respace moves ids, not content.

use std::collections::BTreeSet;
#[cfg(not(target_arch = "wasm32"))]
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
    /// Term space the pack's ids live in (quipu #74): `0` unless the pack was
    /// shipped with `--space`.
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
    /// Ship the pack in this term space (quipu #74). `None` and `Some(0)`
    /// leave it in space 0. File packs only: the move is a respace of the
    /// built file, so [`pack_to_bytes`] (no file) and [`pack_turtle`] (no
    /// term ids at all) refuse a non-zero space rather than ignoring it.
    pub space: Option<i64>,
    /// Repository identity for a repo-embedded pack. These four provenance
    /// fields are all-or-none: a pack must never claim a commit without also
    /// naming the repository and embedding model that produced its graph.
    pub repository: Option<String>,
    /// Git commit whose repository graph is carried by the pack.
    pub repository_sha: Option<String>,
    /// Embedding model identifier used to produce repository knowledge.
    pub model_id: Option<String>,
    /// Version of the embedding model used to produce repository knowledge.
    pub model_version: Option<String>,
}

/// What [`unpack`] materialized and installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpackReport {
    /// `loaded` for new content, `unchanged` when this exact content hash was
    /// already loaded. The latter is a successful incremental no-op.
    pub outcome: String,
    /// Destination graph IRI.
    pub graph: String,
    /// Imported fact count.
    pub facts: usize,
    /// Versioned shape definitions installed.
    pub shapes: usize,
    /// Versioned stored queries installed.
    pub queries: usize,
    /// Entity embeddings restored from the pack, re-keyed by IRI (quipu-0v4).
    /// A pack built without `--with-vectors` carries none and reports 0.
    pub vectors: usize,
    /// Repository commit represented by this pack, when it is repo-embedded.
    pub repository_sha: Option<String>,
    /// Consumer HEAD supplied by the loader. A differing value means normal
    /// post-load ingestion must cover `repository_sha..head_sha`.
    pub head_sha: Option<String>,
}

#[derive(Debug, Clone, Default)]
/// Expectations and checkout state supplied while loading a repository pack.
pub struct LoadOptions<'a> {
    /// Optional destination graph override.
    pub into: Option<&'a str>,
    /// Repository identity the caller expects the manifest to carry.
    pub expect_repository: Option<&'a str>,
    /// Current checkout commit, used to report the incremental ingest range.
    pub head_sha: Option<&'a str>,
}

/// Materialize a pack into `destination`, installing registries by their
/// versioned write paths rather than overwriting consumer state (quipu #82).
///
/// # Errors
/// The pack is invalid, import-with-remap fails, or a carried registry entry
/// does not validate in the destination.
#[cfg(not(target_arch = "wasm32"))]
pub fn unpack(
    pack_path: &str,
    destination: &str,
    into: Option<&str>,
    timestamp: &str,
) -> Result<UnpackReport> {
    unpack_verified(
        pack_path,
        destination,
        &LoadOptions {
            into,
            ..Default::default()
        },
        timestamp,
    )
}

/// Verify and incrementally materialize a pack. The destination records the
/// content hash, making repeated clone/setup runs successful no-ops.
#[cfg(not(target_arch = "wasm32"))]
pub fn unpack_verified(
    pack_path: &str,
    destination: &str,
    opts: &LoadOptions<'_>,
    timestamp: &str,
) -> Result<UnpackReport> {
    let manifest = read_manifest(pack_path)?;
    let (stored, recomputed, matches) = verify(pack_path)?;
    if !matches {
        return Err(Error::InvalidValue(format!(
            "pack: HASH MISMATCH: manifest {stored}, recomputed {recomputed}"
        )));
    }
    let provenance: serde_json::Value = serde_json::from_str(&manifest.producer)
        .map_err(|e| Error::InvalidValue(format!("pack: invalid producer manifest: {e}")))?;
    let repository = provenance.get("repository").and_then(|v| v.as_str());
    let repository_sha = provenance
        .get("repository_sha")
        .and_then(|v| v.as_str())
        .map(String::from);
    if let Some(expected) = opts.expect_repository
        && repository != Some(expected)
    {
        return Err(Error::InvalidValue(format!(
            "pack: repository mismatch: expected {expected:?}, manifest has {repository:?}"
        )));
    }
    let graph = opts.into.unwrap_or(&manifest.source_graph).to_string();

    let dest = Store::open(destination)?;
    dest.conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pack_loads (
             content_hash TEXT PRIMARY KEY,
             repository TEXT,
             repository_sha TEXT,
             loaded_at TEXT NOT NULL
         );",
    )?;
    let loaded: bool = dest.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pack_loads WHERE content_hash=?1)",
        rusqlite::params![manifest.content_hash],
        |r| r.get(0),
    )?;
    drop(dest);
    if loaded {
        return Ok(UnpackReport {
            outcome: "unchanged".into(),
            graph,
            facts: 0,
            shapes: 0,
            queries: 0,
            vectors: 0,
            repository_sha,
            head_sha: opts.head_sha.map(String::from),
        });
    }

    // Read the carried registries before opening the destination. They remain
    // ordinary versioned definitions, never raw rows copied between files.
    let source = Store::open(pack_path)?;
    let shapes = source.list_shapes()?;
    let queries = source.query_list()?;
    drop(source);

    let imported =
        crate::store::import::import_graph(Path::new(destination), Path::new(pack_path), &graph)?;
    let dest = Store::open(destination)?;
    for (name, turtle, _) in &shapes {
        dest.load_shapes(name, turtle, timestamp)?;
    }
    for query in &queries {
        dest.query_load(query, timestamp)?;
    }
    dest.conn.execute(
        "INSERT INTO pack_loads (content_hash, repository, repository_sha, loaded_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![manifest.content_hash, repository, repository_sha, timestamp],
    )?;
    Ok(UnpackReport {
        outcome: "loaded".into(),
        graph,
        facts: imported.facts,
        shapes: shapes.len(),
        queries: queries.len(),
        vectors: imported.vectors,
        repository_sha,
        head_sha: opts.head_sha.map(String::from),
    })
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
    // Additive: a graph with no declared kind emits nothing here, so every
    // pre-kind pack hashes identically.
    if let Some(k) = &l.kind.value {
        out.push_str(&format!("kind={k}\n"));
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
pub(crate) const MANIFEST_SQL: &str = "CREATE TABLE IF NOT EXISTS pack_manifest (
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

pub(crate) fn local_name(iri: &str) -> String {
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
/// against a non-SQLite backend, a `space` outside
/// `[0, `[`crate::store::respace::MAX_SPACE`]`]`, or any store/IO error.
#[cfg(not(target_arch = "wasm32"))]
pub fn pack(
    store: &Store,
    graph_iri: &str,
    out_path: &str,
    opts: &PackOptions,
    timestamp: &str,
) -> Result<Manifest> {
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

    let built = (|| -> Result<()> {
        let mut out = Store::open(&build_path)?;
        pack_into(store, graph_iri, opts, timestamp, &mut out)?;
        match opts.space.unwrap_or(0) {
            0 => {
                // VACUUM INTO produces a single clean file with no WAL siblings.
                out.conn
                    .execute("VACUUM INTO ?1", rusqlite::params![out_path])?;
            }
            space => {
                // The build store is space 0 by construction; ship it in the
                // requested space through the respace machinery, which owns
                // Ref-BLOB rewriting, the `pack_manifest.term_space` row, and
                // its own post-conditions. `respace_file` closes cleanly, so
                // the shipped file has no `-wal`/`-shm` siblings either.
                drop(out);
                crate::store::respace::respace_file(
                    Path::new(&build_path),
                    Path::new(out_path),
                    space,
                )?;
            }
        }
        Ok(())
    })();
    // Remove the build artifact and ITS wal/shm siblings — on success so only
    // the shipped file remains, on failure so a refused pack leaves nothing.
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{build_path}{suffix}");
        if Path::new(&p).exists() {
            let _ = std::fs::remove_file(&p);
        }
    }
    built?;

    read_manifest(out_path)
}

/// Export `graph_iri` from `store` as the exact bytes of a pack `.db` file
/// (quipu-2l5). The browser-side counterpart of [`pack`]: the same build via
/// [`pack_into`], serialized with `sqlite3_serialize` instead of written to
/// disk. The bytes attach to a native store like any pack file.
///
/// # Errors
/// Same as [`pack`], minus the file IO; additionally refuses a non-zero
/// `space`, which only the file path can honour.
pub fn pack_to_bytes(
    store: &Store,
    graph_iri: &str,
    opts: &PackOptions,
    timestamp: &str,
) -> Result<(Manifest, Vec<u8>)> {
    // Refused rather than ignored: shipping bytes in space 0 when a space was
    // asked for is the same "accepted and inert flag" defect `--with-vectors`
    // shipped with once already.
    if opts.space.unwrap_or(0) != 0 {
        return Err(Error::InvalidValue(
            "pack --space requires a file destination: the space move is a \
             respace of the built file, and an in-memory pack has no file to \
             respace. Pack to a path, or respace the written bytes with \
             `quipu db respace`."
                .into(),
        ));
    }
    let mut out = Store::open_in_memory()?;
    let manifest = pack_into(store, graph_iri, opts, timestamp, &mut out)?;
    let bytes = out.serialize_db()?;
    Ok((manifest, bytes))
}

/// The pack build itself: re-intern `graph_iri`'s current facts (plus opted
/// shapes/queries/vectors, the graph label, and the manifest) into `out`.
/// Shared by [`pack`] (native, builds into a file store) and
/// [`pack_to_bytes`] (any target, builds into memory and serializes).
///
/// # Errors
/// Unknown graph, a named shape or query that does not exist, or
/// `with_vectors` against a non-SQLite backend.
fn pack_into(
    store: &Store,
    graph_iri: &str,
    opts: &PackOptions,
    timestamp: &str,
    out: &mut Store,
) -> Result<Manifest> {
    let repo_fields = [
        opts.repository.as_ref(),
        opts.repository_sha.as_ref(),
        opts.model_id.as_ref(),
        opts.model_version.as_ref(),
    ];
    let repo_field_count = repo_fields.iter().filter(|v| v.is_some()).count();
    if repo_field_count != 0 && repo_field_count != repo_fields.len() {
        return Err(Error::InvalidValue(
            "pack: --repo, --repo-sha, --model-id, and --model-version must be supplied together"
                .into(),
        ));
    }
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

    let facts = store.current_facts_in_graph(store.lookup(graph_iri)?.unwrap_or(0))?;
    let fact_count = facts.len();
    {
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
            durability: None,
            freshness: l.freshness.value,
            trust: l.trust.value.clone(),
            policy: l.policy.value.clone(),
            // Kind travels: it describes the CONTENT, not this store's custody
            // of it (durability, by contrast, is the consumer's judgment).
            kind: l.kind.value.clone(),
        };
        if !label.is_empty() {
            out.set_graph_label(graph_iri, &label, timestamp, None)?;
        }

        let manifest = Manifest {
            pack_format: "1".into(),
            name: opts.name.clone().unwrap_or_else(|| local_name(graph_iri)),
            version: opts.version.clone().unwrap_or_else(|| "0.1.0".into()),
            // The BUILD store's space. A `--space` ship rewrites this row on
            // the shipped file through the respace machinery (see `pack`).
            term_space: 0,
            content_hash: hash.clone(),
            created_at: timestamp.to_string(),
            source_graph: graph_iri.to_string(),
            producer: serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "git_sha": env!("QUIPU_GIT_SHA"),
                "tool": "quipu pack",
                "pack_schema_version": 1,
                "repository": opts.repository,
                "repository_sha": opts.repository_sha,
                "model_id": opts.model_id,
                "model_version": opts.model_version,
            })
            .to_string(),
            counts: serde_json::json!({
                "facts": fact_count,
                "shapes": opts.shapes.len(),
                "queries": opts.queries.len(),
                "vectors": vector_count,
                "embedding_model": store.embedding_config().model_path.as_ref()
                    .and_then(|p| p.file_name()).map(|p| p.to_string_lossy()),
                "embedding_dimension": store.embedding_config().dimension,
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

        Ok(manifest)
    }
}

/// Read a pack's manifest.
///
/// # Errors
/// The file is not a pack, or cannot be opened.
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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

// The turtle interop bundle: split for the size ratchet, path unchanged,
// gated like its module (export-only file IO — never exists on wasm).
#[cfg(not(target_arch = "wasm32"))]
pub use crate::pack_turtle::pack_turtle;
