//! Snapshot promotion must not undo a later local decision to retract a fact.
//!
//! A foreign `tx_anchor` is not comparable with the receiving store's transaction
//! IDs. Conservatively retain ROOT's exact-fact retractions until the operator
//! explicitly reasserts that fact locally. Source graph membership remains the
//! unmodified evidence of what the pack supplied.

use std::collections::HashSet;

use crate::error::Result;
use crate::store::{Datum, Store};
use crate::types::{Fact, Op};

pub(crate) fn promote_snapshot(
    store: &mut Store,
    graph: i64,
    timestamp: &str,
    actor: Option<&str>,
    source: &str,
) -> Result<(i64, usize, usize)> {
    store
        .conn
        .execute_batch("SAVEPOINT quipu_snapshot_promotion")?;
    let result = (|| {
        let facts = store.current_facts_in_graph(graph)?;
        let (datums, suppressed) = without_retracted_root_facts(store, &facts, timestamp)?;
        let tx = store.transact(&datums, timestamp, actor, Some(source))?;
        Ok((tx, datums.len(), suppressed))
    })();
    match result {
        Ok(value) => {
            store
                .conn
                .execute_batch("RELEASE quipu_snapshot_promotion")?;
            Ok(value)
        }
        Err(error) => {
            store.conn.execute_batch(
                "ROLLBACK TO quipu_snapshot_promotion; RELEASE quipu_snapshot_promotion",
            )?;
            store.read_model.borrow_mut().clear();
            Err(error)
        }
    }
}

fn without_retracted_root_facts(
    store: &Store,
    facts: &[Fact],
    timestamp: &str,
) -> Result<(Vec<Datum>, usize)> {
    let mut statement = store.conn.prepare(
        "SELECT DISTINCT e, a, v FROM facts WHERE g=0 \
         AND (op=0 OR retracted_tx IS NOT NULL)",
    )?;
    let mut retracted: HashSet<(i64, i64, Vec<u8>)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    // A deliberate local reassertion supersedes a historical retraction. An
    // unrelated predicate, value, or graph never suppresses this fact.
    for fact in store.current_facts()? {
        retracted.remove(&(fact.entity, fact.attribute, fact.value.to_bytes()));
    }
    let datums: Vec<_> = facts
        .iter()
        .filter(|fact| !retracted.contains(&(fact.entity, fact.attribute, fact.value.to_bytes())))
        .map(|fact| Datum {
            entity: fact.entity,
            attribute: fact.attribute,
            value: fact.value.clone(),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    let suppressed = facts.len() - datums.len();
    Ok((datums, suppressed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share_import::{PromoteImportRequest, promote_import};
    use crate::types::Value;

    const TS: &str = "2026-09-24T00:00:00Z";
    const SHARE: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn setup() -> (Store, i64, i64, i64, PromoteImportRequest) {
        let mut store = Store::open_in_memory().unwrap();
        let graph = store
            .graph_create(&format!("urn:quipu:import:staging:{}", &SHARE[7..]))
            .unwrap();
        let e = store.intern("https://example.org/entity").unwrap();
        let a = store.intern("https://example.org/value").unwrap();
        let datums = ["old", "different"].map(|v| Datum {
            entity: e,
            attribute: a,
            value: Value::Str(v.into()),
            valid_from: TS.into(),
            valid_to: None,
            op: Op::Assert,
        });
        store
            .transact_to_graph(&datums, TS, None, Some("pack"), graph)
            .unwrap();
        (
            store,
            graph,
            e,
            a,
            PromoteImportRequest {
                share_id: SHARE.into(),
                actor: None,
            },
        )
    }

    #[test]
    fn stale_promotion_preserves_retraction_and_source_membership() {
        let (mut store, graph, e, a, request) = setup();
        promote_import(&mut store, &request, TS, None).unwrap();
        store
            .retract_triples(
                e,
                Some(a),
                Some(&Value::Str("old".into())),
                TS,
                None,
                true,
                None,
            )
            .unwrap();
        for _ in 0..2 {
            let result = promote_import(&mut store, &request, TS, None).unwrap();
            assert_eq!(result.suppressed_retractions, 1);
            let root = store.current_facts().unwrap();
            assert_eq!(root.len(), 1);
            assert_eq!(root[0].value, Value::Str("different".into()));
            assert_eq!(store.current_facts_in_graph(graph).unwrap().len(), 2);
        }
        // A local, explicit correction can restore a fact; pack reload cannot.
        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str("old".into()),
                    valid_from: TS.into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                TS,
                None,
                Some("local correction"),
            )
            .unwrap();
        let result = promote_import(&mut store, &request, TS, None).unwrap();
        assert_eq!(result.suppressed_retractions, 0);
        assert_eq!(store.current_facts().unwrap().len(), 2);
    }

    #[test]
    fn another_graphs_retraction_does_not_hide_root_content() {
        let (mut store, _, e, a, request) = setup();
        let other = store.graph_create("urn:other").unwrap();
        let mut datum = Datum {
            entity: e,
            attribute: a,
            value: Value::Str("old".into()),
            valid_from: TS.into(),
            valid_to: None,
            op: Op::Assert,
        };
        store
            .transact_to_graph(&[datum.clone()], TS, None, None, other)
            .unwrap();
        datum.op = Op::Retract;
        store
            .transact_to_graph(&[datum], TS, None, None, other)
            .unwrap();
        let result = promote_import(&mut store, &request, TS, None).unwrap();
        assert_eq!(result.suppressed_retractions, 0);
        assert_eq!(store.current_facts().unwrap().len(), 2);
    }
}
