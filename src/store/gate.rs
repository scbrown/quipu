//! The write gate: governance, authority, and OWL hooks on the write path.
//!
//! Split from `mod.rs` (quipu-bu3). Every method here is consulted by
//! `transact()` (see `ops`) around a staged write: policy enforcement and its
//! staged verdicts/requests, principal-chain authority, placement validation,
//! and the OWL write-time constraint family.

#[cfg(feature = "owl")]
use rusqlite::params;

use super::{Datum, Store};
use crate::error::{Error, Result};
use crate::governance::PolicyRegistry;
#[cfg(feature = "owl")]
use crate::types::Op;
#[cfg(feature = "owl")]
use crate::types::Value;

impl Store {
    /// Evaluate action-boundary policies for a staged write. No-op unless
    /// `governance.enforce_on_write` is set. Builds and caches the policy
    /// registry on first use. Returns `Err(PolicyDenied)` when a `deny` policy's
    /// claim is unsatisfied for a touched target — the caller rolls the write
    /// back so nothing is committed.
    pub(crate) fn enforce_write_policies(&mut self, datums: &[Datum], graph: i64) -> Result<()> {
        if !self.governance_config.enforce_on_write || self.recording_verdicts {
            return Ok(());
        }
        if self.policy_registry.is_none() {
            self.policy_registry = Some(PolicyRegistry::build(self)?);
        }
        // Take the registry out so the evaluator can borrow `&self` (SPARQL);
        // restore it afterwards. Evaluation never mutates the registry.
        let registry = self.policy_registry.take().expect("registry just built");
        let mut verdicts = Vec::new();
        let mut requests = Vec::new();
        let result = registry.evaluate_write(self, datums, graph, &mut verdicts, &mut requests);
        self.policy_registry = Some(registry);
        self.pending_requests = requests;
        // STAGED, not written. The caller writes them after the savepoint
        // resolves — a denial rolls back, and a verdict written inside that
        // savepoint would be rolled back with it.
        self.pending_verdicts = verdicts;
        result
    }

    /// Drop the cached ontology so the next write rebuilds it (aegis-bmqup).
    #[cfg(feature = "owl")]
    pub fn invalidate_owl_cache(&mut self) {
        self.owl_cache = None;
    }

    /// Close the prior value of a functional property so a new one can replace it
    /// (aegis-7vn3b).
    ///
    /// THE BUG THIS FIXES. The write path only ever CLOSED a fact on an exact
    /// `(e,a,v)` retraction, so asserting a different value for the same `(e,a)`
    /// left both live. Measured: `contentHash = "aaa"` then `contentHash = "bbb"`
    /// yields BOTH as current facts. Two consequences, one silent and one loud —
    /// cleaning duplicate scalars is undone by the next re-ingest (the aegis-h69po
    /// filePath fix went 205 → 0 → 50 in hours), and declaring the property
    /// `owl:FunctionalProperty` made every ordinary update an HTTP 400, because the
    /// update itself manufactured the second value.
    ///
    /// THE SEMANTICS. In a bitemporal store `owl:FunctionalProperty` means *at most
    /// one value AT A TIME*, so a new value must CLOSE the old — that is an update,
    /// and it is the common case. Rejection remains correct for two distinct values
    /// inside ONE batch, where nothing says which should win; those are left alone
    /// here and `enforce_owl_constraints` still refuses them.
    ///
    /// Tied to the same `owl.validate_on_write` flag as the rejection half, so the
    /// switch turns on one coherent behaviour rather than half of one.
    #[cfg(feature = "owl")]
    pub(crate) fn supersede_functional_values(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
        graph: i64,
        tx_id: i64,
    ) -> Result<usize> {
        if !self.owl_config.validate_on_write || self.recording_verdicts {
            return Ok(0);
        }
        self.ensure_owl_cache()?;
        let Some(ontology) = self.owl_cache.take() else {
            return Ok(0);
        };
        let functional = ontology.axioms.functional_properties.clone();
        self.owl_cache = Some(ontology);
        if functional.is_empty() {
            return Ok(0);
        }

        // Group this batch's asserts by (entity, attribute) for functional attrs.
        let mut proposed: std::collections::HashMap<(i64, i64), Vec<Vec<u8>>> =
            std::collections::HashMap::new();
        for d in datums {
            if d.op != Op::Assert {
                continue;
            }
            let Ok(attr_iri) = self.resolve(d.attribute) else {
                continue;
            };
            if functional.contains(&attr_iri) {
                proposed
                    .entry((d.entity, d.attribute))
                    .or_default()
                    .push(d.value.to_bytes());
            }
        }

        let mut closed = 0usize;
        // quipu #83: stamp the retracting tx here TOO. This is the SECOND
        // fact-closing site; fixing only `close_assertion` would leave every
        // functional-property supersede invisible to as-of-tx, in exactly the
        // way the whole defect describes.
        let mut close_other = self.conn.prepare(
            "UPDATE facts SET valid_to = ?1, retracted_tx = ?6 \
             WHERE e = ?2 AND a = ?3 AND v != ?4 AND g = ?5 AND op = 1 AND valid_to IS NULL",
        )?;
        for ((entity, attribute), values) in &proposed {
            // AMBIGUOUS BATCH: two distinct values for one functional property in
            // a single write. Superseding here would silently pick whichever the
            // loop saw last. Leave it untouched — the validator rejects it, which
            // is the honest outcome when the caller has not said which wins.
            let distinct: std::collections::HashSet<&Vec<u8>> = values.iter().collect();
            if distinct.len() > 1 {
                continue;
            }
            let Some(new_value) = values.first() else {
                continue;
            };
            closed += close_other.execute(params![
                timestamp, entity, attribute, new_value, graph, tx_id
            ])?;
        }
        Ok(closed)
    }

    /// Build the combined ontology cache if it is not already populated.
    #[cfg(feature = "owl")]
    pub(crate) fn ensure_owl_cache(&mut self) -> Result<()> {
        if self.owl_cache.is_some() {
            return Ok(());
        }
        let stored = self.list_ontologies()?;
        if stored.is_empty() {
            return Ok(());
        }
        let combined: String = stored
            .iter()
            .map(|(_, turtle, _)| turtle.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.owl_cache = Some(Box::new(crate::owl::Ontology::from_turtle(&combined)?));
        Ok(())
    }

    /// Derive the rdf:type facts implied by loaded rdfs:domain/rdfs:range
    /// axioms for one pending write (aegis-qfncf).
    #[cfg(feature = "owl")]
    pub(crate) fn owl_domain_range_inferences(
        &mut self,
        datums: &[Datum],
        timestamp: &str,
    ) -> Result<Vec<Datum>> {
        self.ensure_owl_cache()?;
        let Some(ontology) = self.owl_cache.take() else {
            return Ok(Vec::new());
        };
        let domains = ontology.axioms.domains.clone();
        let ranges = ontology.axioms.ranges.clone();
        self.owl_cache = Some(ontology);

        if domains.is_empty() && ranges.is_empty() {
            return Ok(Vec::new());
        }

        let rdf_type = self.intern(crate::namespace::RDF_TYPE)?;
        let mut out = Vec::new();
        for datum in datums.iter().filter(|d| d.op == Op::Assert) {
            let Ok(predicate) = self.resolve(datum.attribute) else {
                continue;
            };
            for (_, class) in domains.iter().filter(|(prop, _)| prop == &predicate) {
                out.push(Datum {
                    entity: datum.entity,
                    attribute: rdf_type,
                    value: Value::Ref(self.intern(class)?),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                });
            }
            if let Value::Ref(target) = &datum.value {
                for (_, class) in ranges.iter().filter(|(prop, _)| prop == &predicate) {
                    out.push(Datum {
                        entity: *target,
                        attribute: rdf_type,
                        value: Value::Ref(self.intern(class)?),
                        valid_from: timestamp.to_string(),
                        valid_to: None,
                        op: Op::Assert,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Reject a write that violates `owl:disjointWith` or `owl:FunctionalProperty`
    /// (aegis-bmqup).
    ///
    /// `Ontology::validate()` implemented both constraints and had NO CALLER in
    /// the server, while `docs/book/src/concepts/owl.md` stated that Quipu
    /// "enforces at write time" and listed them as enforced. This is that caller.
    ///
    /// Cost is bounded by the WRITE, not the store: `validate()` derives the
    /// touched entities from the proposed datums and then reads only those
    /// entities' existing facts. The ontology itself is cached because otherwise
    /// every transaction would re-parse every stored TTL.
    ///
    /// Off by default (`owl.validate_on_write`) — see `OwlConfig` for why
    /// flipping it on is a behaviour change, not a bug fix.
    #[cfg(feature = "owl")]
    pub(crate) fn enforce_owl_constraints(&mut self, datums: &[Datum]) -> Result<()> {
        if !self.owl_config.validate_on_write || self.recording_verdicts {
            return Ok(());
        }
        // One ontology over the union of every stored TTL: a disjointness
        // declared in one set must still bite a write validated against all.
        self.ensure_owl_cache()?;
        let Some(ontology) = self.owl_cache.take() else {
            return Ok(());
        };
        let result = ontology.validate(self, datums);
        self.owl_cache = Some(ontology);
        let violations = result?;
        if violations.is_empty() {
            return Ok(());
        }
        // Structured, and naming EVERY violation rather than just the first —
        // an author fixing them one round-trip at a time is how a strict gate
        // gets switched off.
        let detail = violations
            .iter()
            .map(|v| format!("{} ({})", v.message, v.focus_node))
            .collect::<Vec<_>>()
            .join("; ");
        Err(Error::InvalidValue(format!(
            "OWL constraint violation ({} violation(s)): {detail}",
            violations.len()
        )))
    }

    /// Write the verdicts the gate staged, in their own transaction.
    ///
    /// Called after the write's savepoint has resolved EITHER WAY, so the
    /// verdict of a denial survives the rollback that denial caused. Failures
    /// here are swallowed: a verdict that cannot be recorded must not turn a
    /// successful write into a failed one, nor a denial into a different error
    /// than the policy's.
    pub(crate) fn flush_pending_verdicts(&mut self, timestamp: &str, actor: Option<&str>) {
        self.flush_pending_requests(timestamp);
        let pending = std::mem::take(&mut self.pending_verdicts);
        if pending.is_empty() || self.recording_verdicts {
            return;
        }
        let mut datums = Vec::new();
        for verdict in &pending {
            match crate::governance::verdict_facts::datums_for(self, verdict, timestamp, actor) {
                Ok(mut d) => datums.append(&mut d),
                // No signing identity => no verdict, never an unsigned one.
                Err(_) => return,
            }
        }
        if datums.is_empty() {
            return;
        }
        self.recording_verdicts = true;
        let _ = self.transact(
            &datums,
            timestamp,
            Some("quipu"),
            Some("write-gate verdict"),
        );
        self.recording_verdicts = false;
    }

    /// Set the principal-and-agent chain for subsequent writes.
    ///
    /// Ordered outermost-first: `[originating principal, …, executor]`. The
    /// effective authority is the INTERSECTION along it, so appending a delegate
    /// can only narrow what may be written (SARC §9.3).
    pub fn set_principal_chain(&mut self, chain: Vec<String>) {
        self.principal_chain = chain;
    }

    /// The current chain.
    #[must_use]
    pub fn principal_chain(&self) -> &[String] {
        &self.principal_chain
    }

    /// Refuse a write to `graph` that the chain's authority does not cover.
    ///
    /// Gated by `[quipu.governance] enforce_authority`, default off. With NO
    /// chain set the check does not apply: an unattributed write is the shape
    /// every existing caller has, and turning attribution into a hard
    /// requirement beneath a running deployment would break every one of them
    /// at once. What the flag buys is that a chain, once supplied, is BINDING —
    /// so adopting attribution is opt-in per caller and cannot silently widen.
    pub(crate) fn enforce_graph_authority(&self, graph: i64) -> Result<()> {
        if !self.governance_config.enforce_authority
            || self.recording_verdicts
            || self.principal_chain.is_empty()
        {
            return Ok(());
        }
        let graph_iri = if graph == crate::schema::ROOT_GRAPH {
            crate::schema::ROOT_GRAPH_IRI.to_string()
        } else {
            self.resolve(graph)?
        };
        let authority = crate::governance::authority::chain_authority(self, &self.principal_chain)?;
        if authority.permits(&graph_iri) {
            return Ok(());
        }
        Err(Error::PolicyDenied(crate::governance::authority::refusal(
            &self.principal_chain,
            &graph_iri,
            &authority,
        )))
    }

    /// Write the `DecisionRequest`s the router staged, after the savepoint has
    /// resolved. Same ordering, same reason, as the verdicts.
    fn flush_pending_requests(&mut self, timestamp: &str) {
        let pending = std::mem::take(&mut self.pending_requests);
        if pending.is_empty() || self.recording_verdicts {
            return;
        }
        let mut datums = Vec::new();
        for request in &pending {
            match crate::governance::router::mint_request(
                self,
                &request.policy_iri,
                &request.target_iri,
                None,
                request.window_secs,
                request.now,
                timestamp,
            ) {
                Ok(mut d) => datums.append(&mut d),
                Err(_) => return,
            }
        }
        if datums.is_empty() {
            return;
        }
        self.recording_verdicts = true;
        let _ = self.transact(
            &datums,
            timestamp,
            Some("quipu"),
            Some("escalation request"),
        );
        self.recording_verdicts = false;
    }

    /// Validate the SARC class↔placement rules for any policy this write
    /// defines or amends (`src/governance/placement.rs`). Gated by
    /// `[quipu.governance] validate_placement`, default off.
    ///
    /// Deliberately NOT gated by `enforce_on_write`: definition-time
    /// well-formedness of a constraint is a different question from
    /// evaluation-time enforcement of it, and a deployment may reasonably want
    /// its policy definitions checked while it is still staging enforcement in
    /// advise mode.
    pub(crate) fn validate_policy_placement(&self, datums: &[Datum], graph: i64) -> Result<()> {
        if !self.governance_config.validate_placement || self.recording_verdicts {
            return Ok(());
        }
        crate::governance::validate_placement(self, datums, graph)
    }

    /// Verify agent transition signatures for any `aegis:TransitionEvent`
    /// this write defines or amends (`src/governance/transition.rs`,
    /// quipu-8cc). Gated by `[quipu.governance] verify_transitions`, default
    /// off. Independent of `enforce_on_write` for the same reason the
    /// placement check is: authenticity of a recorded transition is a
    /// different question from policy evaluation over it.
    pub(crate) fn verify_transition_signatures(&self, datums: &[Datum], graph: i64) -> Result<()> {
        if !self.governance_config.verify_transitions || self.recording_verdicts {
            return Ok(());
        }
        crate::governance::verify_transitions(self, datums, graph)
    }

    /// Invalidate the cached policy registry if this transaction defined or
    /// amended a governance policy. Cheap no-op unless enforcement is enabled.
    pub(crate) fn invalidate_policy_registry_if_governance(
        &mut self,
        datums: &[Datum],
    ) -> Result<()> {
        if !self.governance_config.enforce_on_write {
            return Ok(());
        }
        if crate::governance::is_governance_write(self, datums)? {
            self.policy_registry = None;
        }
        Ok(())
    }
}
