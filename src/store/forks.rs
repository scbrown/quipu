//! Persistent named forks — fork the store at any transaction (quipu-gp5).
//!
//! A fork is a **committed-class named graph** pinned to a parent transaction,
//! registered by name in the `forks` table. No storage-engine changes: creating
//! a fork materializes ROOT's live-as-of-tx snapshot into the fork's graph, so
//! the fork is queryable exactly like any named graph and evolves as an
//! independent lineage. Design: `docs/design/fork-at-any-event.md`.
//!
//! The load-bearing constraint, stated up front: **promotion re-enters through
//! the write gates**. `fork_promote` computes the structural delta against ROOT
//! and applies it via `transact_to_graph(..., ROOT)` — the same authority,
//! placement, policy and OWL gates every ROOT write faces — after validating
//! the asserted delta against the stored reject-mode SHACL shapes. A refusal
//! writes nothing. Fork ergonomics must never become a gate bypass.
//!
//! The as-of snapshot uses the quipu #83 predicate
//! (`tx <= N AND (valid_to IS NULL OR retracted_tx > N)`), the same one the
//! SPARQL `as_of_tx` path uses — so "query the fork" and "query ROOT as of N"
//! agree. The #83 caveat carries over: a legacy row closed before
//! `retracted_tx` existed is invisible to as-of reads, so a fork of old history
//! can under-report. That gap is honest, not hidden — see the design doc.

use rusqlite::{OptionalExtension, params};

use crate::error::{Error, Result};
use crate::namespace::{QUIPU_FORK, QUIPU_FORK_TX, RDF_TYPE};
use crate::types::{Fact, Op, Value};

use super::{Datum, Store};

/// IRI prefix fork graphs are minted under: `urn:quipu:fork:<name>`.
pub const FORK_IRI_PREFIX: &str = "urn:quipu:fork:";

/// One row of the fork registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkInfo {
    /// The fork's registry name.
    pub name: String,
    /// The fork's graph id (the interned term id of its `urn:quipu:fork:` IRI).
    pub g: i64,
    /// The committed graph the fork was taken from (ROOT `0` in v1).
    pub parent_branch: i64,
    /// The parent transaction the fork is pinned to.
    pub fork_tx: i64,
    /// When the fork was created.
    pub created_at: String,
    /// `open` | `promoted` | `dropped`.
    pub status: String,
}

/// A structural diff between two graphs' **present-state** triple sets.
///
/// Scope, stated plainly: current `(e, a, v)` sets only. No valid-time-interval
/// diff and no per-transaction attribution — those are `unravel`'s job.
#[derive(Debug, Clone)]
pub struct ForkDiff {
    /// Present in `b`, absent in `a`.
    pub added: Vec<Fact>,
    /// Present in `a`, absent in `b`.
    pub removed: Vec<Fact>,
}

/// The outcome of [`Store::fork_promote`].
#[derive(Debug)]
pub enum ForkPromotion {
    /// The delta was applied to ROOT through the write gates.
    Promoted {
        /// The transaction the delta committed under (`0` for an empty delta).
        tx: i64,
        /// Triples asserted into ROOT.
        asserted: usize,
        /// ROOT triples retracted.
        retracted: usize,
    },
    /// SHACL refused the asserted delta. **Nothing was written.**
    #[cfg(feature = "shacl")]
    Refused(crate::shacl::ValidationFeedback),
}

impl Store {
    /// The graph IRI a fork named `name` lives in.
    #[must_use]
    pub fn fork_iri(name: &str) -> String {
        format!("{FORK_IRI_PREFIX}{name}")
    }

    /// Look up a fork by name, any status.
    pub fn fork_lookup(&self, name: &str) -> Result<Option<ForkInfo>> {
        Ok(self
            .conn
            .query_row(
                "SELECT name, g, parent_branch, fork_tx, created_at, status \
                 FROM forks WHERE name = ?1",
                params![name],
                |r| {
                    Ok(ForkInfo {
                        name: r.get(0)?,
                        g: r.get(1)?,
                        parent_branch: r.get(2)?,
                        fork_tx: r.get(3)?,
                        created_at: r.get(4)?,
                        status: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// Every fork, sorted by name.
    pub fn fork_list(&self) -> Result<Vec<ForkInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, g, parent_branch, fork_tx, created_at, status \
             FROM forks ORDER BY name",
        )?;
        let out = stmt
            .query_map([], |r| {
                Ok(ForkInfo {
                    name: r.get(0)?,
                    g: r.get(1)?,
                    parent_branch: r.get(2)?,
                    fork_tx: r.get(3)?,
                    created_at: r.get(4)?,
                    status: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(out)
    }

    /// Resolve a fork name to its graph id for a READ. Dropped forks are
    /// refused — their facts remain (history), but a dropped fork is no longer
    /// a sanctioned read surface; promoted forks stay readable.
    pub fn fork_graph_for_read(&self, name: &str) -> Result<i64> {
        let f = self
            .fork_lookup(name)?
            .ok_or_else(|| Error::InvalidValue(format!("no such fork: {name}")))?;
        if f.status == "dropped" {
            return Err(Error::InvalidValue(format!(
                "fork '{name}' is dropped; its facts remain as history but it \
                 is no longer readable by name"
            )));
        }
        Ok(f.g)
    }

    /// Create a named fork of ROOT as of transaction `at_tx` (quipu-gp5).
    ///
    /// Registers a committed-class graph `urn:quipu:fork:<name>`, mirrors the
    /// fork into the meta-graph (`quipu:Fork` / `quipu:forkTx`), and
    /// materializes ROOT's live-as-of-`at_tx` triples into the graph under one
    /// new transaction — all in one savepoint.
    ///
    /// # Errors
    /// A malformed name, a name already registered (any status — a dropped
    /// fork's facts remain, so its name is not reusable), or `at_tx` outside
    /// `0..=latest_tx_id`.
    pub fn fork_create(
        &mut self,
        name: &str,
        at_tx: i64,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<ForkInfo> {
        validate_fork_name(name)?;
        if let Some(existing) = self.fork_lookup(name)? {
            return Err(Error::InvalidValue(format!(
                "fork '{name}' already exists (status: {}); fork names are not \
                 reusable because a fork's graph facts are history",
                existing.status
            )));
        }
        let latest = self.latest_tx_id()?;
        if at_tx < 0 || at_tx > latest {
            return Err(Error::InvalidValue(format!(
                "cannot fork at tx {at_tx}: the store's latest transaction is {latest}"
            )));
        }

        let iri = Self::fork_iri(name);
        let meta_g = self.meta_graph_id()?;
        let a_type = self.intern(RDF_TYPE)?;
        let a_fork_tx = self.intern(QUIPU_FORK_TX)?;
        let o_fork = self.intern(QUIPU_FORK)?;

        self.conn.execute_batch("SAVEPOINT quipu_fork_create")?;
        let result = (|| -> Result<ForkInfo> {
            let g = self.graph_create(&iri)?;
            // Meta-graph mirror, through the ordinary write gates — exactly the
            // `dataset_create` precedent.
            let datums = vec![
                Datum {
                    entity: g,
                    attribute: a_type,
                    value: Value::Ref(o_fork),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: g,
                    attribute: a_fork_tx,
                    value: Value::Int(at_tx),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ];
            self.transact_to_graph(&datums, timestamp, actor, Some("fork"), meta_g)?;
            self.conn.execute(
                "INSERT INTO forks (name, g, parent_branch, fork_tx, created_at, status) \
                 VALUES (?1, ?2, 0, ?3, ?4, 'open')",
                params![name, g, at_tx, timestamp],
            )?;
            // Materialize the snapshot under ONE new transaction. GROUP BY
            // collapses a triple re-asserted across transactions to one row
            // (the same dedup the read paths apply); MIN(valid_from) keeps the
            // earliest claim. valid_to is NULL by construction: a row live at
            // `at_tx` but retracted since was live in THIS lineage's history,
            // and in the fork's lineage that retraction never happened.
            self.conn.execute(
                "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
                params![timestamp, actor, format!("fork:{name}")],
            )?;
            let mat_tx = self.conn.last_insert_rowid();
            self.conn.execute(
                "INSERT INTO facts (e, a, v, g, tx, valid_from, valid_to, op) \
                 SELECT e, a, v, ?1, ?2, MIN(valid_from), NULL, 1 FROM facts \
                 WHERE g = 0 AND op = 1 AND tx <= ?3 \
                   AND (valid_to IS NULL OR retracted_tx > ?3) \
                 GROUP BY e, a, v",
                params![g, mat_tx, at_tx],
            )?;
            self.emit_registry_event("fork.created", name, timestamp)?;
            Ok(ForkInfo {
                name: name.to_string(),
                g,
                parent_branch: 0,
                fork_tx: at_tx,
                created_at: timestamp.to_string(),
                status: "open".to_string(),
            })
        })();

        match result {
            Ok(info) => {
                self.conn.execute_batch("RELEASE quipu_fork_create")?;
                Ok(info)
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_fork_create; RELEASE quipu_fork_create");
                Err(e)
            }
        }
    }

    /// Mark a fork dropped. Its facts and meta-graph rows are LEFT IN PLACE —
    /// the `dataset_remove` precedent: a fork that existed is a fact about the
    /// past. Only an `open` fork can be dropped.
    pub fn fork_drop(&mut self, name: &str, timestamp: &str) -> Result<()> {
        let f = self.require_open_fork(name, "drop")?;
        self.conn.execute(
            "UPDATE forks SET status = 'dropped' WHERE name = ?1",
            params![f.name],
        )?;
        self.emit_registry_event("fork.dropped", name, timestamp)?;
        Ok(())
    }

    /// Structural diff of two sides' present-state triple sets. Each side is a
    /// fork name or `main`/`ROOT`. `added` = in `b` only; `removed` = in `a`
    /// only. Term-id comparison is sound here because both sides live in one
    /// store's dictionary.
    pub fn fork_diff(&self, a: &str, b: &str) -> Result<ForkDiff> {
        let ga = self.resolve_diff_side(a)?;
        let gb = self.resolve_diff_side(b)?;
        let fa = self.current_facts_in_graph(ga)?;
        let fb = self.current_facts_in_graph(gb)?;
        let key = |f: &Fact| (f.entity, f.attribute, f.value.to_bytes());
        let set_a: std::collections::BTreeSet<_> = fa.iter().map(&key).collect();
        let set_b: std::collections::BTreeSet<_> = fb.iter().map(&key).collect();
        let mut seen = std::collections::BTreeSet::new();
        let added = fb
            .into_iter()
            .filter(|f| !set_a.contains(&key(f)) && seen.insert(key(f)))
            .collect();
        seen.clear();
        let removed = fa
            .into_iter()
            .filter(|f| !set_b.contains(&key(f)) && seen.insert(key(f)))
            .collect();
        Ok(ForkDiff { added, removed })
    }

    /// Promote an open fork: apply its present-state delta to ROOT **through
    /// the write gates**, then mark it `promoted`.
    ///
    /// The asserted delta is validated against the stored reject-mode SHACL
    /// shapes first (`split_shapes_by_policy` — emit-mode shapes observe, they
    /// do not gate), with the store as repair context. A refusal returns
    /// [`ForkPromotion::Refused`] and writes NOTHING. The transact itself then
    /// supplies the authority / placement / policy / OWL gates — an `Err` from
    /// any of them rolls the whole promotion back and the fork stays `open`.
    pub fn fork_promote(
        &mut self,
        name: &str,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<ForkPromotion> {
        let _f = self.require_open_fork(name, "promote")?;
        let diff = self.fork_diff("main", name)?;

        #[cfg(feature = "shacl")]
        if !diff.added.is_empty()
            && let Some(shapes) = self.get_combined_shapes()?
        {
            let split = crate::shacl::split_shapes_by_policy(&shapes);
            let turtle_bytes =
                crate::rdf::serialize_facts(self, &diff.added, oxrdfio::RdfFormat::NTriples)?;
            let turtle = String::from_utf8(turtle_bytes).map_err(|e| {
                Error::InvalidValue(format!("fork promote: non-UTF8 N-Triples: {e}"))
            })?;
            let feedback =
                crate::shacl_context::validate_with_store_context(self, &split.reject, &turtle)?;
            if !feedback.conforms {
                return Ok(ForkPromotion::Refused(feedback));
            }
        }

        let mut datums: Vec<Datum> = Vec::with_capacity(diff.added.len() + diff.removed.len());
        for f in &diff.added {
            datums.push(Datum {
                entity: f.entity,
                attribute: f.attribute,
                value: f.value.clone(),
                // The fork fact's own valid-time claim, not the promote instant.
                valid_from: f.valid_from.clone(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        for f in &diff.removed {
            datums.push(Datum {
                entity: f.entity,
                attribute: f.attribute,
                value: f.value.clone(),
                valid_from: f.valid_from.clone(),
                valid_to: None,
                op: Op::Retract,
            });
        }

        self.conn.execute_batch("SAVEPOINT quipu_fork_promote")?;
        let result = (|| -> Result<i64> {
            let tx = if datums.is_empty() {
                0
            } else {
                self.transact_to_graph(
                    &datums,
                    timestamp,
                    actor,
                    Some(&format!("fork-promote:{name}")),
                    crate::schema::ROOT_GRAPH,
                )?
            };
            self.conn.execute(
                "UPDATE forks SET status = 'promoted' WHERE name = ?1",
                params![name],
            )?;
            self.emit_registry_event("fork.promoted", name, timestamp)?;
            Ok(tx)
        })();

        match result {
            Ok(tx) => {
                self.conn.execute_batch("RELEASE quipu_fork_promote")?;
                Ok(ForkPromotion::Promoted {
                    tx,
                    asserted: diff.added.len(),
                    retracted: diff.removed.len(),
                })
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_fork_promote; RELEASE quipu_fork_promote");
                Err(e)
            }
        }
    }

    // -- Internal --

    fn require_open_fork(&self, name: &str, verb: &str) -> Result<ForkInfo> {
        let f = self
            .fork_lookup(name)?
            .ok_or_else(|| Error::InvalidValue(format!("no such fork: {name}")))?;
        if f.status != "open" {
            return Err(Error::InvalidValue(format!(
                "cannot {verb} fork '{name}': status is '{}', not 'open'",
                f.status
            )));
        }
        Ok(f)
    }

    fn resolve_diff_side(&self, side: &str) -> Result<i64> {
        if side.eq_ignore_ascii_case("main") || side.eq_ignore_ascii_case("root") {
            return Ok(crate::schema::ROOT_GRAPH);
        }
        self.fork_graph_for_read(side)
    }
}

/// Fork names become IRI suffixes and CLI arguments; keep them to a shape that
/// cannot smuggle either. Refused loudly, never sanitized silently.
fn validate_fork_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidValue(format!(
            "invalid fork name '{name}': use 1-128 of [A-Za-z0-9._-]"
        )))
    }
}

#[cfg(test)]
#[path = "forks_tests.rs"]
mod tests;
