//! Reactive OWL materialization — keep entailments live as facts change.
//!
//! The liveness half of gap G3 (`docs/design/semantic-reasoning-gaps.md`,
//! quipu-923): materialization used to run at ontology load only, so
//! entailments went stale as members arrived and the workaround was
//! re-encoding OWL axioms as Datalog rules. [`ReactiveOwl`] is a
//! [`TransactObserver`] that re-runs [`crate::owl::Ontology::materialize`]
//! when a committed delta touches vocabulary the loaded ontologies mention.
//!
//! Enabled by default via `owl.reactive_materialize`; deployments that need an
//! asserted-only write path can disable it explicitly.
//!
//! Re-materialization is safe to repeat: since the fixpoint change, a pass
//! stages only facts not already present, so a no-op delta produces no write
//! and the observer chain terminates. Self-triggering is cut by source
//! (`owl:materialize` deltas are skipped); Datalog-derived deltas are NOT
//! skipped, so a derived predicate carrying OWL axioms still composes —
//! both engines are monotone and idempotent, so the chain is finite.

use std::collections::HashSet;

use crate::namespace::RDF_TYPE;
use crate::owl::Axioms;
use crate::store::{Delta, Store, TransactObserver};
use crate::types::Value;

/// Observer that re-materializes the loaded ontologies on relevant deltas.
pub struct ReactiveOwl;

/// The property and class IRIs the axioms mention — the relevance test.
fn vocab_of(axioms: &Axioms) -> (HashSet<&str>, HashSet<&str>) {
    let mut properties: HashSet<&str> = HashSet::new();
    let mut classes: HashSet<&str> = HashSet::new();
    for (a, b) in &axioms.subproperty_of {
        properties.insert(a);
        properties.insert(b);
    }
    for (a, b) in &axioms.inverse_of {
        properties.insert(a);
        properties.insert(b);
    }
    for (a, b) in &axioms.equivalent_properties {
        properties.insert(a);
        properties.insert(b);
    }
    properties.extend(axioms.symmetric_properties.iter().map(String::as_str));
    properties.extend(axioms.transitive_properties.iter().map(String::as_str));
    for (p, c) in axioms.domains.iter().chain(axioms.ranges.iter()) {
        properties.insert(p);
        classes.insert(c);
    }
    for (a, b) in &axioms.subclass_of {
        classes.insert(a);
        classes.insert(b);
    }
    for (a, b) in &axioms.equivalent_classes {
        classes.insert(a);
        classes.insert(b);
    }
    (properties, classes)
}

impl TransactObserver for ReactiveOwl {
    fn after_commit(&self, store: &mut Store, delta: &Delta) -> crate::error::Result<()> {
        // Skip our own output and the inferred plane's bookkeeping writes;
        // everything else, Datalog derivations included, may feed an axiom.
        if matches!(
            delta.source.as_deref(),
            Some(
                "owl:materialize"
                    | crate::store::inferred::PLANE_SOURCE
                    | crate::store::inferred::MIGRATE_SOURCE
            )
        ) {
            return Ok(());
        }
        if delta.asserts.is_empty() && delta.retracts.is_empty() {
            return Ok(());
        }

        store.ensure_owl_cache()?;
        let Some(ontology) = store.owl_cache.as_deref() else {
            return Ok(());
        };
        // Clone to end the borrow — materialize needs `&mut Store`. The
        // ontology is small (axioms + raw triples), and this path only runs
        // for deployments that opted in.
        let ontology = ontology.clone();
        let (properties, classes) = vocab_of(&ontology.axioms);

        let rdf_type_id = store.lookup(RDF_TYPE)?;
        let mut relevant = false;
        for d in delta.asserts.iter().chain(delta.retracts.iter()) {
            if let Ok(attr_iri) = store.resolve(d.attribute)
                && properties.contains(attr_iri.as_str())
            {
                relevant = true;
                break;
            }
            if Some(d.attribute) == rdf_type_id
                && let Value::Ref(class_id) = &d.value
                && let Ok(class_iri) = store.resolve(*class_id)
                && classes.contains(class_iri.as_str())
            {
                relevant = true;
                break;
            }
        }
        if !relevant {
            return Ok(());
        }

        let timestamp = crate::time::now_iso();
        // SEMI-NAIVE from this delta (aegis-2dp8e2). The full path re-read every
        // current fact per pass — at 641,803 facts that was ~2.3 s per relevant
        // write against a 29/min offered load, i.e. more work than wall clock,
        // and it ratcheted because its own output is a premise next time.
        let seed: Vec<crate::types::Fact> = delta
            .asserts
            .iter()
            .map(|d| crate::types::Fact {
                entity: d.entity,
                attribute: d.attribute,
                value: d.value.clone(),
                tx: 0,
                valid_from: d.valid_from.clone(),
                valid_to: None,
                op: crate::types::Op::Assert,
            })
            .collect();
        ontology.materialize_delta(store, &timestamp, &seed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ReactiveOwl;
    use crate::store::Store;

    const TS: &str = "2026-01-01T00:00:00Z";

    /// Entailments stay live as members arrive — the recorded staleness
    /// scenario (a snapshot "correct once, then stale") no longer needs the
    /// re-encode-as-Datalog workaround.
    #[test]
    fn relevant_write_rematerializes() {
        const ONT: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .
ex:dependsOn a owl:ObjectProperty, owl:TransitiveProperty .
"#;
        let mut store = Store::open_in_memory().unwrap();
        store.load_ontology("t", ONT, TS).unwrap();
        store.add_observer(Arc::new(ReactiveOwl));

        let ingest = |store: &mut Store, ttl: &str| {
            crate::rdf::ingest_rdf(
                store,
                ttl.as_bytes(),
                oxrdfio::RdfFormat::Turtle,
                None,
                TS,
                None,
                None,
            )
            .unwrap();
        };
        ingest(
            &mut store,
            "@prefix ex: <http://example.org/> .\nex:a ex:dependsOn ex:b .\n",
        );
        // The second write is what the one-shot materializer went stale on.
        ingest(
            &mut store,
            "@prefix ex: <http://example.org/> .\nex:b ex:dependsOn ex:c .\n",
        );

        let result = crate::sparql::query(
            &store,
            "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/a> <http://example.org/dependsOn> <http://example.org/c> }",
        )
        .unwrap();
        assert!(
            matches!(result, crate::sparql::QueryResult::Ask(true)),
            "the closure must extend reactively as members arrive"
        );
    }

    #[test]
    fn inverse_and_symmetric_axioms_remain_live_after_load() {
        const ONT: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .
ex:authors owl:inverseOf ex:authoredBy .
ex:friendOf a owl:SymmetricProperty .
"#;
        let mut store = Store::open_in_memory().unwrap();
        store.load_ontology("relations", ONT, TS).unwrap();
        store.add_observer(Arc::new(ReactiveOwl));

        crate::rdf::ingest_rdf(
            &mut store,
            b"@prefix ex: <http://example.org/> .\nex:alice ex:authors ex:paper .\nex:alice ex:friendOf ex:bob .\n".as_ref(),
            oxrdfio::RdfFormat::Turtle,
            None,
            TS,
            None,
            None,
        )
        .unwrap();

        for query in [
            "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/paper> <http://example.org/authoredBy> <http://example.org/alice> }",
            "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> { <http://example.org/bob> <http://example.org/friendOf> <http://example.org/alice> }",
        ] {
            let result = crate::sparql::query(&store, query).unwrap();
            assert!(
                matches!(result, crate::sparql::QueryResult::Ask(true)),
                "missing reactive entailment for {query}"
            );
        }
    }

    /// An irrelevant write must not trigger a materialization pass — pinned
    /// through the absence of any owl:materialize transaction.
    #[test]
    fn irrelevant_write_is_skipped() {
        const ONT: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://example.org/> .
ex:dependsOn a owl:ObjectProperty, owl:TransitiveProperty .
"#;
        let mut store = Store::open_in_memory().unwrap();
        store.load_ontology("t", ONT, TS).unwrap();
        store.add_observer(Arc::new(ReactiveOwl));

        crate::rdf::ingest_rdf(
            &mut store,
            b"@prefix ex: <http://example.org/> .\nex:a ex:unrelated ex:b .\n".as_ref(),
            oxrdfio::RdfFormat::Turtle,
            None,
            TS,
            None,
            None,
        )
        .unwrap();

        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE source = 'owl:materialize'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "no materialization pass may run for unrelated vocabulary"
        );
    }
}
