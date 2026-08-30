//! Store write and read operations: transact, query, retract, time-travel.

use rusqlite::params;

use crate::embedding;
use crate::error::{Error, Result};
use crate::types::{Fact, Op, Value};

use super::{Datum, Store};

pub use super::retraction::{IdentityOrphan, OrphanPolicy, RetractEpisodeOutcome};

/// Result of staging a transaction's rows into the open `quipu_transact`
/// savepoint, threaded to [`Store::after_commit_hooks`] after RELEASE.
struct Staged {
    tx_id: i64,
    /// The COMPLETE set of changes this write made to the current-fact view, or
    /// `None` when the write path cannot vouch that it is complete.
    ///
    /// `Some` lets the resident read model be maintained instead of rebuilt.
    /// It is `None` whenever something closed or added facts that are not in
    /// this list — OWL domain/range inference extends the batch, and
    /// functional-property supersede closes prior values with a bulk
    /// `UPDATE ... WHERE v != ?` that never enumerates what it hit. Neither is
    /// reconstructible from the caller's datums, so the honest answer is to say
    /// so and let the model rebuild.
    effective: Option<Vec<Datum>>,
    /// Datums actually asserted (skipping idempotent no-ops), for reactive
    /// observer notification.
    #[cfg(feature = "reactive-reasoner")]
    asserts: Vec<Datum>,
    /// Datums actually retracted, for reactive observer notification.
    #[cfg(feature = "reactive-reasoner")]
    retracts: Vec<Datum>,
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
    /// triple because ROOT has it, and never closes a ROOT fact's `valid_to`. The
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
        // Manual savepoint commands (not the RAII `Savepoint`) so `&self` stays
        // usable for the write-time policy guard AFTER the datums are staged but
        // BEFORE commit — the `&Store`-based SPARQL evaluator cannot run while a
        // `Savepoint` holds a mutable borrow of the connection. Nests inside
        // speculate()'s outer savepoint exactly as the RAII form did.
        // Attachment first, ahead even of authority: a write to an attached
        // graph is not a permission question, it is a category error — that
        // graph's facts live in a file this store does not own (quipu #75).
        // The inferred suffix is RESERVED (quipu-0b6): a companion graph holds
        // engine-derived entailments only, so an external write there could
        // forge reachability the reasoner never derived. Refused by SOURCE,
        // not by caller: only reasoner/materializer output, the plane's own
        // bookkeeping, and the one-time migration pass.
        self.assert_write_target_allowed(graph, source)?;
        // Authority first, before anything is staged: a write the chain may not
        // make should not reach the policy gate, the placement check, or the
        // fact table. SARC I5. This gate runs BEFORE the savepoint opens, so
        // its refusal is recorded directly — there is no rollback to outlive.
        if let Err(e) = self.enforce_graph_authority(graph) {
            let refusal = super::events::PendingRefusal {
                gate: "authority",
                reason: e.to_string(),
                refused_datums: datums.len(),
            };
            self.record_refusal(
                &refusal,
                &self.graph_iri_of(graph),
                actor,
                source,
                timestamp,
            );
            return Err(e);
        }
        // SUSPEND read-model consultation for the duration of this write rather
        // than dropping the model. The write-time policy guard runs INSIDE the
        // savepoint and queries the pending post-state, so it must see staged
        // rows — the model holds the pre-write state and would answer without
        // them, denying a write it had just satisfied
        // (deny_blocks_noncompliant_write). Suspending rather than dropping is
        // what lets the commit below MAINTAIN the model instead of forcing a
        // rebuild, and it means a rolled-back write cannot poison it either:
        // the model never observed the staged rows at all.
        self.set_write_in_progress(true);
        self.conn.execute_batch("SAVEPOINT quipu_transact")?;
        match self.stage_and_guard(datums, timestamp, actor, source, graph) {
            Ok(mut staged) => {
                let tx_id = staged.tx_id;
                // Taken before `staged` moves into after_commit_hooks below.
                let effective = staged.effective.take();
                self.conn.execute_batch("RELEASE quipu_transact")?;
                self.after_commit_hooks(datums, staged, timestamp, source)?;
                // A write that defined or amended a policy makes the cached
                // registry stale.
                self.invalidate_policy_registry_if_governance(datums)?;
                // Bring the resident model up to date: apply the change set when
                // the write vouched for it, drop when it could not (quipu-m9h).
                self.set_write_in_progress(false);
                self.maintain_read_model(graph, effective.as_deref(), tx_id);
                // Memory telemetry (memory telemetry): count the commit and sample RSS
                // so a burst-export spike is captured at the write that caused it.
                crate::metrics::metrics().observe_write(datums.len() as u64);
                // Q-VERDICT-PERSIST: outside the savepoint, so the accept case
                // and the denial case below record identically.
                self.flush_pending_verdicts(timestamp, actor);
                Ok(tx_id)
            }
            Err(e) => {
                // Undo the staged datums and the transaction row; the store is
                // left byte-identical to before this call (no partial mutation).
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_transact; RELEASE quipu_transact");
                // Nothing to invalidate: the model was suspended for the whole
                // write, so it never observed the rows this rollback discarded.
                // That is the structural version of the fix — the earlier one
                // dropped the model here to undo poisoning that could no longer
                // happen.
                self.set_write_in_progress(false);
                // Take the staged refusal BEFORE flushing verdicts: the flush
                // runs a nested transact whose own error path would otherwise
                // consume it under the verdict-writer's actor/source.
                let refusal = self.pending_refusal.take();
                // AFTER the rollback, deliberately. The verdict of a denial is
                // the one worth keeping — an accepted write leaves its own
                // evidence in the facts it wrote, a refused one leaves nothing.
                self.flush_pending_verdicts(timestamp, actor);
                // Same reason, same ordering, for the refusal event
                // (camayoc-0d3): recorded only after the rollback succeeded,
                // on the same connection, and never able to mask `e` —
                // `record_refusal` swallows its own failures. Refusals under
                // speculate() are skipped inside (actor/source "speculate").
                if let Some(r) = refusal {
                    self.record_refusal(&r, &self.graph_iri_of(graph), actor, source, timestamp);
                }
                Err(e)
            }
        }
    }

    /// Stage a transaction's rows into the open `quipu_transact` savepoint, then
    /// run the write-time policy guard against the pending post-state. Returns
    /// the tx id (and, under `reactive-reasoner`, the written datum indices) on
    /// success; any `Err` (including a policy denial) leaves the savepoint open
    /// for the caller to roll back.
    fn stage_and_guard(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
        graph: i64,
    ) -> Result<Staged> {
        self.conn.execute(
            "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
            params![timestamp, actor, source],
        )?;
        let tx_id = self.conn.last_insert_rowid();

        // Domain/range axioms are not merely a one-time migration at ontology
        // load. Apply them to every newly asserted edge so a range such as
        // `runs_on -> Host` stays true for facts ingested after the load
        // (aegis-qfncf). The inferred datums join this same transaction: guards,
        // events and reactive observers see the post-inference write atomically.
        #[cfg(feature = "owl")]
        let staged_datums = {
            let mut staged = datums.to_vec();
            staged.extend(self.owl_domain_range_inferences(datums, timestamp)?);
            staged
        };
        #[cfg(not(feature = "owl"))]
        let staged_datums = datums.to_vec();
        // Whether anything beyond the caller's datums entered this write.
        let inferred = staged_datums.len() != datums.len();

        // Collect the datums actually written (for observer notification),
        // classified by op. Overlay view-markers (Tombstone) are excluded from
        // the committed reactive stream (#36).
        #[cfg(feature = "reactive-reasoner")]
        let mut asserts: Vec<Datum> = Vec::new();
        #[cfg(feature = "reactive-reasoner")]
        let mut retracts: Vec<Datum> = Vec::new();

        // Datums ACTUALLY written (idempotent no-ops excluded), for the event
        // log (event-log P1). Unconditional — events are core, not a feature.
        let mut written_asserts: Vec<&Datum> = Vec::new();
        let mut written_retracts: Vec<&Datum> = Vec::new();

        // Functional-property supersede (aegis-7vn3b). MUST run before the insert
        // loop below: it closes the PRIOR value so the new one lands as the only
        // current fact. Run after, and both would be live and the OWL gate would
        // reject the write — which is exactly the bug, an ordinary update turned
        // into an HTTP 400.
        #[cfg(feature = "owl")]
        let superseded =
            self.supersede_functional_values(&staged_datums, timestamp, graph, tx_id)?;
        // Without `owl` there is no functional-property supersede at all, so
        // nothing outside the caller's datums can close a fact.
        #[cfg(not(feature = "owl"))]
        let superseded = 0usize;

        {
            let mut insert = self.conn.prepare(
                "INSERT INTO facts (e, a, v, g, tx, valid_from, valid_to, op) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            // Retraction is SCOPED to `graph`: an overlay closing an assertion
            // touches only its own graph, never ROOT (base un-mutated, #36).
            // quipu #83: `retracted_tx` records WHICH transaction closed the
            // fact. Without it `as_of_tx` cannot tell a fact that was live at N
            // from one retracted since, and silently under-reports.
            let mut close_assertion = self.conn.prepare(
                "UPDATE facts SET valid_to = ?1, retracted_tx = ?6 \
                 WHERE e = ?2 AND a = ?3 AND v = ?4 AND g = ?5 AND op = 1 AND valid_to IS NULL",
            )?;
            // Idempotent assertions: skip if an active fact with the same
            // (e, a, v) already exists IN THIS GRAPH. Scoped to `graph` so the
            // same triple can be asserted independently into ROOT and overlays.
            let mut check_exists = self.conn.prepare(
                "SELECT 1 FROM facts \
                 WHERE e = ?1 AND a = ?2 AND v = ?3 AND g = ?4 AND op = 1 AND valid_to IS NULL \
                 LIMIT 1",
            )?;
            for d in &staged_datums {
                let v_bytes = d.value.to_bytes();
                if d.op == Op::Retract {
                    close_assertion.execute(params![
                        timestamp,
                        d.entity,
                        d.attribute,
                        v_bytes,
                        graph,
                        tx_id
                    ])?;
                    written_retracts.push(d);
                } else {
                    // Skip assertion if an identical active fact already exists
                    // IN THIS GRAPH.
                    let exists: bool = check_exists
                        .query_row(params![d.entity, d.attribute, v_bytes, graph], |_| Ok(true))
                        .unwrap_or(false);
                    if exists {
                        continue;
                    }
                    if d.op == Op::Assert {
                        written_asserts.push(d);
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
                match d.op {
                    Op::Assert => asserts.push(d.clone()),
                    Op::Retract => retracts.push(d.clone()),
                    Op::Tombstone => {}
                }
            }
        }

        // Definition-time placement check (SARC §4.2). Runs FIRST and only on
        // writes that define or amend a policy: a malformed constraint must be
        // refused before it can be evaluated, or the very next write is judged
        // by a rule whose class and enforcement point disagree.
        // Each gate's refusal is STASHED (not written) via `stash_refusal`:
        // this all runs inside the open savepoint, so the durable
        // `write.refused` event is recorded by the caller's error path after
        // the rollback (camayoc-0d3).
        self.validate_policy_placement(&staged_datums, graph)
            .map_err(|e| self.stash_refusal("placement", e, staged_datums.len()))?;

        // Transition-signature gate (quipu-8cc): a staged
        // `aegis:TransitionEvent` must carry a signature that verifies under
        // its performing agent's registered key, or the write rolls back.
        // Same contract and refusal plumbing as the placement check above.
        self.verify_transition_signatures(&staged_datums, graph)
            .map_err(|e| self.stash_refusal("transition", e, staged_datums.len()))?;

        // Write-time policy guard (the loom). Runs against the staged post-state
        // (same connection sees the open savepoint). A denial returns Err here
        // and the caller rolls the savepoint back — the write never commits.
        self.enforce_write_policies(&staged_datums, graph)
            .map_err(|e| self.stash_refusal("policy", e, staged_datums.len()))?;

        // OWL write-time constraints (aegis-bmqup): disjointWith and
        // FunctionalProperty. Same contract as the policy guard above — an Err
        // here rolls the savepoint back, so a violating write never commits.
        // Placed AFTER the policy gate so authority and governance still decide
        // first, and BEFORE emit_events so a rejected write emits nothing.
        #[cfg(feature = "owl")]
        self.enforce_owl_constraints(&staged_datums)
            .map_err(|e| self.stash_refusal("owl", e, staged_datums.len()))?;

        // Event log (event-log P1): append this tx's semantic events INSIDE the
        // savepoint, AFTER the policy guard — a denied write emits nothing, a
        // rolled-back write takes its events with it, and offset order equals
        // commit order because nothing else can commit between here and RELEASE.
        self.emit_events(
            &written_asserts,
            &written_retracts,
            tx_id,
            timestamp,
            source,
            graph,
        )?;

        // Only vouch for the change set when nothing outside it touched the
        // current-fact view. `superseded` counts functional-property closures;
        // `inferred` is true when OWL inference extended the batch.
        let effective = if superseded == 0 && !inferred {
            let mut all: Vec<Datum> =
                Vec::with_capacity(written_asserts.len() + written_retracts.len());
            all.extend(written_asserts.iter().map(|d| (*d).clone()));
            all.extend(written_retracts.iter().map(|d| (*d).clone()));
            Some(all)
        } else {
            None
        };

        Ok(Staged {
            tx_id,
            effective,
            #[cfg(feature = "reactive-reasoner")]
            asserts,
            #[cfg(feature = "reactive-reasoner")]
            retracts,
        })
    }

    /// Post-commit side effects: auto-embedding and reactive observers. Runs
    /// after the savepoint is released (the write is durable relative to the
    /// parent), preserving the previous ordering.
    fn after_commit_hooks(
        &mut self,
        datums: &[Datum],
        staged: Staged,
        timestamp: &str,
        source: Option<&str>,
    ) -> Result<()> {
        // Post-transact hook: auto-embed touched entities.
        // Skipped when a vector search delegate is set — embeddings belong in
        // the external index layer (e.g. Bobbin's LanceDB).
        if self.embedding_config.auto_embed
            && self.vector_delegate.is_none()
            && let Some(provider) = &self.embedding_provider
        {
            let entity_ids = embedding::touched_entity_ids(datums);
            if self.defer_auto_embed {
                // Deferred path: do only the cheap parts here —
                // close retired embeddings and build texts — and queue the
                // multi-second ONNX embed for the caller to run OUTSIDE the
                // store lock. An episode write's lock hold-time drops from
                // ~seconds (embed under lock) to the transact itself.
                let work = embedding::collect_embed_work(self, &entity_ids, timestamp, datums)?;
                if !work.is_empty() {
                    match &mut self.pending_embed {
                        Some(pending) => pending.merge(work),
                        None => self.pending_embed = Some(work),
                    }
                }
            } else {
                embedding::auto_embed_entities(
                    self,
                    provider,
                    &entity_ids,
                    timestamp,
                    self.embedding_config.embed_batch_size,
                    datums,
                )?;
            }
        }

        // Notify registered observers. Observers may call store.transact()
        // directly for per-rule provenance. Cloning the Arc vec first
        // avoids a borrow conflict with &mut self.
        #[cfg(feature = "reactive-reasoner")]
        {
            use super::Delta;

            if !self.observers.is_empty() {
                let delta = Delta {
                    tx: staged.tx_id,
                    asserts: staged.asserts,
                    retracts: staged.retracts,
                    source: source.map(String::from),
                };

                let observers: Vec<_> = self.observers.clone();
                for obs in &observers {
                    obs.after_commit(self, &delta)?;
                }
            }
        }

        // `staged` (its tx id / written indices) and `source` are only consumed
        // by the reactive-reasoner observer dispatch above.
        #[cfg(not(feature = "reactive-reasoner"))]
        let _ = (staged, source);

        Ok(())
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
            // A value was given but matched nothing. Distinguish an idempotent
            // no-op (the triple is genuinely absent) from the SILENT FOOTGUN
            // that froze re-parenting: a bare-string object for an IRI-valued
            // predicate. The write API turns a bare `"kprobe-b"` into
            // `Value::Str`, which can NEVER equal the stored `Value::Ref(kprobe-b)`,
            // so the edge survives and the call reports success with
            // `retracted: 0` — a refusal shaped exactly like a success. `0` here
            // means BOTH "nothing was there" and "you addressed it wrongly", which
            // are opposite facts (one a clean no-op, one your bug) the caller
            // cannot tell apart.
            //
            // Error on the two cases that are unambiguously the shape mistake, and
            // ONLY those, so genuine idempotent retraction (a correctly-shaped
            // value that is simply absent) still falls through to `Ok((0, 0))`:
            //   1. the predicate on this entity holds ONLY `Ref` objects — a `Str`
            //      could never have matched, so it is always a caller mistake;
            //   2. the predicate has NO current fact on this entity to compare
            //      against, but the bare string itself PARSES as an IRI (has a
            //      `scheme://`) — almost certainly a mis-shaped edge retract, e.g.
            //      `rdf:type` whose value was already retracted once. A value that
            //      is a plain literal (no scheme) stays a quiet no-op.
            // A predicate that stores string literals (holds a `Str`) is left
            // alone: a bare string is the RIGHT shape there, so absence is a real
            // idempotent no-op, not a mistake.
            if let (Some(Value::Str(s)), Some(p)) = (value, predicate) {
                let on_pred: Vec<Value> = self
                    .entity_facts(entity)?
                    .into_iter()
                    .filter(|f| f.attribute == p)
                    .map(|f| f.value)
                    .collect();
                let holds_ref = on_pred.iter().any(|v| matches!(v, Value::Ref(_)));
                let holds_str = on_pred.iter().any(|v| matches!(v, Value::Str(_)));
                // A bare token with a `scheme://` and no whitespace is an IRI a
                // caller forgot to wrap, not a literal.
                let looks_like_iri = s.contains("://") && !s.chars().any(char::is_whitespace);
                let ref_only = holds_ref && !holds_str;
                let absent_but_iri_shaped = on_pred.is_empty() && looks_like_iri;
                if ref_only || absent_but_iri_shaped {
                    let pred_iri = self.resolve(p)?;
                    return Err(Error::InvalidValue(format!(
                        "retract matched nothing: object \"{s}\" is a string literal, but \
                         <{pred_iri}> takes an IRI reference. Pass the object as \
                         {{\"iri\": \"{s}\"}} to retract the edge — a bare string is matched \
                         as a literal and can never equal a stored reference, so it silently \
                         retracts nothing."
                    )));
                }
            }
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

    /// Set `(entity, predicate)` to exactly `value`: retract every current
    /// object on that predicate and assert the new one, in ONE transaction
    /// — the ergonomic follow-up to the bare-string-object fix.
    ///
    /// Re-parenting as retract-then-assert is two calls the caller must
    /// sequence; forgetting the retract half leaves a MULTI-VALUED
    /// `reports_to` — silently wrong in the opposite direction from the
    /// silent-no-op vqy9 fixed. Single-valued predicates want a supersede
    /// primitive, and atomicity has to come from the store: both halves ride
    /// one `transact`, so no reader ever observes the predicate empty and a
    /// crash cannot strand it half-done.
    ///
    /// SINGLE-VALUE semantics by design: ALL current objects are replaced,
    /// however many there are (that is what "set" means — it is also the
    /// repair for an accidentally multi-valued predicate). A caller wanting
    /// add-without-remove asserts via `/knot` instead.
    ///
    /// Idempotent: setting the already-sole-current value is a no-op returning
    /// `(0, 0, 0)` — no transaction is written, matching `retract_triples`'
    /// `(0, 0)` convention for "nothing to do".
    ///
    /// Returns `(tx_id, retracted, asserted)`.
    pub fn set_triple(
        &mut self,
        entity: i64,
        predicate: i64,
        value: Value,
        timestamp: &str,
        actor: Option<&str>,
        explicit_str: bool,
    ) -> Result<(i64, usize, usize)> {
        let current: Vec<Fact> = self
            .entity_facts(entity)?
            .into_iter()
            .filter(|f| f.attribute == predicate)
            .collect();

        // The vqy9 footgun, write-side: a bare string for an IRI-valued
        // predicate would ASSERT a `Str` literal where every neighbour is a
        // `Ref` — a mis-shaped edge that answers no traversal. Refuse the two
        // unambiguous cases, the SAME two as retract_triples: the predicate
        // currently holds only Refs; or it holds NOTHING and the string is
        // IRI-shaped. A predicate that already stores string literals is left
        // alone even when the new string looks like a URL — URL-valued
        // literals (backend, externalUrl) are legitimate Strs, and refusing
        // them blocked the first real supersede batch this route was built
        // for. `explicit_str` is the caller saying {"str": "..."} — a tagged
        // literal is a stated intent, so the heuristic stands aside entirely
        // (json_to_value collapses both spellings to Value::Str, so the tag
        // must ride in as a flag or it is not an escape hatch at all).
        if let Value::Str(s) = &value {
            let holds_ref = current.iter().any(|f| matches!(f.value, Value::Ref(_)));
            let holds_str = current.iter().any(|f| matches!(f.value, Value::Str(_)));
            let looks_like_iri = s.contains("://") && !s.chars().any(char::is_whitespace);
            if !explicit_str
                && ((holds_ref && !holds_str) || (current.is_empty() && looks_like_iri))
            {
                let pred_iri = self.resolve(predicate)?;
                return Err(Error::InvalidValue(format!(
                    "set refused: object \"{s}\" is a string literal, but <{pred_iri}> \
                     takes an IRI reference. Pass the object as {{\"iri\": \"{s}\"}} to \
                     set an edge, or as {{\"str\": \"{s}\"}} to state that a literal is \
                     intended — a bare IRI-shaped string here is almost always a mis-shaped \
                     edge that no graph traversal can follow."
                )));
            }
        }

        let already_present = current.iter().any(|f| f.value == value);
        let to_retract: Vec<Fact> = current.into_iter().filter(|f| f.value != value).collect();

        if to_retract.is_empty() && already_present {
            return Ok((0, 0, 0));
        }

        let mut datums = retraction_datums(&to_retract);
        let retracted = datums.len();
        let asserted = usize::from(!already_present);
        if !already_present {
            datums.push(Datum {
                entity,
                attribute: predicate,
                value,
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: crate::types::Op::Assert,
            });
        }
        let tx_id = self.transact(&datums, timestamp, actor, Some("set"))?;
        Ok((tx_id, retracted, asserted))
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

    /// Build the retractions needed to replace a producer-owned episode
    /// snapshot, without committing them. The caller combines these datums with
    /// the replacement assertions in one transaction.
    pub(crate) fn plan_episode_retraction(
        &self,
        episode_name: &str,
        graph: i64,
    ) -> Result<Vec<Datum>> {
        let source_tag = format!("episode:{episode_name}");
        self.plan_source_retraction(&source_tag, graph)
    }

    /// Build the retractions needed to atomically replace all current facts
    /// owned by one stable transaction source.
    ///
    /// Episodes use `episode:<name>`; complete Turtle producers use
    /// `snapshot:<producer-key>`. Keeping the planner source-based makes
    /// replacement a store primitive rather than an episode-only convention.
    pub(crate) fn plan_source_retraction(
        &self,
        source_tag: &str,
        graph: i64,
    ) -> Result<Vec<Datum>> {
        let facts = {
            let mut stmt = self.conn.prepare(
                "SELECT f.e, f.a, f.v, f.tx, f.valid_from, f.valid_to, f.op \
                 FROM facts f JOIN transactions t ON f.tx = t.id \
                 WHERE t.source = ?1 AND f.g = ?2 AND f.op = 1 AND f.valid_to IS NULL \
                 ORDER BY f.e, f.a",
            )?;
            Self::collect_facts(&mut stmt, params![source_tag, graph])?
        };
        // Keep a label for nodes still referenced by another producer, so a
        // disappearing inventory item does not leave an unnameable dangling
        // reference. Types are deliberately not preserved: for snapshot
        // producers, a type assertion is often the inventory membership fact
        // readers count, and retaining it would make removal invisible.
        let orphans = self.identity_orphans(&facts, source_tag)?;
        let (label_id, _) = self.identity_predicate_ids()?;
        Ok(facts
            .into_iter()
            .filter(|f| {
                !orphans
                    .iter()
                    .any(|o| o.entity == f.entity && o.lost_label && Some(f.attribute) == label_id)
            })
            .map(|f| Datum {
                entity: f.entity,
                attribute: f.attribute,
                value: f.value,
                valid_from: f.valid_from,
                valid_to: None,
                op: Op::Retract,
            })
            .collect())
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
        let started = crate::time::Stopwatch::start();
        let limit_ms = self.search_config().query_timeout_ms;
        let deadline = (limit_ms > 0).then(|| crate::time::Deadline::after_millis(limit_ms));
        self.identity_orphans_until(in_scope, source_tag, started, deadline)
    }

    pub(super) fn identity_orphans_until(
        &self,
        in_scope: &[Fact],
        source_tag: &str,
        started: crate::time::Stopwatch,
        deadline: Option<crate::time::Deadline>,
    ) -> Result<Vec<IdentityOrphan>> {
        let check_deadline = || {
            if deadline.is_some_and(|dl| dl.passed()) {
                return Err(crate::error::Error::QueryTimeout {
                    elapsed_ms: started.elapsed_ms(),
                    limit_ms: deadline
                        .map(|dl| dl.millis_from(&started))
                        .unwrap_or_default(),
                });
            }
            Ok(())
        };
        check_deadline()?;
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
            // This is internal write planning, not SPARQL, but it runs while
            // the caller owns the sole writer mutex. Apply the same configured
            // budget so a future bad plan cannot park every writer for hours.
            check_deadline()?;
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
        // Keep subject and object probes separate. The former is covered by
        // idx_geav; the latter by the partial idx_active_vge. Combining them as
        // `(f.e = ? OR f.v = ?)` made SQLite abandon both access paths and
        // full-scan `facts` once PER candidate entity. A large snapshot /knot
        // therefore spent hours in identity_orphans while holding the sole
        // writer mutex (aegis-ffeud, captured in a live gdb backtrace).
        let mut subject = self.conn.prepare(
            "SELECT 1 FROM facts f JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL AND f.g = 0 \
               AND f.e = ?1 \
               AND (t.source IS NULL OR t.source <> ?2) \
             LIMIT 1",
        )?;
        if subject.exists(params![entity, source_tag])? {
            return Ok(true);
        }
        let mut object = self.conn.prepare(
            "SELECT 1 FROM facts f JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL AND f.g = 0 \
               AND f.v = ?1 \
               AND (t.source IS NULL OR t.source <> ?2) \
             LIMIT 1",
        )?;
        Ok(object.exists(params![as_object, source_tag])?)
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
             WHERE f.op = 1 AND f.valid_to IS NULL AND f.g = 0 \
               AND f.e = ?1 AND f.a = ?2 \
               AND (t.source IS NULL OR t.source <> ?3) \
             LIMIT 1",
        )?;
        Ok(stmt.exists(params![entity, predicate, source_tag])?)
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
