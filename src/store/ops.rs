//! Store write and read operations: transact, query, retract, time-travel.

use rusqlite::params;

use crate::embedding;
use crate::error::{Error, Result};
use crate::types::{Fact, Op, Value};

use super::{AsOf, Datum, Store};

/// What episode-scoped retraction does when removing an episode's facts would
/// strip a node's IDENTITY (`rdfs:label` / `rdf:type`) while leaving it
/// referenced by facts from other episodes — a GHOST NODE (aegis-arup).
///
/// A ghost is present (answers predicate queries, holds edges, inflates counts)
/// and absent (unnameable, untypeable) at the same time: it is invisible to the
/// label scan and `rdf:type` query that the read path and every agent-facing
/// skill use for discovery. Before this policy existed, retraction was purely
/// tx-source-scoped and identity triples were ordinary facts, so
/// `{"retracted": N}` looked identical for a clean cleanup and a mutilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrphanPolicy {
    /// DEFAULT. Keep the episode's `rdfs:label` / `rdf:type` facts alive for any
    /// entity that still has surviving references after the retraction. A node
    /// that survives keeps its name and type; a node with nothing left is
    /// removed whole, identity included. Reported in
    /// [`RetractEpisodeOutcome::preserved_identity`].
    #[default]
    Preserve,
    /// Refuse the whole retraction (no writes) if it would orphan any identity.
    /// For callers that want retract-then-repost to be an explicit decision.
    Refuse,
    /// Legacy strict episode-scope: retract identity triples too, creating the
    /// ghosts. Still reported in [`RetractEpisodeOutcome::orphans`] so the
    /// caller can repost — the API can no longer stay silent about it.
    Allow,
}

impl OrphanPolicy {
    /// Parse a policy name (`preserve` | `refuse` | `allow`). Case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "preserve" => Some(Self::Preserve),
            "refuse" => Some(Self::Refuse),
            "allow" => Some(Self::Allow),
            _ => None,
        }
    }

    /// The policy's canonical name, as accepted by [`OrphanPolicy::parse`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Refuse => "refuse",
            Self::Allow => "allow",
        }
    }
}

/// An entity whose identity the retraction would strip while leaving it
/// referenced elsewhere in the graph (aegis-arup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityOrphan {
    /// The entity term id.
    pub entity: i64,
    /// The episode declared this entity's only `rdfs:label`.
    pub lost_label: bool,
    /// The episode declared this entity's only `rdf:type`.
    pub lost_type: bool,
}

/// Result of an episode-scoped retraction.
#[derive(Debug, Clone)]
pub struct RetractEpisodeOutcome {
    /// The retraction transaction, or [`crate::episode::NOOP_TX`] when nothing
    /// was written.
    pub tx_id: i64,
    /// Facts actually closed by this retraction.
    pub retracted: Vec<Fact>,
    /// Identity facts deliberately left ACTIVE under
    /// [`OrphanPolicy::Preserve`], so their entities stay findable.
    pub preserved_identity: Vec<Fact>,
    /// Entities whose identity was at risk. Under `Preserve` these were saved
    /// (see `preserved_identity`); under `Allow` they are now real ghosts.
    pub orphans: Vec<IdentityOrphan>,
    /// The policy that was applied.
    pub policy: OrphanPolicy,
}

impl Store {
    // -- Write path --

    /// Atomically write a batch of datums in a single transaction.
    /// Returns the transaction id.
    pub fn transact(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
    ) -> Result<i64> {
        // Default write target is the ROOT graph (g=0) — the source of truth.
        // Named-graph overlays use transact_to_graph (aegis-g1al / quipu #36).
        self.transact_to_graph(datums, timestamp, actor, source, 0)
    }

    /// Write a batch to a specific named graph. g=0 is ROOT. Idempotency and
    /// retraction are SCOPED to `graph`: writing to an overlay never skips a
    /// triple because ROOT has it, and never closes a ROOT fact's valid_to. The
    /// same (e,a,v) coexists across graphs, and an overlay's retract closes only
    /// its OWN assertions — the base stays un-mutated (Stiwi's #36 requirement,
    /// enforced in SQL, not convention). Cross-graph SHADOWING (an overlay
    /// hiding a ROOT fact in the composed view) is a read-time resolution over
    /// the ordered dataset stack, handled by the query path — not a base write.
    pub fn transact_to_graph(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
        graph: i64,
    ) -> Result<i64> {
        // Use savepoint (not transaction) so transact() can nest inside
        // speculate()'s outer savepoint.
        let sp = self.conn.savepoint()?;
        sp.execute(
            "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
            params![timestamp, actor, source],
        )?;
        let tx_id = sp.last_insert_rowid();

        // Track which datums were actually written (for observer notification).
        #[cfg(feature = "reactive-reasoner")]
        let mut written_datums: Vec<&Datum> = Vec::with_capacity(datums.len());

        {
            let mut insert = sp.prepare(
                "INSERT INTO facts (e, a, v, g, tx, valid_from, valid_to, op) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            // Retraction is SCOPED to `graph`: an overlay closing an assertion
            // touches only its own graph, never ROOT (base un-mutated, #36).
            let mut close_assertion = sp.prepare(
                "UPDATE facts SET valid_to = ?1 \
                 WHERE e = ?2 AND a = ?3 AND v = ?4 AND g = ?5 AND op = 1 AND valid_to IS NULL",
            )?;
            // Idempotent assertions: skip if an active fact with the same
            // (e, a, v) already exists IN THIS GRAPH. Scoped to `graph` so the
            // same triple can be asserted independently into ROOT and overlays.
            let mut check_exists = sp.prepare(
                "SELECT 1 FROM facts \
                 WHERE e = ?1 AND a = ?2 AND v = ?3 AND g = ?4 AND op = 1 AND valid_to IS NULL \
                 LIMIT 1",
            )?;
            for d in datums {
                let v_bytes = d.value.to_bytes();
                if d.op == Op::Retract {
                    close_assertion
                        .execute(params![timestamp, d.entity, d.attribute, v_bytes, graph])?;
                } else {
                    // Skip assertion if an identical active fact already exists
                    // IN THIS GRAPH.
                    let exists: bool = check_exists
                        .query_row(params![d.entity, d.attribute, v_bytes, graph], |_| Ok(true))
                        .unwrap_or(false);
                    if exists {
                        continue;
                    }
                }
                insert.execute(params![
                    d.entity,
                    d.attribute,
                    v_bytes,
                    graph,
                    tx_id,
                    d.valid_from,
                    d.valid_to,
                    d.op as i32,
                ])?;
                #[cfg(feature = "reactive-reasoner")]
                written_datums.push(d);
            }
        }

        sp.commit()?;

        // Post-transact hook: auto-embed touched entities.
        // Skipped when a vector search delegate is set — embeddings belong in
        // the external index layer (e.g. Bobbin's LanceDB).
        if self.embedding_config.auto_embed
            && self.vector_delegate.is_none()
            && let Some(provider) = &self.embedding_provider
        {
            let entity_ids = embedding::touched_entity_ids(datums);
            embedding::auto_embed_entities(
                self,
                provider,
                &entity_ids,
                timestamp,
                self.embedding_config.embed_batch_size,
                datums,
            )?;
        }

        // Notify registered observers. Observers may call store.transact()
        // directly for per-rule provenance. Cloning the Arc vec first
        // avoids a borrow conflict with &mut self.
        #[cfg(feature = "reactive-reasoner")]
        {
            use super::Delta;

            if !self.observers.is_empty() {
                let mut asserts = Vec::new();
                let mut retracts = Vec::new();
                // Only notify observers about datums that were actually written.
                for d in &written_datums {
                    match d.op {
                        Op::Assert => asserts.push(d.clone()),
                        Op::Retract => retracts.push(d.clone()),
                        // Overlay view-markers are transient/overlay-class and
                        // are excluded from the committed reactive stream (#36).
                        Op::Tombstone => {}
                    }
                }
                let delta = Delta {
                    tx: tx_id,
                    asserts,
                    retracts,
                    source: source.map(String::from),
                };

                let observers: Vec<_> = self.observers.clone();
                for obs in &observers {
                    obs.after_commit(self, &delta)?;
                }
            }
        }

        Ok(tx_id)
    }

    /// Execute `f` against a speculative fork of the store with `hypothetical`
    /// datums applied. The fork is discarded after `f` returns — the underlying
    /// store is never mutated. Useful for counterfactual impact analysis
    /// ("what would break if I removed this?").
    ///
    /// Implementation: wraps the hypothetical write and query in a `SQLite`
    /// `SAVEPOINT` that is always rolled back. Because [`transact`](Store::transact)
    /// uses nested savepoints, observers (including the reactive reasoner) fire
    /// normally inside the speculative fork, so derived facts are recomputed.
    pub fn speculate<F, R>(&mut self, hypothetical: &[Datum], timestamp: &str, f: F) -> Result<R>
    where
        F: FnOnce(&Store) -> Result<R>,
    {
        self.conn.execute_batch("SAVEPOINT speculate")?;

        let result = self
            .transact(
                hypothetical,
                timestamp,
                Some("speculate"),
                Some("speculate"),
            )
            .and_then(|_| f(self));

        // Always rollback: speculative state must not persist.
        match self
            .conn
            .execute_batch("ROLLBACK TO speculate; RELEASE speculate")
        {
            Ok(()) => result,
            Err(rollback_err) => match result {
                Err(e) => Err(e),
                Ok(_) => Err(rollback_err.into()),
            },
        }
    }

    /// Retract all current facts for an entity (or just those matching a predicate).
    /// Returns `(tx_id, count)`.
    pub fn retract_entity(
        &mut self,
        entity: i64,
        predicate: Option<i64>,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<(i64, usize)> {
        self.retract_triples(entity, predicate, None, timestamp, actor, false)
    }

    /// Retract current facts for an entity, narrowed by predicate and/or value.
    ///
    /// With all three of entity, predicate and value supplied this is
    /// TRIPLE-LEVEL retraction: exactly one `(e, a, v)` statement is closed
    /// (aegis-arup ask 1). Episode granularity was previously the finest handle
    /// available, so removing two stray edges meant retracting the whole episode
    /// — 33 statements for a 2-statement target, a 16x blast radius, and the
    /// rebuild that follows is what put node identity at risk in the first
    /// place. Precision here removes the reason to reach for the blunt tool.
    ///
    /// Returns `(tx_id, count)`; `(0, 0)` when nothing matched.
    pub fn retract_triples(
        &mut self,
        entity: i64,
        predicate: Option<i64>,
        value: Option<&Value>,
        timestamp: &str,
        actor: Option<&str>,
        allow_orphan: bool,
    ) -> Result<(i64, usize)> {
        let facts: Vec<Fact> = self
            .entity_facts(entity)?
            .into_iter()
            .filter(|f| predicate.is_none_or(|p| f.attribute == p))
            .filter(|f| value.is_none_or(|v| &f.value == v))
            .collect();

        if facts.is_empty() {
            return Ok((0, 0));
        }

        // Refuse to strip an entity's LAST rdf:type while it keeps other facts —
        // that is a HALF-GHOST: label, comments and edges survive, so the node
        // stays visible to label scans and disappears from every `?s a ?t`
        // query. /episode/retract has had an orphan policy since aegis-arup;
        // /retract is the sharper instrument and had none, so the blunt tool was
        // guarded and the precise one was not. A real node (goldblum-repo) was
        // left typeless this way while the call reported success (aegis-a0ne).
        //
        // Only fires when the entity SURVIVES: retracting an entity whole
        // (predicate = None) removes identity and references together, which
        // leaves no ghost and stays allowed.
        if !allow_orphan {
            let type_id = self.intern(crate::namespace::RDF_TYPE)?;
            let all = self.entity_facts(entity)?;
            let types_before = all.iter().filter(|f| f.attribute == type_id).count();
            let types_going = facts.iter().filter(|f| f.attribute == type_id).count();
            let survivors = all.len() - facts.len();
            if types_before > 0 && types_going == types_before && survivors > 0 {
                return Err(Error::InvalidValue(format!(
                    "refusing to retract entity {entity}'s last rdf:type while {survivors} other \
                     fact(s) survive — this would leave a typeless node that label scans still \
                     find and type queries cannot. Retract the whole entity, assert the \
                     replacement type first, or pass allow_orphan to override deliberately."
                )));
            }
        }

        let datums = retraction_datums(&facts);
        let count = datums.len();
        let tx_id = self.transact(&datums, timestamp, actor, Some("retract"))?;
        Ok((tx_id, count))
    }

    /// Episode-scoped logical retraction (aegis-hxb).
    ///
    /// Retract every currently-active asserted fact whose owning transaction was
    /// the ingest of the named episode. This is a *logical* retraction via the
    /// existing bitemporal [`Op::Retract`]/`valid_to` close path — facts are
    /// never physically deleted, so time-travel queries (`/cord`, `facts_as_of`,
    /// `entity_history`) still show them, now closed.
    ///
    /// **Provenance unit = the episode's transaction(s).** Every episode write
    /// goes through one [`transact`](Store::transact) stamped
    /// `source = "episode:{name}"` (see `episode::ingest_episode` →
    /// `rdf::ingest_rdf`). Selecting active facts by that transaction source is
    /// *complete* — it covers the episode activity node, its generated entities,
    /// the bare relationship triples (edges), and any reified confidence
    /// statements — whereas `prov:wasGeneratedBy` only links entity nodes.
    ///
    /// **Shared-IRI safety.** Idempotent assertion (see `transact`) means each
    /// active `(e, a, v)` belongs to exactly one owning transaction. Retracting
    /// episode X therefore closes only the facts X's transaction actually wrote;
    /// facts about shared entities (e.g. `quipu-server`, `kota`) asserted by
    /// other episodes or write paths stay active. Re-ingests of the same episode
    /// produce multiple transactions that all share the source tag, so every one
    /// of the episode's currently-active contributions is caught.
    ///
    /// **Identity is not collateral (aegis-arup).** Episode scope is not
    /// attribute-aware — `rdfs:label` and `rdf:type` are ordinary facts — so a
    /// naive scope-only retraction strips the name and type off any node whose
    /// identity this episode declared, while edges from other episodes keep it
    /// alive: a GHOST, present to predicate queries and invisible to every label
    /// scan and type query. This path therefore applies
    /// [`OrphanPolicy::Preserve`] by default: identity facts are kept ACTIVE for
    /// entities that retain surviving references, and removed with the rest for
    /// entities that leave the graph entirely. Use
    /// [`retract_episode_with_policy`](Store::retract_episode_with_policy) to
    /// pick `Refuse` or the legacy `Allow`, and to see which entities were
    /// affected.
    ///
    /// **Idempotent.** Retracting an episode with no active facts (already
    /// retracted, or never ingested) is a no-op: returns `(NOOP_TX, [])`. A
    /// second retraction after a preserving one is also a no-op — the only facts
    /// left in scope are the preserved identity triples, which are preserved
    /// again.
    ///
    /// Returns `(tx_id, retracted_facts)`. `tx_id` is the retraction
    /// transaction, or [`crate::episode::NOOP_TX`] when nothing was retracted.
    pub fn retract_episode(
        &mut self,
        episode_name: &str,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<(i64, Vec<Fact>)> {
        let outcome = self.retract_episode_with_policy(
            episode_name,
            timestamp,
            actor,
            OrphanPolicy::default(),
        )?;
        Ok((outcome.tx_id, outcome.retracted))
    }

    /// [`retract_episode`](Store::retract_episode) with an explicit ghost-node
    /// policy and a full report of what the retraction did to node identity
    /// (aegis-arup).
    ///
    /// Episode scope alone is *not* attribute-aware: `rdfs:label` and `rdf:type`
    /// are ordinary facts, so retracting the episode that declared a node's
    /// identity strips its name and type while edges asserted by OTHER episodes
    /// survive — leaving a node that answers predicate queries but is invisible
    /// to every label scan and type query the read path uses. See
    /// [`OrphanPolicy`] for the three contracts and which one is the default.
    ///
    /// Whatever the policy, the outcome names the affected entities: the API can
    /// no longer return an identical `{"retracted": N}` for a clean cleanup and
    /// a mutilation.
    pub fn retract_episode_with_policy(
        &mut self,
        episode_name: &str,
        timestamp: &str,
        actor: Option<&str>,
        policy: OrphanPolicy,
    ) -> Result<RetractEpisodeOutcome> {
        let source_tag = format!("episode:{episode_name}");

        // Currently-active asserted facts written by this episode's
        // transaction(s). Join keeps it precise: a fact is in scope only if its
        // own owning tx carried the episode source tag.
        let facts = {
            let mut stmt = self.conn.prepare(
                "SELECT f.e, f.a, f.v, f.tx, f.valid_from, f.valid_to, f.op \
                 FROM facts f JOIN transactions t ON f.tx = t.id \
                 WHERE t.source = ?1 AND f.op = 1 AND f.valid_to IS NULL \
                 ORDER BY f.e, f.a",
            )?;
            Self::collect_facts(&mut stmt, params![source_tag])?
        };

        if facts.is_empty() {
            return Ok(RetractEpisodeOutcome {
                tx_id: crate::episode::NOOP_TX,
                retracted: Vec::new(),
                preserved_identity: Vec::new(),
                orphans: Vec::new(),
                policy,
            });
        }

        let orphans = self.identity_orphans(&facts, &source_tag)?;

        if policy == OrphanPolicy::Refuse && !orphans.is_empty() {
            let mut names: Vec<String> = orphans
                .iter()
                .map(|o| {
                    let iri = self
                        .resolve(o.entity)
                        .unwrap_or_else(|_| o.entity.to_string());
                    let missing = match (o.lost_label, o.lost_type) {
                        (true, true) => "rdfs:label + rdf:type",
                        (true, false) => "rdfs:label",
                        _ => "rdf:type",
                    };
                    format!("{iri} (loses {missing})")
                })
                .collect();
            names.sort();
            return Err(crate::error::Error::InvalidValue(format!(
                "retracting episode '{episode_name}' would orphan the identity of {} node(s) that \
                 keep surviving references — they would become unlabelled/untyped ghosts, \
                 invisible to label scans and rdf:type queries: {}. Re-run with \
                 on_orphan=preserve to keep their identity, or on_orphan=allow to accept the \
                 ghosts (and re-post identity yourself).",
                orphans.len(),
                names.join(", ")
            )));
        }

        // Under Preserve, identity facts for at-risk entities stay ACTIVE: a node
        // that survives the retraction keeps its name and type. A node with no
        // surviving references is not at risk and is removed whole.
        let preserved_identity: Vec<Fact> = if policy == OrphanPolicy::Preserve {
            let (label_id, type_id) = self.identity_predicate_ids()?;
            facts
                .iter()
                .filter(|f| {
                    orphans.iter().any(|o| {
                        o.entity == f.entity
                            && ((o.lost_label && Some(f.attribute) == label_id)
                                || (o.lost_type && Some(f.attribute) == type_id))
                    })
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let retracted: Vec<Fact> = facts
            .into_iter()
            .filter(|f| {
                !preserved_identity.iter().any(|p| {
                    p.entity == f.entity && p.attribute == f.attribute && p.value == f.value
                })
            })
            .collect();

        if retracted.is_empty() {
            return Ok(RetractEpisodeOutcome {
                tx_id: crate::episode::NOOP_TX,
                retracted,
                preserved_identity,
                orphans,
                policy,
            });
        }

        let datums: Vec<Datum> = retracted
            .iter()
            .map(|f| Datum {
                entity: f.entity,
                attribute: f.attribute,
                value: f.value.clone(),
                valid_from: f.valid_from.clone(),
                valid_to: None,
                op: Op::Retract,
            })
            .collect();

        // Stamp the retraction tx with a distinct source so it is traceable and
        // never re-selected as one of the episode's own assertions.
        let tx_id = self.transact(
            &datums,
            timestamp,
            actor,
            Some(&format!("retract-episode:{episode_name}")),
        )?;
        Ok(RetractEpisodeOutcome {
            tx_id,
            retracted,
            preserved_identity,
            orphans,
            policy,
        })
    }

    /// Term ids for `rdfs:label` and `rdf:type`, or `None` if never interned
    /// (in which case no fact can carry that predicate).
    fn identity_predicate_ids(&self) -> Result<(Option<i64>, Option<i64>)> {
        Ok((
            self.lookup(crate::namespace::RDFS_LABEL)?,
            self.lookup(crate::namespace::RDF_TYPE)?,
        ))
    }

    /// Entities that `in_scope` (an episode's active facts) would leave without
    /// a label and/or a type, while OTHER writes still reference them.
    ///
    /// "Still referenced" is deliberately broad — subject or object of any
    /// surviving fact. A node reachable by any edge must stay findable by name.
    fn identity_orphans(&self, in_scope: &[Fact], source_tag: &str) -> Result<Vec<IdentityOrphan>> {
        let (label_id, type_id) = self.identity_predicate_ids()?;
        if label_id.is_none() && type_id.is_none() {
            return Ok(Vec::new());
        }

        let mut entities: Vec<i64> = in_scope
            .iter()
            .filter(|f| Some(f.attribute) == label_id || Some(f.attribute) == type_id)
            .map(|f| f.entity)
            .collect();
        entities.sort_unstable();
        entities.dedup();

        let mut orphans = Vec::new();
        for entity in entities {
            // Would the entity still be referenced at all once this episode's
            // facts are gone? If not, it leaves the graph whole — not a ghost.
            if !self.has_surviving_reference(entity, source_tag)? {
                continue;
            }
            let declared_label = in_scope
                .iter()
                .any(|f| f.entity == entity && Some(f.attribute) == label_id);
            let declared_type = in_scope
                .iter()
                .any(|f| f.entity == entity && Some(f.attribute) == type_id);
            // Another episode may independently assert a label/type; then ours
            // is not the node's only identity and losing it orphans nothing.
            let lost_label =
                declared_label && !self.has_surviving_predicate(entity, label_id, source_tag)?;
            let lost_type =
                declared_type && !self.has_surviving_predicate(entity, type_id, source_tag)?;
            if lost_label || lost_type {
                orphans.push(IdentityOrphan {
                    entity,
                    lost_label,
                    lost_type,
                });
            }
        }
        Ok(orphans)
    }

    /// Is `entity` the subject or object of any active fact NOT written by
    /// `source_tag`'s transaction(s)?
    fn has_surviving_reference(&self, entity: i64, source_tag: &str) -> Result<bool> {
        let as_object = Value::Ref(entity).to_bytes();
        let mut stmt = self.conn.prepare(
            "SELECT 1 FROM facts f JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL \
               AND (f.e = ?1 OR f.v = ?2) \
               AND (t.source IS NULL OR t.source <> ?3) \
             LIMIT 1",
        )?;
        Ok(stmt.exists(params![entity, as_object, source_tag])?)
    }

    /// Does `entity` carry an active `predicate` fact from outside `source_tag`?
    fn has_surviving_predicate(
        &self,
        entity: i64,
        predicate: Option<i64>,
        source_tag: &str,
    ) -> Result<bool> {
        let Some(predicate) = predicate else {
            return Ok(false);
        };
        let mut stmt = self.conn.prepare(
            "SELECT 1 FROM facts f JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL \
               AND f.e = ?1 AND f.a = ?2 \
               AND (t.source IS NULL OR t.source <> ?3) \
             LIMIT 1",
        )?;
        Ok(stmt.exists(params![entity, predicate, source_tag])?)
    }

    // -- Read path --

    /// Return the current state: all asserted facts not yet retracted.
    pub fn current_facts(&self) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE op = 1 AND valid_to IS NULL \
             ORDER BY e, a",
        )?;
        Self::collect_facts(&mut stmt, params![])
    }

    /// Return facts for a specific entity (current state).
    pub fn entity_facts(&self, entity: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 AND op = 1 AND valid_to IS NULL \
             ORDER BY a",
        )?;
        Self::collect_facts(&mut stmt, params![entity])
    }

    /// Time-travel query: return facts as they were at a given point.
    pub fn facts_as_of(&self, as_of: &AsOf) -> Result<Vec<Fact>> {
        let mut sql =
            String::from("SELECT e, a, v, tx, valid_from, valid_to, op FROM facts WHERE op = 1");
        if as_of.tx.is_some() {
            sql.push_str(" AND tx <= ?1");
        }
        if as_of.valid_at.is_some() {
            let param_idx = if as_of.tx.is_some() { "?2" } else { "?1" };
            sql.push_str(&format!(
                " AND valid_from <= {param_idx} AND (valid_to IS NULL OR valid_to > {param_idx})"
            ));
        }
        sql.push_str(" ORDER BY e, a");

        let mut stmt = self.conn.prepare(&sql)?;
        match (&as_of.tx, &as_of.valid_at) {
            (Some(tx), Some(vt)) => Self::collect_facts(&mut stmt, params![tx, vt]),
            (Some(tx), None) => Self::collect_facts(&mut stmt, params![tx]),
            (None, Some(vt)) => Self::collect_facts(&mut stmt, params![vt]),
            (None, None) => Self::collect_facts(&mut stmt, params![]),
        }
    }

    /// Detect contradictions: overlapping valid-time intervals for the same
    /// entity+attribute pair.
    pub fn detect_contradictions(&self, entity: i64, attribute: i64) -> Result<Vec<(Fact, Fact)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f1.e, f1.a, f1.v, f1.tx, f1.valid_from, f1.valid_to, f1.op, \
                    f2.e, f2.a, f2.v, f2.tx, f2.valid_from, f2.valid_to, f2.op \
             FROM facts f1 \
             JOIN facts f2 ON f1.e = f2.e AND f1.a = f2.a \
             WHERE f1.e = ?1 AND f1.a = ?2 \
               AND f1.op = 1 AND f2.op = 1 \
               AND f1.rowid < f2.rowid \
               AND f1.v != f2.v \
               AND f1.valid_from < COALESCE(f2.valid_to, '9999-12-31') \
               AND f2.valid_from < COALESCE(f1.valid_to, '9999-12-31')",
        )?;

        let mut pairs = Vec::new();
        let mut rows = stmt.query(params![entity, attribute])?;
        while let Some(row) = rows.next()? {
            let v1_bytes: Vec<u8> = row.get(2)?;
            let v2_bytes: Vec<u8> = row.get(9)?;
            let f1 = Fact {
                entity: row.get(0)?,
                attribute: row.get(1)?,
                value: Value::from_bytes(&v1_bytes)?,
                tx: row.get(3)?,
                valid_from: row.get(4)?,
                valid_to: row.get(5)?,
                op: Op::Assert,
            };
            let f2 = Fact {
                entity: row.get(7)?,
                attribute: row.get(8)?,
                value: Value::from_bytes(&v2_bytes)?,
                tx: row.get(10)?,
                valid_from: row.get(11)?,
                valid_to: row.get(12)?,
                op: Op::Assert,
            };
            pairs.push((f1, f2));
        }
        Ok(pairs)
    }

    /// Return the full history of an entity: all facts (asserts + retracts) ordered by tx.
    pub fn entity_history(&self, entity: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 \
             ORDER BY tx, a",
        )?;
        Self::collect_facts(&mut stmt, params![entity])
    }

    /// List all transactions ordered by id.
    pub fn list_transactions(&self) -> Result<Vec<crate::types::Transaction>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, timestamp, actor, source FROM transactions ORDER BY id")?;
        let mut txns = Vec::new();
        let mut rows = stmt.query(params![])?;
        while let Some(row) = rows.next()? {
            txns.push(crate::types::Transaction {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                actor: row.get(2)?,
                source: row.get(3)?,
            });
        }
        Ok(txns)
    }

    /// Return the full history of a specific entity+attribute pair.
    pub fn attribute_history(&self, entity: i64, attribute: i64) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
             WHERE e = ?1 AND a = ?2 \
             ORDER BY tx",
        )?;
        Self::collect_facts(&mut stmt, params![entity, attribute])
    }

    // -- Internal --

    pub(crate) fn collect_facts(
        stmt: &mut rusqlite::Statement<'_>,
        params: impl rusqlite::Params,
    ) -> Result<Vec<Fact>> {
        let mut facts = Vec::new();
        let mut rows = stmt.query(params)?;
        while let Some(row) = rows.next()? {
            let v_bytes: Vec<u8> = row.get(2)?;
            let op_raw: i32 = row.get(6)?;
            facts.push(Fact {
                entity: row.get(0)?,
                attribute: row.get(1)?,
                value: Value::from_bytes(&v_bytes)?,
                tx: row.get(3)?,
                valid_from: row.get(4)?,
                valid_to: row.get(5)?,
                op: Op::from_i32(op_raw).unwrap_or(Op::Assert),
            });
        }
        Ok(facts)
    }
}

/// Build retraction datums for `facts`, at most ONE per distinct `(e, a, v)`.
///
/// The store is append-only, so one logical triple can be backed by MANY fact
/// rows — every re-assert from a different transaction adds another. `facts` is
/// therefore NOT deduplicated, and mapping it 1:1 onto retraction datums put
/// several rows with the same `(e, a, v)` into a single transaction. That
/// violates `facts`' PRIMARY KEY `(e, a, v, tx)`, so retraction failed outright
/// with a UNIQUE constraint error — on exactly the entities most worth
/// retracting, because being old and re-asserted is what creates the duplicates.
///
/// Measured on the live graph (aegis-a0ne), counting only what `entity_facts`
/// actually returns (`op = 1 AND valid_to IS NULL`): 86 duplicate groups over
/// 29 of 972 entities (3.0%), worst single group 132 rows. The affected set is
/// the graph's most-referenced nodes — graphiti (+273 redundant rows), kota
/// (+123), bobbin, traefik, dolt — i.e. retraction failed precisely on the
/// entities most likely to need correcting. `luvu` carried 29 rows of one
/// `rdf:type` value, 2 of another, and 32 `rdfs:label` rows.
///
/// Retracting a triple once is also the correct SEMANTICS — the duplicate rows
/// are one statement, not N — so this both fixes the crash and states the
/// intent. Dedupe is linear because `Value` is only `PartialEq`; per-entity
/// fact counts are small.
fn retraction_datums(facts: &[Fact]) -> Vec<Datum> {
    let mut datums: Vec<Datum> = Vec::with_capacity(facts.len());
    for f in facts {
        let already = datums
            .iter()
            .any(|d| d.entity == f.entity && d.attribute == f.attribute && d.value == f.value);
        if already {
            continue;
        }
        datums.push(Datum {
            entity: f.entity,
            attribute: f.attribute,
            value: f.value.clone(),
            valid_from: f.valid_from.clone(),
            valid_to: None,
            op: Op::Retract,
        });
    }
    datums
}
