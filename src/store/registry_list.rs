//! The graph registry, listed — `GET /graphs` / `quipu_graph_list`.
//!
//! One row per registered graph with its class, provenance (`source`),
//! lifecycle and label cache. This is also the capability PROBE a consumer
//! uses to tell "this store predates graph kinds" apart from "there are no
//! such graphs" — a 404 on the endpoint is the former, an empty list the
//! latter. Silent zero rows is the failure mode this distinction exists to
//! prevent.

use rusqlite::params;

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
