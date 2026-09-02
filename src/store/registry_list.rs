//! The graph registry, listed — `GET /graphs` / `quipu_graph_list`.
//!
//! One row per registered graph with its class, provenance (`source`),
//! lifecycle and label cache. This is also the capability PROBE a consumer
//! uses to tell "this store predates graph kinds" apart from "there are no
//! such graphs" — a 404 on the endpoint is the former, an empty list the
//! latter. Silent zero rows is the failure mode this distinction exists to
//! prevent.

use rusqlite::{OptionalExtension, params};

use crate::error::Result;
use crate::schema::ROOT_GRAPH_IRI;

use super::Store;

/// One registered graph, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphInfo {
    /// The graph's IRI (`urn:quipu:graph:root` for ROOT).
    pub iri: String,
    /// The graph's registry id.
    pub g: i64,
    /// Storage class: `committed` or `overlay`.
    pub class: String,
    /// The attachment alias that contributed this graph; `None` = local.
    pub source: Option<String>,
    /// Storage lifecycle (`frozen` after a deep-freeze; `None` otherwise).
    pub lifecycle: Option<String>,
    /// Cached label axes, as declared strings.
    pub freshness: Option<String>,
    /// Cached durability.
    pub durability: Option<String>,
    /// Cached trust rank (meaningful only with `trust_chain`).
    pub trust_rank: Option<i64>,
    /// The chain the cached trust rank is expressed in.
    pub trust_chain: Option<String>,
    /// Cached policy tokens, space-separated.
    pub policy: Option<String>,
    /// The graph's declared data kind.
    pub kind: Option<String>,
}

impl Store {
    /// Live triple counts for every registered graph, excluding the metadata graph.
    ///
    /// This is the read-side inventory used by standards descriptions. Counts
    /// use the same live-fact predicate as graph queries (`op = 1` and no
    /// `valid_to`), so the advertised inventory cannot drift from queryable
    /// state.
    pub fn graph_fact_counts(&self) -> Result<Vec<(String, u64)>> {
        let meta_g = self.meta_graph_id()?;
        let mut stmt = self.conn.prepare(
            "SELECT gr.g, COUNT(f.e) FROM graphs gr \
             LEFT JOIN facts f ON f.g = gr.g AND f.op = 1 AND f.valid_to IS NULL \
             WHERE gr.g <> ?1 GROUP BY gr.g ORDER BY gr.g",
        )?;
        let rows = stmt
            .query_map([meta_g], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(g, count)| {
                let iri = if g == 0 {
                    ROOT_GRAPH_IRI.to_string()
                } else {
                    self.resolve(g)?
                };
                Ok((iri, u64::try_from(count).unwrap_or(0)))
            })
            .collect()
    }

    /// Vocabulary namespace IRIs actually used by live predicates.
    pub fn predicate_vocabularies(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT t.iri FROM facts f JOIN terms t ON t.id = f.a \
             WHERE f.op = 1 AND f.valid_to IS NULL ORDER BY t.iri",
        )?;
        let predicates = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut namespaces: Vec<String> = predicates
            .into_iter()
            .filter_map(|iri| {
                let cut = iri.rfind(['#', '/']).map(|i| i + 1)?;
                Some(iri[..cut].to_string())
            })
            .collect();
        namespaces.sort();
        namespaces.dedup();
        Ok(namespaces)
    }

    /// Every registered graph, optionally filtered by kind and/or lifecycle.
    ///
    /// The meta-graph is excluded (it holds labels *about* graphs); ROOT is
    /// included, named by its authority IRI. Filters compare the CACHE
    /// columns, which `quipu doctor labels` verifies against the RDF.
    pub fn list_graphs(
        &self,
        kind: Option<&str>,
        lifecycle: Option<&str>,
    ) -> Result<Vec<GraphInfo>> {
        let meta_g = self.meta_graph_id()?;
        let mut stmt = self.conn.prepare(
            "SELECT g, class, source, lifecycle, fresh_rank, durability_rank, \
                    trust_rank, trust_chain, policy, data_kind \
             FROM graphs WHERE g <> ?1 \
               AND (?2 IS NULL OR data_kind = ?2) \
               AND (?3 IS NULL OR lifecycle = ?3) \
             ORDER BY g",
        )?;
        type Row = (
            i64,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let rows: Vec<Row> = stmt
            .query_map(params![meta_g, kind, lifecycle], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;

        let mut out = Vec::with_capacity(rows.len());
        for (g, class, source, lc, fresh, durab, trust_rank, trust_chain, policy, data_kind) in rows
        {
            let iri = if g == 0 {
                ROOT_GRAPH_IRI.to_string()
            } else {
                self.resolve(g).unwrap_or_else(|_| format!("g={g}"))
            };
            out.push(GraphInfo {
                iri,
                g,
                class,
                source,
                lifecycle: lc,
                freshness: fresh.map(rank_to_freshness),
                durability: durab.map(rank_to_durability),
                trust_rank,
                trust_chain,
                policy,
                kind: data_kind,
            });
        }
        Ok(out)
    }

    /// The ids of every LOCAL, non-frozen-excluded graph declaring one of
    /// `kinds` — the resolver behind the `include_kinds` query param.
    ///
    /// Frozen graphs are included: composing archive graphs back in is
    /// exactly what the param is for. The meta-graph never carries a kind, so
    /// it can never be swept in.
    pub fn graphs_of_kinds(&self, kinds: &[String]) -> Result<Vec<i64>> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        // rusqlite has no array bind; the tokens are validated kind tokens
        // (lexically [a-z][a-z0-9-]*) so building the IN list is safe, but we
        // bind them anyway — one placeholder per token.
        let placeholders = (1..=kinds.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT g FROM graphs WHERE data_kind IN ({placeholders}) ORDER BY g"
        ))?;
        let ids = stmt
            .query_map(rusqlite::params_from_iter(kinds.iter()), |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// The latest RML materialization that wrote into graph `g`, if any
    /// (quipu-212). Parsed from the executor's transaction provenance
    /// convention — `rml:<mapping>|mapping=<hash>|source=<subject>|
    /// verified=<hash>` — so serving it costs no new storage and cannot
    /// drift from what actually committed. `None` means the graph has no
    /// RML materialization on record; the field is then omitted rather
    /// than faked, per the freshness discipline.
    pub fn mapped_provenance(&self, g: i64) -> Result<Option<Materialization>> {
        let row: Option<(i64, Option<String>, String)> = self
            .conn
            .query_row(
                "SELECT t.id, t.timestamp, t.source FROM transactions t \
                 WHERE t.source LIKE 'rml:%' \
                   AND EXISTS (SELECT 1 FROM facts f WHERE f.tx = t.id AND f.g = ?1) \
                 ORDER BY t.id DESC LIMIT 1",
                params![g],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(tx, timestamp, source)| {
            Materialization::parse(&source).map(|mut m| {
                m.tx = tx;
                m.timestamp = timestamp;
                m
            })
        }))
    }
}

/// One RML materialization, as recorded in transaction provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Materialization {
    /// The `rr:TriplesMap` IRI that produced the write.
    pub mapping_iri: String,
    /// Hash of the mapping closure at execution time.
    pub mapping_hash: String,
    /// The governed external-truth subject the source was verified against.
    pub source_subject: String,
    /// Hash of the source bytes actually read — the staleness comparand.
    pub verified_hash: String,
    /// The materializing transaction.
    pub tx: i64,
    /// Its commit timestamp.
    pub timestamp: Option<String>,
}

impl Materialization {
    /// Parse the executor's provenance string; anything that does not carry
    /// all four fields is not an RML materialization record.
    fn parse(source: &str) -> Option<Self> {
        let rest = source.strip_prefix("rml:")?;
        let mut mapping_iri = None;
        let mut mapping_hash = None;
        let mut source_subject = None;
        let mut verified_hash = None;
        for (i, part) in rest.split('|').enumerate() {
            if i == 0 {
                mapping_iri = Some(part.to_string());
            } else if let Some(v) = part.strip_prefix("mapping=") {
                mapping_hash = Some(v.to_string());
            } else if let Some(v) = part.strip_prefix("source=") {
                source_subject = Some(v.to_string());
            } else if let Some(v) = part.strip_prefix("verified=") {
                verified_hash = Some(v.to_string());
            }
        }
        Some(Self {
            mapping_iri: mapping_iri?,
            mapping_hash: mapping_hash?,
            source_subject: source_subject?,
            verified_hash: verified_hash?,
            tx: 0,
            timestamp: None,
        })
    }

    /// JSON as served on the graph listing.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "mapping": self.mapping_iri,
            "mapping_hash": self.mapping_hash,
            "source": self.source_subject,
            "verified_hash": self.verified_hash,
            "tx": self.tx,
            "timestamp": self.timestamp,
        })
    }
}

/// Decode the `fresh_rank` cache column to its declared string.
fn rank_to_freshness(rank: i64) -> String {
    match rank {
        0 => "stale".into(),
        1 => "recomputing".into(),
        2 => "fresh".into(),
        other => format!("<invalid {other}>"),
    }
}

/// Decode the `durability_rank` cache column to its declared string.
fn rank_to_durability(rank: i64) -> String {
    match rank {
        0 => "soleRecord".into(),
        1 => "reproducible".into(),
        2 => "backed".into(),
        other => format!("<invalid {other}>"),
    }
}
