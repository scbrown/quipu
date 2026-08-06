//! Graph labels — the store half of quipu #65.
//!
//! Design: `docs/design/graph-labels.md` §2.1, §3. The algebra lives in
//! [`crate::lattice`] (#66); this module is storage and retrieval only.
//!
//! ## RDF is authoritative; the columns are a cache
//!
//! A label is written as ordinary facts in the reserved meta-graph
//! ([`META_GRAPH_IRI`]) **and** denormalized into five nullable columns on
//! `graphs`, in one savepoint. The columns exist because the dataset fold runs
//! at query entry, where a nested SPARQL evaluation would be a re-entrancy
//! hazard on the single connection — but they are derived, and
//! [`Store::graph_label_drift`] recomputes from RDF to prove it.
//!
//! Pure columns were rejected because they lose history, governance and
//! queryability — the reasons to put labels in a knowledge graph at all.
//!
//! ## The authority consequence, stated plainly
//!
//! Label writes go through [`Store::transact_to_graph`], which calls
//! `enforce_graph_authority` **on the graph being written** — and that is the
//! *meta-graph*, not the graph being labelled. So relabelling a graph requires
//! authority over `urn:quipu:graph:meta`. That is the point: otherwise a tenant
//! with authority over its own graph relabels itself `attested`.
//!
//! ## Undeclared is not a lattice value
//!
//! An unlabelled graph reads back as [`Coverage::None`] with no value — never a
//! fabricated `fresh`/⊤/⊥ (§2.1). Silence must not flatter.

use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::error::{Error, Result};
use crate::lattice::{Composed, Coverage, Freshness, PolicyClass, Trust};
use crate::namespace::{
    META_GRAPH_IRI, QUIPU_FRESHNESS, QUIPU_IN_CHAIN, QUIPU_POLICY_CLASS, QUIPU_TRUST,
    QUIPU_TRUST_RANK,
};
use crate::types::{Op, Value};

use super::{Datum, Store};

/// A graph's declared label across the three axes. Every axis is optional —
/// declaring freshness alone is normal and must not imply anything about trust.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphLabel {
    /// How current the graph's contents are.
    pub freshness: Option<Freshness>,
    /// The graph's trust value, with the chain that ranks it.
    pub trust: Option<Trust>,
    /// Obligation tokens carried by the graph.
    pub policy: Option<PolicyClass>,
}

impl GraphLabel {
    /// Whether this label declares nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.freshness.is_none() && self.trust.is_none() && self.policy.is_none()
    }

    /// Per-axis coverage for a single graph: `Full` where declared, `None`
    /// where not. The dataset fold composes these.
    #[must_use]
    fn coverage_of<T>(v: Option<&T>) -> Coverage {
        if v.is_some() {
            Coverage::Full
        } else {
            Coverage::None
        }
    }
}

/// A graph's label as read back, each axis carrying its own coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadLabel {
    /// Composed freshness for this graph.
    pub freshness: Composed<Freshness>,
    /// Composed trust for this graph.
    pub trust: Composed<Trust>,
    /// Composed policy for this graph.
    pub policy: Composed<PolicyClass>,
    /// The transaction that last wrote this graph's labels.
    pub labels_tx: Option<i64>,
}

/// A dataset's composed label — the fold over its member graphs (quipu #67).
///
/// Each axis carries its own [`Coverage`], because a graph may declare
/// freshness and say nothing about trust. `value: None` means *undeclared*, and
/// is never a fabricated `fresh`/⊤/⊥.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetLabels {
    /// Composed freshness — the meet over declared members.
    pub freshness: Composed<Freshness>,
    /// Composed trust — the meet over declared members, within one chain.
    pub trust: Composed<Trust>,
    /// Composed policy — the JOIN (union) of declared members' obligations.
    pub policy: Composed<PolicyClass>,
}

impl DatasetLabels {
    /// Whether no member declared anything on any axis. `/query` reports
    /// `"labels": null` in this case rather than three empty objects.
    #[must_use]
    pub fn is_undeclared(&self) -> bool {
        self.freshness.value.is_none() && self.trust.value.is_none() && self.policy.value.is_none()
    }
}

/// One graph's disagreement between the RDF source of truth and the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDrift {
    /// The graph's IRI.
    pub graph_iri: String,
    /// The axis that disagrees (`freshness`, `trust`, `policy`).
    pub axis: &'static str,
    /// What the meta-graph facts say — the authority.
    pub rdf: String,
    /// What the `graphs` cache column says.
    pub cached: String,
}

/// Serialize obligation tokens for the `policy` cache column.
///
/// Space-separated and sorted (`PolicyClass` is backed by a `BTreeSet`, so the
/// order is already canonical). Tokens containing whitespace are refused rather
/// than silently producing a value that reads back as two tokens.
fn encode_policy(p: &PolicyClass) -> Result<String> {
    for tok in p.tokens() {
        if tok.chars().any(char::is_whitespace) {
            return Err(Error::InvalidValue(format!(
                "policy token '{tok}' contains whitespace; tokens are stored \
                 space-separated and would read back as two distinct tokens"
            )));
        }
    }
    Ok(p.tokens().join(" "))
}

fn decode_policy(s: &str) -> PolicyClass {
    PolicyClass::new(s.split_whitespace())
}

/// The five cache columns as read from `graphs`:
/// `(fresh_rank, trust_rank, trust_chain, policy, labels_tx)`.
type CacheRow = (
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// One `graphs` row during a drift sweep: `(g, fresh_rank, trust_chain, policy)`.
type DriftRow = (i64, Option<i64>, Option<String>, Option<String>);

impl Store {
    /// The meta-graph's `g`. Interned by the migration, so this is a lookup and
    /// never a write.
    pub fn meta_graph_id(&self) -> Result<i64> {
        self.lookup(META_GRAPH_IRI)?.ok_or_else(|| {
            Error::Store(
                "the label meta-graph is not interned; the graph-label migration \
                 did not run on this store"
                    .into(),
            )
        })
    }

    /// Declare a graph's label.
    ///
    /// Writes the meta-graph facts **and** the cache columns inside one
    /// savepoint, so the two can never diverge through a partial failure —
    /// the invariant `quipu doctor labels` exists to check is established at
    /// write time rather than repaired afterwards.
    ///
    /// Returns the transaction id of the label write.
    ///
    /// # Errors
    /// - the caller's chain lacks authority over the **meta-graph** (see the
    ///   module docs — that is deliberately not authority over `graph_iri`);
    /// - a policy token contains whitespace;
    /// - the label declares nothing (an empty label is a no-op the caller
    ///   almost certainly did not mean, so it is refused rather than silently
    ///   writing an empty transaction).
    pub fn set_graph_label(
        &mut self,
        graph_iri: &str,
        label: &GraphLabel,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<i64> {
        if label.is_empty() {
            return Err(Error::InvalidValue(format!(
                "set_graph_label on '{graph_iri}' declares no axis; to clear a \
                 label, retract the meta-graph facts explicitly"
            )));
        }

        let meta_g = self.meta_graph_id()?;
        let subject = self.intern(graph_iri)?;

        let mut datums: Vec<Datum> = Vec::new();
        if let Some(f) = label.freshness {
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(QUIPU_FRESHNESS)?,
                value: Value::Str(f.as_str().to_string()),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        if let Some(t) = &label.trust {
            let trust_term = self.intern(&t.iri)?;
            let chain_term = self.intern(&t.chain)?;
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(QUIPU_TRUST)?,
                value: Value::Ref(trust_term),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
            // The chain and rank are facts about the TRUST VALUE, not about the
            // graph — that is what makes the ordering data rather than a
            // hardcoded enum, and what lets two consumers ship different chains.
            datums.push(Datum {
                entity: trust_term,
                attribute: self.intern(QUIPU_IN_CHAIN)?,
                value: Value::Ref(chain_term),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
            datums.push(Datum {
                entity: trust_term,
                attribute: self.intern(QUIPU_TRUST_RANK)?,
                value: Value::Int(t.rank),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        if let Some(p) = &label.policy {
            let attr = self.intern(QUIPU_POLICY_CLASS)?;
            for tok in p.tokens() {
                datums.push(Datum {
                    entity: subject,
                    attribute: attr,
                    value: Value::Str(tok.to_string()),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                });
            }
        }

        let policy_encoded = match &label.policy {
            Some(p) => Some(encode_policy(p)?),
            None => None,
        };

        // One savepoint over BOTH writes. `transact_to_graph` opens its own
        // `quipu_transact` inside this one — the same nesting `speculate` uses.
        self.conn.execute_batch("SAVEPOINT quipu_set_label")?;
        let result = (|| -> Result<i64> {
            let tx =
                self.transact_to_graph(&datums, timestamp, actor, Some("graph-label"), meta_g)?;
            let updated = self.conn.execute(
                "UPDATE graphs SET fresh_rank = ?2, trust_rank = ?3, trust_chain = ?4, \
                 policy = ?5, labels_tx = ?6 WHERE g = ?1",
                params![
                    subject,
                    label.freshness.map(|f| f as i64),
                    label.trust.as_ref().map(|t| t.rank),
                    label.trust.as_ref().map(|t| t.chain.clone()),
                    policy_encoded,
                    tx,
                ],
            )?;
            // An UPDATE matching no row is not an error in SQL, and that is
            // exactly how this would go wrong quietly: labelling a graph that
            // was never registered in `graphs` (a typo'd IRI, most likely)
            // would write the meta-graph facts and cache nothing, leaving
            // permanent drift that only `doctor labels` would ever surface.
            // Refuse instead, and let the savepoint take the facts back out.
            if updated != 1 {
                return Err(Error::InvalidValue(format!(
                    "cannot label '{graph_iri}': it is not a registered graph \
                     (no row in `graphs`). Create it first — labelling an \
                     unregistered graph would write facts the cache could \
                     never mirror."
                )));
            }
            Ok(tx)
        })();

        match result {
            Ok(tx) => {
                self.conn.execute_batch("RELEASE quipu_set_label")?;
                Ok(tx)
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_set_label; RELEASE quipu_set_label");
                Err(e)
            }
        }
    }

    /// Read a graph's label from the cache.
    ///
    /// An unregistered or unlabelled graph yields [`Coverage::None`] on every
    /// axis with no value — *undeclared*, never a fabricated label.
    ///
    /// # Errors
    /// When the cached `trust_chain` no longer matches the chain the RDF
    /// declares for that trust value — a chain redefinition has left the cache
    /// describing an ordering that no longer exists, and the honest response is
    /// a refusal rather than a rank whose meaning has silently moved.
    pub fn label_of(&self, graph_iri: &str) -> Result<ReadLabel> {
        let Some(g) = self.lookup(graph_iri)? else {
            return Ok(Self::undeclared_label());
        };
        self.label_of_id(g)
    }

    /// [`Store::label_of`] by graph id, for callers that already resolved it —
    /// the dataset fold reads ids straight out of `GraphScope`.
    ///
    /// # Errors
    /// As [`Store::label_of`].
    pub fn label_of_id(&self, g: i64) -> Result<ReadLabel> {
        let graph_iri = self.resolve(g).unwrap_or_else(|_| format!("g={g}"));
        let graph_iri = graph_iri.as_str();

        let row: Option<CacheRow> = self
            .conn
            .query_row(
                "SELECT fresh_rank, trust_rank, trust_chain, policy, labels_tx \
                     FROM graphs WHERE g = ?1",
                params![g],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;

        let Some((fresh_rank, trust_rank, trust_chain, policy, labels_tx)) = row else {
            return Ok(Self::undeclared_label());
        };

        let freshness = match fresh_rank {
            Some(0) => Some(Freshness::Stale),
            Some(1) => Some(Freshness::Recomputing),
            Some(2) => Some(Freshness::Fresh),
            Some(other) => {
                return Err(Error::Store(format!(
                    "graph '{graph_iri}' has cached fresh_rank {other}, which is not \
                     a freshness value; recompute with `quipu doctor labels`"
                )));
            }
            None => None,
        };

        let trust = match (trust_rank, trust_chain) {
            (Some(rank), Some(chain)) => {
                let iri = self.declared_trust_iri(g)?;
                if let Some(iri) = iri {
                    self.assert_chain_still_current(graph_iri, &iri, &chain)?;
                    Some(Trust::new(iri, chain, rank))
                } else {
                    // Cache says trusted, RDF has no trust fact. That is drift,
                    // not an undeclared graph — refuse rather than report either
                    // value as if it were agreed.
                    return Err(Error::Store(format!(
                        "graph '{graph_iri}' has a cached trust rank but no \
                         quipu:trust fact in the meta-graph; the cache and RDF \
                         disagree. Run `quipu doctor labels`."
                    )));
                }
            }
            (None, None) => None,
            _ => {
                return Err(Error::Store(format!(
                    "graph '{graph_iri}' has a half-written trust cache (rank and \
                     chain must be set together). Run `quipu doctor labels`."
                )));
            }
        };

        let policy = policy.as_deref().map(decode_policy);

        Ok(ReadLabel {
            freshness: Composed {
                coverage: GraphLabel::coverage_of(freshness.as_ref()),
                value: freshness,
            },
            trust: Composed {
                coverage: GraphLabel::coverage_of(trust.as_ref()),
                value: trust,
            },
            policy: Composed {
                coverage: GraphLabel::coverage_of(policy.as_ref()),
                value: policy,
            },
            labels_tx,
        })
    }

    fn undeclared_label() -> ReadLabel {
        ReadLabel {
            freshness: Composed {
                value: None,
                coverage: Coverage::None,
            },
            trust: Composed {
                value: None,
                coverage: Coverage::None,
            },
            policy: Composed {
                value: None,
                coverage: Coverage::None,
            },
            labels_tx: None,
        }
    }

    /// The most recent current value of `(e, predicate)` in `graph`.
    ///
    /// `facts.v` is an opaque [`Value::to_bytes`] BLOB, **not** a SQL-typed
    /// column — reading it as TEXT or INTEGER fails at runtime, and that is by
    /// design (it is also why quipu #74's respace has to rewrite `Ref` payloads
    /// rather than remap them in SQL). Every read below goes through
    /// [`Value::from_bytes`].
    ///
    /// Plain SQL, deliberately not SPARQL: `label_of` runs on the query path,
    /// where a nested evaluation is the re-entrancy hazard the cache exists to
    /// avoid in the first place.
    fn current_value(&self, e: i64, predicate: &str, graph: i64) -> Result<Option<Value>> {
        let Some(attr) = self.lookup(predicate)? else {
            return Ok(None);
        };
        let raw: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT v FROM facts WHERE e = ?1 AND a = ?2 AND g = ?3 \
                 AND op = 1 AND valid_to IS NULL ORDER BY tx DESC LIMIT 1",
                params![e, attr, graph],
                |r| r.get(0),
            )
            .optional()?;
        match raw {
            Some(bytes) => Ok(Some(Value::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Every current value of `(e, predicate)` in `graph`.
    fn current_values(&self, e: i64, predicate: &str, graph: i64) -> Result<Vec<Value>> {
        let Some(attr) = self.lookup(predicate)? else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT v FROM facts WHERE e = ?1 AND a = ?2 AND g = ?3 \
             AND op = 1 AND valid_to IS NULL",
        )?;
        let raws = stmt
            .query_map(params![e, attr, graph], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        raws.iter().map(|b| Value::from_bytes(b)).collect()
    }

    /// Resolve a `Value::Ref` to its IRI; anything else is not a reference.
    fn ref_iri(&self, v: Option<&Value>) -> Result<Option<String>> {
        match v {
            Some(Value::Ref(id)) => Ok(Some(self.resolve(*id)?)),
            _ => Ok(None),
        }
    }

    /// The trust IRI currently asserted for `g` in the meta-graph.
    fn declared_trust_iri(&self, g: i64) -> Result<Option<String>> {
        let meta_g = self.meta_graph_id()?;
        let v = self.current_value(g, QUIPU_TRUST, meta_g)?;
        self.ref_iri(v.as_ref())
    }

    /// Refuse if the chain that ranks `trust_iri` has been redefined since the
    /// cache was written.
    fn assert_chain_still_current(
        &self,
        graph_iri: &str,
        trust_iri: &str,
        cached_chain: &str,
    ) -> Result<()> {
        let Some(trust_term) = self.lookup(trust_iri)? else {
            return Ok(());
        };
        let meta_g = self.meta_graph_id()?;
        let current = self.current_value(trust_term, QUIPU_IN_CHAIN, meta_g)?;
        let Some(current_iri) = self.ref_iri(current.as_ref())? else {
            return Ok(());
        };
        if current_iri != cached_chain {
            return Err(Error::Store(format!(
                "graph '{graph_iri}' caches trust value '{trust_iri}' in chain \
                 '{cached_chain}', but the meta-graph now ranks it in chain \
                 '{current_iri}'. A rank means nothing outside the chain that \
                 declared it, so this is refused rather than answered. Run \
                 `quipu doctor labels` to recompute."
            )));
        }
        Ok(())
    }

    /// The composed label of a whole dataset — the fold over its member graphs
    /// (quipu #67, graph-labels.md §4).
    ///
    /// **Computed once per query, not per row.** This is
    /// `O(|dataset|)` — a handful of PK-indexed reads on `graphs` — and never
    /// `O(|rows|)`. Per-solution labelling is a *semantic* blocker rather than a
    /// cost knob: `sparql/triple.rs` omits `g` from the projection precisely so
    /// `SELECT DISTINCT e, a, v` collapses a triple present in three graphs to
    /// one solution, which IS the RDF-merge semantics of `FROM`. Projecting `g`
    /// to carry a per-row label would turn one solution into three — different
    /// results, not slower ones.
    ///
    /// **Conservative by construction:** a dataset containing a stale graph is
    /// labelled stale even if no returned row came from it. Conservative cannot
    /// overstate, which is the direction every mapping in this stack chooses.
    ///
    /// # Errors
    /// When two member graphs carry trust values from *different chains*. Ranks
    /// are only comparable within a declared chain (#66), so this refuses
    /// rather than returning a composed trust that means nothing. Freshness and
    /// policy would still be computable, but reporting a partial label as if it
    /// were the label is the kind of quiet half-truth this axis exists to stop.
    pub fn dataset_labels(&self, graphs: &[i64]) -> Result<DatasetLabels> {
        let mut fresh = Vec::with_capacity(graphs.len());
        let mut trust = Vec::with_capacity(graphs.len());
        let mut policy = Vec::with_capacity(graphs.len());

        for &g in graphs {
            let l = self.label_of_id(g)?;
            fresh.push(l.freshness.value);
            trust.push(l.trust.value);
            policy.push(l.policy.value);
        }

        Ok(DatasetLabels {
            freshness: crate::lattice::fold_meet(fresh)?,
            trust: crate::lattice::fold_meet(trust)?,
            policy: crate::lattice::fold_join(policy)?,
        })
    }

    /// Refuse the query if this dataset's label falls below a configured floor
    /// (quipu #68, graph-labels.md §5).
    ///
    /// **Unset floors are a no-op** — an unconfigured store returns `Ok` before
    /// reading a single label, so there is no query-path cost and no behaviour
    /// change whatsoever.
    ///
    /// The refusal **names the graph that dragged the label down**, mirroring
    /// `authority::refusal`, which names the narrowest link in a chain. "Which
    /// layer made this answer untrustworthy" is then a one-line answer instead
    /// of a manual walk over the dataset. The check therefore runs **per
    /// member** rather than over the composed label: the fold tells you the
    /// dataset is stale, but only the members tell you *which one*.
    ///
    /// **Undeclared fails a configured floor.** Fail-safe at enforcement,
    /// honest at reporting (§2.1): `label_of` still reports *undeclared* rather
    /// than inventing a value, and a floor treats that absence as a failure
    /// rather than as permission.
    ///
    /// ⚠️ This is **not access control** (§11). It refuses a query; it does not
    /// hide rows, and a caller naming a graph directly still reads it.
    ///
    /// # Errors
    /// [`Error::PolicyDenied`] naming the offending graph and the failing axis.
    pub fn check_label_floor(&self, graphs: &[i64]) -> Result<()> {
        let floor = self.labels_config();
        if floor.is_unset() {
            return Ok(());
        }

        let min_fresh = match floor.min_freshness.as_deref() {
            Some(s) => Some(Freshness::parse(s).ok_or_else(|| {
                Error::InvalidValue(format!(
                    "[quipu.labels] min_freshness = '{s}' is not a freshness value \
                     (fresh|recomputing|stale)"
                ))
            })?),
            None => None,
        };
        if floor.min_trust_rank.is_some() && floor.min_trust_chain.is_none() {
            return Err(Error::InvalidValue(
                "[quipu.labels] min_trust_rank is set without min_trust_chain. A rank \
                 means nothing outside the chain that declared it — say which chain the \
                 floor is expressed in, or ranks from an unrelated vocabulary would be \
                 compared as bare integers."
                    .into(),
            ));
        }

        for &g in graphs {
            let iri = self.resolve(g).unwrap_or_else(|_| format!("g={g}"));
            let l = self.label_of_id(g)?;

            if let Some(min) = min_fresh {
                match l.freshness.value {
                    Some(f) if f >= min => {}
                    Some(f) => {
                        return Err(Error::PolicyDenied(format!(
                            "query refused: graph '{iri}' is '{f}', below the configured \
                             freshness floor '{min}'. Labels are not access control — this \
                             refuses the query, it does not hide rows."
                        )));
                    }
                    None => {
                        return Err(Error::PolicyDenied(format!(
                            "query refused: graph '{iri}' declares no freshness, and a \
                             freshness floor of '{min}' is configured. Undeclared fails a \
                             floor — fail-safe at enforcement, rather than reading silence \
                             as '{min}'."
                        )));
                    }
                }
            }

            if let (Some(min_rank), Some(min_chain)) =
                (floor.min_trust_rank, floor.min_trust_chain.as_deref())
            {
                match &l.trust.value {
                    Some(t) if t.chain != min_chain => {
                        return Err(Error::PolicyDenied(format!(
                            "query refused: graph '{iri}' carries trust '{}' ranked in chain \
                             '{}', but the configured floor is expressed in chain \
                             '{min_chain}'. Ranks are not comparable across chains, so this \
                             cannot be evaluated rather than being evaluated wrongly.",
                            t.iri, t.chain
                        )));
                    }
                    Some(t) if t.rank < min_rank => {
                        return Err(Error::PolicyDenied(format!(
                            "query refused: graph '{iri}' carries trust '{}' at rank {} in \
                             chain '{min_chain}', below the configured floor of {min_rank}.",
                            t.iri, t.rank
                        )));
                    }
                    Some(_) => {}
                    None => {
                        return Err(Error::PolicyDenied(format!(
                            "query refused: graph '{iri}' declares no trust, and a trust floor \
                             of {min_rank} in chain '{min_chain}' is configured. Undeclared \
                             fails a floor."
                        )));
                    }
                }
            }

            if !floor.deny_policy_tokens.is_empty()
                && let Some(p) = &l.policy.value
                && let Some(tok) = floor
                    .deny_policy_tokens
                    .iter()
                    .find(|t| p.contains(t.as_str()))
            {
                return Err(Error::PolicyDenied(format!(
                    "query refused: graph '{iri}' carries the denied policy token '{tok}'. \
                     Obligations compose by union, so one restricted graph taints the whole \
                     dataset."
                )));
            }
        }

        Ok(())
    }

    /// Every named graph's id (`g <> 0`, excluding the reserved meta-graph).
    ///
    /// The meta-graph is excluded deliberately: it holds labels *about* graphs,
    /// so folding its own label into a dataset that merely ranges over "all
    /// named graphs" would be a category error.
    pub fn all_named_graph_ids(&self) -> Result<Vec<i64>> {
        let meta_g = self.meta_graph_id()?;
        let mut stmt = self
            .conn
            .prepare("SELECT g FROM graphs WHERE g <> 0 AND g <> ?1")?;
        let ids = stmt
            .query_map(params![meta_g], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Recompute every graph's label from the meta-graph facts and report where
    /// the cache disagrees. The source of truth is RDF; a non-empty result
    /// means the cache is wrong, never the other way round.
    pub fn graph_label_drift(&self) -> Result<Vec<LabelDrift>> {
        let mut drift = Vec::new();
        let meta_g = self.meta_graph_id()?;

        let mut stmt = self
            .conn
            .prepare("SELECT g, fresh_rank, trust_chain, policy FROM graphs WHERE g <> ?1")?;
        let rows: Vec<DriftRow> = stmt
            .query_map(params![meta_g], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<std::result::Result<_, _>>()?;

        for (g, fresh_rank, trust_chain, policy) in rows {
            let graph_iri = self.resolve(g).unwrap_or_else(|_| format!("g={g}"));

            // Freshness.
            let rdf_fresh = self.declared_str(g, QUIPU_FRESHNESS, meta_g)?;
            let cached_fresh = match fresh_rank {
                Some(0) => Some("stale".to_string()),
                Some(1) => Some("recomputing".to_string()),
                Some(2) => Some("fresh".to_string()),
                Some(other) => Some(format!("<invalid {other}>")),
                None => None,
            };
            if rdf_fresh != cached_fresh {
                drift.push(LabelDrift {
                    graph_iri: graph_iri.clone(),
                    axis: "freshness",
                    rdf: rdf_fresh.unwrap_or_else(|| "(undeclared)".into()),
                    cached: cached_fresh.unwrap_or_else(|| "(undeclared)".into()),
                });
            }

            // Trust chain.
            let rdf_chain = match self.declared_trust_iri(g)? {
                Some(iri) => self.chain_of(&iri, meta_g)?,
                None => None,
            };
            if rdf_chain != trust_chain {
                drift.push(LabelDrift {
                    graph_iri: graph_iri.clone(),
                    axis: "trust",
                    rdf: rdf_chain.unwrap_or_else(|| "(undeclared)".into()),
                    cached: trust_chain.unwrap_or_else(|| "(undeclared)".into()),
                });
            }

            // Policy — compare as SETS, since the cache is a canonical join and
            // a string compare would report a false drift on reordering.
            let rdf_tokens = self.declared_all(g, QUIPU_POLICY_CLASS, meta_g)?;
            let rdf_policy = if rdf_tokens.is_empty() {
                None
            } else {
                Some(PolicyClass::new(rdf_tokens))
            };
            let cached_policy = policy.as_deref().map(decode_policy);
            if rdf_policy != cached_policy {
                drift.push(LabelDrift {
                    graph_iri,
                    axis: "policy",
                    rdf: rdf_policy.map_or_else(|| "(undeclared)".into(), |p| p.to_string()),
                    cached: cached_policy.map_or_else(|| "(undeclared)".into(), |p| p.to_string()),
                });
            }
        }

        Ok(drift)
    }

    /// The single current string value of `(e, predicate)` in `graph`.
    fn declared_str(&self, e: i64, predicate: &str, graph: i64) -> Result<Option<String>> {
        Ok(match self.current_value(e, predicate, graph)? {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        })
    }

    /// Every current string value of `(e, predicate)` in `graph`.
    fn declared_all(&self, e: i64, predicate: &str, graph: i64) -> Result<Vec<String>> {
        Ok(self
            .current_values(e, predicate, graph)?
            .into_iter()
            .filter_map(|v| match v {
                Value::Str(s) => Some(s),
                _ => None,
            })
            .collect())
    }

    /// The chain IRI currently ranking `trust_iri`.
    fn chain_of(&self, trust_iri: &str, meta_g: i64) -> Result<Option<String>> {
        let Some(term) = self.lookup(trust_iri)? else {
            return Ok(None);
        };
        let v = self.current_value(term, QUIPU_IN_CHAIN, meta_g)?;
        self.ref_iri(v.as_ref())
    }
}

#[cfg(test)]
#[path = "labels_tests.rs"]
mod tests;
