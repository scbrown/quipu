//! Named datasets — a name for an arbitrary set of graphs (quipu #69).
//!
//! Design: `docs/design/graph-labels.md` §7. The overlapping structure already
//! existed — `FROM a b c` *is* an arbitrary graph set and its merge semantics
//! are already correct. What was missing is a **name** for a set, so it can be
//! reused, labelled, governed, and handed to another agent.
//!
//! ## Called "dataset", not "view"
//!
//! `src/graph_view.rs` owns the word *view*. **Dataset** is SPARQL's own term
//! for the thing `apply_dataset` resolves, which is exactly what this is.
//!
//! ## `parent_branch` is not touched, and that is the point
//!
//! The branch tree is not a taxonomy; it is `compose_view`'s resolution root,
//! bind-once so an overlay cannot forge presence in a base it was never bound
//! to. Datasets and the branch tree are **different relations over the same
//! node set** — the branch tree is a tree, the datasets a semilattice, and
//! conflating them is the failure Alexander's essay names. Making
//! `parent_branch` many-parent would break overlay resolution for a benefit
//! that belongs here instead.
//!
//! ## Silence must not widen the dataset
//!
//! A dataset is **never implicitly active**. The ROOT-alone default survives
//! untouched: you get a dataset's graphs only by naming it.

use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::error::{Error, Result};
use crate::namespace::{META_GRAPH_IRI, QUIPU_DATASET, QUIPU_INCLUDES_GRAPH, RDF_TYPE};
use crate::types::{Op, Value};

use super::{Datum, Store};

/// One member of a named dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetMember {
    /// The member graph's IRI.
    pub graph_iri: String,
    /// Its rank within a *declared ordering*, or `None` for an unordered
    /// dataset.
    pub ord: Option<i64>,
}

impl DatasetMember {
    /// An unordered member.
    pub fn new(graph_iri: impl Into<String>) -> Self {
        Self {
            graph_iri: graph_iri.into(),
            ord: None,
        }
    }

    /// A member at a declared rank.
    pub fn ranked(graph_iri: impl Into<String>, ord: i64) -> Self {
        Self {
            graph_iri: graph_iri.into(),
            ord: Some(ord),
        }
    }
}

impl Store {
    /// Create (or replace) a named dataset.
    ///
    /// Members are stored as graph **IRIs**, not interned term ids — see
    /// `migrate_datasets` for why (the quipu #74 respace surface).
    ///
    /// Mirrored into the meta-graph as `quipu:Dataset` / `quipu:includesGraph`
    /// in the same savepoint as the tables, so the dataset is queryable and
    /// governed exactly as a graph label is.
    ///
    /// # Errors
    /// - a declared ordering repeats a rank — refused rather than tiebroken
    ///   silently, because a silent tiebreak is how "learned tactic beats
    ///   canonical" ships;
    /// - the caller lacks authority over the meta-graph;
    /// - no members (an empty dataset would widen nothing and name nothing).
    pub fn dataset_create(
        &mut self,
        name: &str,
        members: &[DatasetMember],
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<()> {
        if members.is_empty() {
            return Err(Error::InvalidValue(format!(
                "dataset '{name}' has no members; an empty dataset names nothing"
            )));
        }
        // Refuse duplicate ranks BEFORE touching the store. The unique index
        // would also catch it, but a checked refusal can name both members.
        let mut ranks: Vec<i64> = members.iter().filter_map(|m| m.ord).collect();
        ranks.sort_unstable();
        if let Some(dup) = ranks.windows(2).find(|w| w[0] == w[1]) {
            let clashing: Vec<&str> = members
                .iter()
                .filter(|m| m.ord == Some(dup[0]))
                .map(|m| m.graph_iri.as_str())
                .collect();
            return Err(Error::InvalidValue(format!(
                "dataset '{name}' declares rank {} for {} members ({}). A declared \
                 ordering must be unambiguous — ties are reported, never resolved \
                 silently.",
                dup[0],
                clashing.len(),
                clashing.join(", ")
            )));
        }

        let meta_g = self.meta_graph_id()?;
        let subject = self.intern(name)?;
        let a_type = self.intern(RDF_TYPE)?;
        let a_includes = self.intern(QUIPU_INCLUDES_GRAPH)?;
        let o_dataset = self.intern(QUIPU_DATASET)?;

        let mut datums = vec![Datum {
            entity: subject,
            attribute: a_type,
            value: Value::Ref(o_dataset),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        }];
        for m in members {
            let g_term = self.intern(&m.graph_iri)?;
            datums.push(Datum {
                entity: subject,
                attribute: a_includes,
                value: Value::Ref(g_term),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }

        self.conn.execute_batch("SAVEPOINT quipu_dataset")?;
        let result = (|| -> Result<()> {
            self.transact_to_graph(&datums, timestamp, actor, Some("dataset"), meta_g)?;
            self.conn.execute(
                "INSERT OR REPLACE INTO datasets (name, created_at) VALUES (?1, ?2)",
                params![name, timestamp],
            )?;
            self.conn.execute(
                "DELETE FROM dataset_members WHERE dataset = ?1",
                params![name],
            )?;
            for m in members {
                self.conn.execute(
                    "INSERT INTO dataset_members (dataset, graph_iri, ord) VALUES (?1, ?2, ?3)",
                    params![name, m.graph_iri, m.ord],
                )?;
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("RELEASE quipu_dataset")?;
                Ok(())
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_dataset; RELEASE quipu_dataset");
                Err(e)
            }
        }
    }

    /// Whether `iri` names a dataset.
    pub fn is_dataset(&self, iri: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM datasets WHERE name = ?1",
                params![iri],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// A dataset's members, ordered by declared rank (unranked members last,
    /// then by IRI so the order is stable rather than incidental).
    pub fn dataset_members(&self, name: &str) -> Result<Vec<DatasetMember>> {
        let mut stmt = self.conn.prepare(
            "SELECT graph_iri, ord FROM dataset_members WHERE dataset = ?1 \
             ORDER BY ord IS NULL, ord, graph_iri",
        )?;
        let out = stmt
            .query_map(params![name], |r| {
                Ok(DatasetMember {
                    graph_iri: r.get(0)?,
                    ord: r.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(out)
    }

    /// Every dataset name, sorted.
    pub fn dataset_list(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM datasets ORDER BY name")?;
        let out = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(out)
    }

    /// A dataset's members as interned graph ids, for the query path.
    ///
    /// A member naming a graph that is not REGISTERED contributes nothing — the
    /// same rule `apply_dataset` already applies to an unknown `FROM` IRI, so a
    /// dataset naming a graph that does not exist yet matches nothing rather
    /// than falling through to ROOT.
    ///
    /// The registry check is the load-bearing part, not the intern lookup:
    /// `dataset_create` interns every member IRI in order to write the
    /// meta-graph facts, so *every* member resolves as a term whether or not
    /// its graph exists. Filtering on `lookup` alone would therefore admit a
    /// typo'd member as a real one — matching no facts (harmless) but counting
    /// against the #67 label fold's coverage as a permanently undeclared
    /// member (not harmless, and invisible).
    pub fn dataset_member_ids(&self, name: &str) -> Result<Vec<i64>> {
        let mut ids = Vec::new();
        for m in self.dataset_members(name)? {
            let Some(g) = self.lookup(&m.graph_iri)? else {
                continue;
            };
            let registered = self
                .conn
                .query_row("SELECT 1 FROM graphs WHERE g = ?1", params![g], |_| Ok(()))
                .optional()?
                .is_some();
            if registered {
                ids.push(g);
            }
        }
        Ok(ids)
    }

    /// Remove a named dataset. The meta-graph facts are left in place —
    /// they are bitemporal history, and a dataset that existed is a fact about
    /// the past even once it is no longer active.
    pub fn dataset_remove(&self, name: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM datasets WHERE name = ?1", params![name])?;
        self.conn.execute(
            "DELETE FROM dataset_members WHERE dataset = ?1",
            params![name],
        )?;
        Ok(n > 0)
    }
}

/// The meta-graph IRI datasets are mirrored into, re-exported for callers that
/// want to query them directly.
pub const DATASET_META_GRAPH: &str = META_GRAPH_IRI;

#[cfg(test)]
#[path = "datasets_tests.rs"]
mod tests;
