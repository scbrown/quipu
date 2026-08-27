//! Companion inferred graphs — the entailment regime's placement (quipu-0b6).
//!
//! Decision record (Stiwi, 2026-08-27, `docs/design/entailment-regime.md`):
//! reasoner- and OWL-derived facts are **quarantined by placement** — they
//! land in a companion inferred graph, never in the graph of their premises.
//! The companion is found by a **reserved IRI suffix**, chosen for intuitive
//! access (`FROM <g> FROM <g#inferred>` composes the closure with no registry
//! lookup):
//!
//! - a premise graph `<g>` has companion `<g>#inferred`;
//! - a premise IRI already carrying a fragment gets `-inferred` appended;
//! - ROOT (graph id 0) is addressable by its existing well-known IRI
//!   `urn:quipu:graph:root` (`schema::ROOT_GRAPH_IRI`), with companion
//!   `urn:quipu:graph:root#inferred`.
//!
//! What makes the convention safe rather than merely convenient: the suffix
//! is **reserved**. `transact_to_graph` refuses an external write to any
//! graph IRI carrying it — only the engines (and the migration) may populate
//! a companion, so a user minting `…#inferred` cannot forge entailments.
//!
//! Each companion is self-describing: it carries
//! `<companion> aegis:sourceKind "inferred"` (the tag attaches at the GRAPH
//! level — tagging a derived triple's *subject* would mistag the subject,
//! which may itself be observed) and a freshness note
//! `<companion> quipu:derivedAsOfTx <n>`, the premise-side transaction head
//! the closure last reflected. Reported, never faked: consumers compare it to
//! the premise graph's head and judge staleness themselves.

use rusqlite::params;

use crate::error::{Error, Result};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// Reserved suffix marking a companion inferred graph.
pub const INFERRED_SUFFIX: &str = "#inferred";

/// ROOT's companion inferred graph — `schema::ROOT_GRAPH_IRI` + the suffix.
pub const ROOT_INFERRED_GRAPH_IRI: &str = "urn:quipu:graph:root#inferred";

/// Freshness note predicate: the premise-side transaction head the
/// companion's closure last reflected.
pub const DERIVED_AS_OF_TX: &str = "http://quipu.local/graph#derivedAsOfTx";

/// The epistemic-class tag camayoc's ingress discipline defines; carried by
/// the companion graph node itself.
pub const SOURCE_KIND: &str = "http://aegis.gastown.local/ontology/sourceKind";

/// Source string for the plane's own bookkeeping writes (tag, freshness).
pub const PLANE_SOURCE: &str = "quipu:inferred-plane";

/// Source string the one-time migration writes under.
pub const MIGRATE_SOURCE: &str = "quipu:migrate-inferred";

/// The companion IRI for a premise graph IRI.
pub fn companion_iri_for(premise_iri: &str) -> String {
    if premise_iri.contains('#') {
        format!("{premise_iri}-inferred")
    } else {
        format!("{premise_iri}{INFERRED_SUFFIX}")
    }
}

/// Does this IRI carry the reserved companion marking?
pub fn is_inferred_graph_iri(iri: &str) -> bool {
    iri.ends_with(INFERRED_SUFFIX) || (iri.contains('#') && iri.ends_with("-inferred"))
}

/// May a transaction with this `source` write into a companion graph?
pub fn source_may_write_inferred(source: Option<&str>) -> bool {
    match source {
        Some(s) => {
            s.starts_with("reasoner:")
                || s == "owl:materialize"
                || s == PLANE_SOURCE
                || s == MIGRATE_SOURCE
        }
        None => false,
    }
}

impl Store {
    /// Refuse external writes that target a reserved companion graph.
    pub(crate) fn assert_write_target_allowed(
        &self,
        graph: i64,
        source: Option<&str>,
    ) -> Result<()> {
        self.assert_graph_is_writable(graph)?;
        if graph == crate::schema::ROOT_GRAPH {
            return Ok(());
        }
        let iri = self.graph_iri_of(graph);
        if is_inferred_graph_iri(&iri) && !source_may_write_inferred(source) {
            return Err(Error::InvalidValue(format!(
                "graph <{iri}> is a companion inferred graph — the \
                 '#inferred' suffix is reserved for engine-derived \
                 entailments (quipu-0b6), so external writes are refused. \
                 Write to the premise graph instead; the reasoner and \
                 materializer populate the companion."
            )));
        }
        Ok(())
    }

    /// The companion inferred-graph IRI for a premise graph id.
    pub fn companion_inferred_iri(&self, g: i64) -> Result<String> {
        if g == crate::schema::ROOT_GRAPH {
            return Ok(ROOT_INFERRED_GRAPH_IRI.to_string());
        }
        Ok(companion_iri_for(&self.resolve(g)?))
    }

    /// Intern (and, on first use, tag) the companion inferred graph for `g`.
    ///
    /// First use writes the graph-level `aegis:sourceKind "inferred"` tag so
    /// the companion is self-describing; subsequent calls are a lookup.
    pub fn ensure_companion_inferred_graph(&mut self, g: i64, timestamp: &str) -> Result<i64> {
        let iri = self.companion_inferred_iri(g)?;
        let companion = self.intern(&iri)?;
        let kind_attr = self.intern(SOURCE_KIND)?;
        let tagged = self
            .current_facts_in_graph(companion)?
            .iter()
            .any(|f| f.entity == companion && f.attribute == kind_attr);
        if !tagged {
            self.transact_to_graph(
                &[Datum {
                    entity: companion,
                    attribute: kind_attr,
                    value: Value::Str("inferred".into()),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                timestamp,
                Some("quipu"),
                Some(PLANE_SOURCE),
                companion,
            )?;
        }
        Ok(companion)
    }

    /// The premise-side transaction head — what a freshness note records.
    pub fn transaction_head(&self) -> Result<i64> {
        let head: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM transactions",
            params![],
            |r| r.get(0),
        )?;
        Ok(head)
    }

    /// Refresh `<companion> quipu:derivedAsOfTx <as_of>` inside the companion
    /// graph. Retract-then-assert, so the note stays single-valued.
    pub fn note_inferred_freshness(
        &mut self,
        companion: i64,
        as_of_tx: i64,
        timestamp: &str,
    ) -> Result<()> {
        let attr = self.intern(DERIVED_AS_OF_TX)?;
        let mut datums = Vec::new();
        for f in self.current_facts_in_graph(companion)? {
            if f.entity == companion && f.attribute == attr {
                if f.value == Value::Int(as_of_tx) {
                    return Ok(()); // already current
                }
                datums.push(Datum {
                    entity: companion,
                    attribute: attr,
                    value: f.value,
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Retract,
                });
            }
        }
        datums.push(Datum {
            entity: companion,
            attribute: attr,
            value: Value::Int(as_of_tx),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
        self.transact_to_graph(
            &datums,
            timestamp,
            Some("quipu"),
            Some(PLANE_SOURCE),
            companion,
        )?;
        Ok(())
    }

    /// One-time migration (quipu-0b6): move current engine-derived facts out
    /// of their premise graphs into the companion inferred graphs.
    ///
    /// A fact qualifies when its transaction source is `owl:materialize` or
    /// `reasoner:<id>` and it does not already sit in a companion graph. The
    /// move is retract-in-premise + assert-in-companion under
    /// `quipu:migrate-inferred` — ordinary bitemporal writes, no history
    /// rewriting. Returns `(graphs_touched, facts_moved)`.
    pub fn migrate_inferred(&mut self, timestamp: &str) -> Result<(usize, usize)> {
        struct Row {
            g: i64,
            e: i64,
            a: i64,
            v: Vec<u8>,
        }
        let rows: Vec<Row> = {
            let mut stmt = self.conn.prepare(
                "SELECT f.g, f.e, f.a, f.v FROM facts f \
                 JOIN transactions t ON f.tx = t.id \
                 WHERE f.op = 1 AND f.valid_to IS NULL \
                   AND (t.source = 'owl:materialize' OR t.source LIKE 'reasoner:%')",
            )?;
            let mut out = Vec::new();
            let mut q = stmt.query(params![])?;
            while let Some(r) = q.next()? {
                out.push(Row {
                    g: r.get(0)?,
                    e: r.get(1)?,
                    a: r.get(2)?,
                    v: r.get(3)?,
                });
            }
            out
        };

        let mut graphs = std::collections::BTreeMap::<i64, Vec<Row>>::new();
        for row in rows {
            // Facts already in a companion graph stay put.
            if row.g != crate::schema::ROOT_GRAPH && is_inferred_graph_iri(&self.resolve(row.g)?) {
                continue;
            }
            graphs.entry(row.g).or_default().push(row);
        }

        let mut moved = 0_usize;
        let graph_count = graphs.len();
        for (g, rows) in graphs {
            let companion = self.ensure_companion_inferred_graph(g, timestamp)?;
            let mut retracts = Vec::with_capacity(rows.len());
            let mut asserts = Vec::with_capacity(rows.len());
            for row in &rows {
                let value = Value::from_bytes(&row.v)?;
                retracts.push(Datum {
                    entity: row.e,
                    attribute: row.a,
                    value: value.clone(),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Retract,
                });
                asserts.push(Datum {
                    entity: row.e,
                    attribute: row.a,
                    value,
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                });
            }
            moved += asserts.len();
            self.transact_to_graph(
                &asserts,
                timestamp,
                Some("quipu"),
                Some(MIGRATE_SOURCE),
                companion,
            )?;
            self.transact_to_graph(&retracts, timestamp, Some("quipu"), Some(MIGRATE_SOURCE), g)?;
        }
        Ok((graph_count, moved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    const TS: &str = "2026-01-01T00:00:00Z";

    fn triple(store: &mut Store, s: &str, p: &str, o: &str) -> Datum {
        let e = store.intern(s).unwrap();
        let a = store.intern(p).unwrap();
        let v = store.intern(o).unwrap();
        Datum {
            entity: e,
            attribute: a,
            value: Value::Ref(v),
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        }
    }

    /// The suffix is RESERVED: an external write to a companion graph is
    /// refused, whatever the caller; engine sources pass.
    #[test]
    fn reserved_suffix_refuses_external_writes() {
        let mut store = Store::open_in_memory().unwrap();
        let g = store.intern("http://example.org/data#inferred").unwrap();
        let datum = triple(&mut store, "ex:a", "ex:p", "ex:b");

        let err = store
            .transact_to_graph(
                std::slice::from_ref(&datum),
                TS,
                Some("test"),
                Some("episode"),
                g,
            )
            .expect_err("an external source must be refused");
        assert!(
            format!("{err}").contains("reserved"),
            "the refusal must say why: {err}"
        );

        store
            .transact_to_graph(&[datum], TS, Some("reasoner"), Some("reasoner:R1"), g)
            .expect("engine output must be accepted");
    }

    /// Placement proof: a derivation is ABSENT from the premise graph, present
    /// in the companion, and a composed FROM query sees the union — while a
    /// plain (asserted-only) query does not.
    #[test]
    fn derivations_live_apart_and_compose_via_from() {
        use crate::reasoner::{RULE_NS, evaluate, parse_rules};
        let ttl = format!(
            r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .
ex:r a rule:Rule ; rule:id "R1" ;
    rule:head "<http://ex/h>(?x, ?y)" ; rule:body "<http://ex/p>(?x, ?y)" .
"#
        );
        let mut store = Store::open_in_memory().unwrap();
        let base = triple(&mut store, "http://ex/a", "http://ex/p", "http://ex/b");
        store
            .transact(&[base], TS, Some("test"), Some("base"))
            .unwrap();
        let rs = parse_rules(&ttl, None).unwrap();
        evaluate(&mut store, &rs, TS).unwrap();

        let plain =
            crate::sparql::query(&store, "ASK { <http://ex/a> <http://ex/h> <http://ex/b> }")
                .unwrap();
        assert!(
            matches!(plain, crate::sparql::QueryResult::Ask(false)),
            "a plain query sees asserted facts only — the derivation must NOT be in ROOT"
        );

        let composed = crate::sparql::query(
            &store,
            "ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> \
             { <http://ex/a> <http://ex/h> <http://ex/b> }",
        )
        .unwrap();
        assert!(
            matches!(composed, crate::sparql::QueryResult::Ask(true)),
            "the base+inferred composition must see the derivation"
        );
    }

    /// The companion is self-describing: graph-level sourceKind tag and a
    /// freshness note recording the premise head the closure reflects.
    #[test]
    fn companion_carries_tag_and_freshness_note() {
        use crate::reasoner::{RULE_NS, evaluate, parse_rules};
        let ttl = format!(
            r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .
ex:r a rule:Rule ; rule:id "R1" ;
    rule:head "<http://ex/h>(?x, ?y)" ; rule:body "<http://ex/p>(?x, ?y)" .
"#
        );
        let mut store = Store::open_in_memory().unwrap();
        let base = triple(&mut store, "http://ex/a", "http://ex/p", "http://ex/b");
        store
            .transact(&[base], TS, Some("test"), Some("base"))
            .unwrap();
        let head_before = store.transaction_head().unwrap();
        evaluate(&mut store, &parse_rules(&ttl, None).unwrap(), TS).unwrap();

        let companion = store.lookup(ROOT_INFERRED_GRAPH_IRI).unwrap().unwrap();
        let kind = store.intern(SOURCE_KIND).unwrap();
        let note = store.intern(DERIVED_AS_OF_TX).unwrap();
        let facts = store.current_facts_in_graph(companion).unwrap();
        assert!(
            facts.iter().any(|f| f.entity == companion
                && f.attribute == kind
                && f.value == Value::Str("inferred".into())),
            "the companion must carry the graph-level sourceKind tag"
        );
        let freshness: Vec<&crate::types::Fact> = facts
            .iter()
            .filter(|f| f.entity == companion && f.attribute == note)
            .collect();
        assert_eq!(freshness.len(), 1, "one freshness note, single-valued");
        assert!(
            matches!(freshness[0].value, Value::Int(n) if n >= head_before),
            "the note records the premise head the closure reflects"
        );
    }

    /// Migration moves legacy-placed derived facts (engine sources in a
    /// premise graph) into the companion — retract there, assert here.
    #[test]
    fn migrate_inferred_moves_legacy_derivations() {
        let mut store = Store::open_in_memory().unwrap();
        // A base fact and a LEGACY derived fact, both sitting in ROOT the way
        // pre-regime stores hold them.
        let base = triple(&mut store, "http://ex/a", "http://ex/p", "http://ex/b");
        store
            .transact(&[base], TS, Some("test"), Some("base"))
            .unwrap();
        let legacy = triple(&mut store, "http://ex/a", "http://ex/h", "http://ex/b");
        store
            .transact(&[legacy], TS, Some("reasoner"), Some("reasoner:R1"))
            .unwrap();

        let (graphs, moved) = store.migrate_inferred(TS).unwrap();
        assert_eq!((graphs, moved), (1, 1), "one graph touched, one fact moved");

        let h = store.lookup("http://ex/h").unwrap().unwrap();
        assert!(
            !store
                .current_facts()
                .unwrap()
                .iter()
                .any(|f| f.attribute == h),
            "the derived fact must have left ROOT"
        );
        let companion = store.lookup(ROOT_INFERRED_GRAPH_IRI).unwrap().unwrap();
        assert!(
            store
                .current_facts_in_graph(companion)
                .unwrap()
                .iter()
                .any(|f| f.attribute == h),
            "the derived fact must now sit in the companion"
        );
        // The base fact stays put.
        let p = store.lookup("http://ex/p").unwrap().unwrap();
        assert!(
            store
                .current_facts()
                .unwrap()
                .iter()
                .any(|f| f.attribute == p)
        );

        // Idempotent: a second run finds nothing to move.
        let (_, moved_again) = store.migrate_inferred(TS).unwrap();
        assert_eq!(moved_again, 0, "migration must be idempotent");
    }
}
