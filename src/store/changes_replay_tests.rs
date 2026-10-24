//! Feed replay conformance (local-sync-mode.md §2, P0; aegis-xdvg.1).
//!
//! A replica built only from `/changes` records must equal its source. Each
//! test replays every page into an EMPTY store and compares a canonical digest
//! of the live facts. The plain fixture is the positive control: it proves the
//! instrument can say "equal". Each defect fixture isolates one feed gap.

use std::collections::BTreeSet;

use serde_json::Value as Json;

use super::*;
use crate::store::{Datum, Store};
use crate::types::Op;

pub(super) fn datum(e: i64, a: i64, value: Value, valid_from: &str, op: Op) -> Datum {
    Datum {
        entity: e,
        attribute: a,
        value,
        valid_from: valid_from.into(),
        valid_to: None,
        op,
    }
}

/// `(e, a, v, g, valid_from, valid_to)` as stored.
type FactRow = (i64, i64, Vec<u8>, i64, String, Option<String>);

/// Live facts as resolved strings, so two stores with different term ids
/// compare by meaning. Valid time is part of a fact's identity here.
pub(super) fn live_digest(store: &Store) -> BTreeSet<String> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT e, a, v, g, valid_from, valid_to FROM facts \
             WHERE op = 1 AND retracted_tx IS NULL",
        )
        .unwrap();
    let rows: Vec<FactRow> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    rows.into_iter()
        .map(|(e, a, v, g, from, to)| {
            let value = match Value::from_bytes(&v).unwrap() {
                Value::Ref(id) => format!("ref:{}", store.resolve(id).unwrap()),
                Value::Bytes(b) => format!("bytes:{b:?}"),
                other => format!("{other:?}"),
            };
            let graph = if g == 0 {
                "ROOT".to_string()
            } else {
                store.resolve(g).unwrap()
            };
            format!(
                "{graph} {} {} {value} [{from}, {to:?})",
                store.resolve(e).unwrap(),
                store.resolve(a).unwrap()
            )
        })
        .collect()
}

/// Rebuild a stored value from its feed JSON. What the feed leaves out cannot
/// be invented here: a `bytes_len` stub replays as empty bytes.
fn value_of(replica: &Store, json: &Json) -> Value {
    match json {
        Json::String(s) => Value::Str(s.clone()),
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) if n.is_i64() => Value::Int(n.as_i64().unwrap()),
        Json::Number(n) => Value::Float(n.as_f64().unwrap()),
        Json::Object(o) if o.contains_key("ref") => {
            Value::Ref(replica.intern(o["ref"].as_str().unwrap()).unwrap())
        }
        Json::Object(o) if o.contains_key("lang") => Value::Lang {
            lexical: o["value"].as_str().unwrap().into(),
            lang: o["lang"].as_str().unwrap().into(),
        },
        Json::Object(o) if o.contains_key("datatype") => Value::Typed {
            lexical: o["value"].as_str().unwrap().into(),
            datatype: o["datatype"].as_str().unwrap().into(),
        },
        Json::Object(o) if o.contains_key("base64") => {
            use base64::Engine as _;
            Value::Bytes(
                base64::engine::general_purpose::STANDARD
                    .decode(o["base64"].as_str().unwrap())
                    .unwrap(),
            )
        }
        Json::Object(o) if o.contains_key("bytes_len") => Value::Bytes(Vec::new()),
        other => panic!("unreplayable value {other}"),
    }
}

/// Replay the whole feed, one source transaction per replica transaction.
/// Valid time comes from the record when the feed carries it, else from the
/// record's commit timestamp, which is all a consumer has today.
fn replay(source: &Store) -> Store {
    replay_onto(source, 0, Store::open_in_memory().unwrap())
}

/// Tail the feed from `since` into `replica`, as a follower does after a
/// bootstrap.
pub(super) fn replay_onto(source: &Store, since: i64, mut replica: Store) -> Store {
    let page = source
        .changes_after(since, usize::MAX >> 1, Capture::OldAndNewValues, None)
        .unwrap();
    let mut by_tx: Vec<(i64, Vec<&Json>)> = Vec::new();
    for record in &page.records {
        let tx = record["tx"].as_i64().unwrap();
        match by_tx.last_mut() {
            Some((last, list)) if *last == tx => list.push(record),
            _ => by_tx.push((tx, vec![record])),
        }
    }
    for (_, records) in by_tx {
        let timestamp = records[0]["timestamp"].as_str().unwrap().to_string();
        apply_tx(&mut replica, &records, &timestamp);
    }
    replica
}

/// Apply one group of records as one replica transaction. A record without
/// its own `valid_from` takes `timestamp`.
pub(super) fn apply_tx(replica: &mut Store, records: &[&Json], timestamp: &str) {
    let graph_name = records[0]["graph"].as_str().unwrap().to_string();
    let datums: Vec<Datum> = records
        .iter()
        .map(|r| {
            let e = replica.intern(r["entity"].as_str().unwrap()).unwrap();
            let a = replica.intern(r["attribute"].as_str().unwrap()).unwrap();
            let from = r["valid_from"].as_str().unwrap_or(timestamp).to_string();
            let mut d = match r["op"].as_str().unwrap() {
                "retract" => datum(e, a, value_of(replica, &r["old_value"]), &from, Op::Retract),
                _ => datum(e, a, value_of(replica, &r["value"]), &from, Op::Assert),
            };
            // A retract names the fact it closes; its own row's valid time
            // describes the closing, not an interval to replay.
            if d.op == Op::Assert {
                d.valid_to = r["valid_to"].as_str().map(String::from);
            }
            d
        })
        .collect();
    if graph_name == "ROOT" {
        replica.transact(&datums, timestamp, None, None).unwrap();
    } else {
        let g = replica.graph_create(&graph_name).unwrap();
        replica
            .transact_to_graph(&datums, timestamp, None, None, g)
            .unwrap();
    }
}

fn assert_replica_equals_source(source: &Store) {
    let replica = replay(source);
    let (want, got) = (live_digest(source), live_digest(&replica));
    assert!(!want.is_empty(), "fixture wrote nothing live");
    let missing: Vec<_> = want.difference(&got).collect();
    let extra: Vec<_> = got.difference(&want).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "replica diverges from source\n  missing: {missing:#?}\n  extra: {extra:#?}"
    );
}

const TS1: &str = "2026-04-01T00:00:00Z";
const TS2: &str = "2026-04-02T00:00:00Z";

/// Positive control: asserts, an explicit retract and a graph replay exactly.
#[test]
fn control_plain_asserts_and_retracts_replay_exactly() {
    let mut source = Store::open_in_memory().unwrap();
    let e = source.intern("http://example.org/korimako").unwrap();
    let a = source.intern("http://example.org/cpuCores").unwrap();
    let n = source.intern("http://example.org/name").unwrap();
    source
        .transact(
            &[
                datum(e, a, Value::Int(4), TS1, Op::Assert),
                datum(e, n, Value::Str("korimako".into()), TS1, Op::Assert),
            ],
            TS1,
            None,
            None,
        )
        .unwrap();
    source
        .transact(
            &[datum(e, a, Value::Int(4), TS2, Op::Retract)],
            TS2,
            None,
            None,
        )
        .unwrap();
    assert_replica_equals_source(&source);
}

/// F3: change records carry no valid time, so a fact asserted with a valid
/// time other than its commit time replays at the wrong time.
#[test]
fn f3_valid_time_survives_replay() {
    let mut source = Store::open_in_memory().unwrap();
    let e = source.intern("http://example.org/lease").unwrap();
    let a = source.intern("http://example.org/holder").unwrap();
    let mut bounded = datum(
        e,
        a,
        Value::Str("globex".into()),
        "2024-01-01T00:00:00Z",
        Op::Assert,
    );
    bounded.valid_to = Some("2024-12-31T00:00:00Z".into());
    source
        .transact(
            &[
                datum(
                    e,
                    a,
                    Value::Str("acme".into()),
                    "2025-01-01T00:00:00Z",
                    Op::Assert,
                ),
                bounded,
            ],
            TS1,
            None,
            None,
        )
        .unwrap();
    assert_replica_equals_source(&source);
}

/// F4: a byte value is reduced to its length, so the replica cannot hold it.
#[test]
fn f4_byte_values_replay_losslessly() {
    let mut source = Store::open_in_memory().unwrap();
    let e = source.intern("http://example.org/blob").unwrap();
    let a = source.intern("http://example.org/body").unwrap();
    source
        .transact(
            &[datum(
                e,
                a,
                Value::Bytes(vec![0, 159, 146, 150]),
                TS1,
                Op::Assert,
            )],
            TS1,
            None,
            None,
        )
        .unwrap();
    assert_replica_equals_source(&source);
}

/// A record must not change after it is served. A later retract closes the
/// asserted row in place; the assert's record still reads as it did at its tx.
#[test]
fn an_assert_record_reads_the_same_before_and_after_a_later_retract() {
    let mut source = Store::open_in_memory().unwrap();
    let e = source.intern("http://example.org/korimako").unwrap();
    let a = source.intern("http://example.org/cpuCores").unwrap();
    let tx1 = source
        .transact(
            &[datum(e, a, Value::Int(4), TS1, Op::Assert)],
            TS1,
            None,
            None,
        )
        .unwrap();
    let before = source
        .changes_after(tx1 - 1, 1, Capture::OldAndNewValues, None)
        .unwrap()
        .records;
    source
        .transact(
            &[datum(e, a, Value::Int(4), TS2, Op::Retract)],
            TS2,
            None,
            None,
        )
        .unwrap();
    let after = source
        .changes_after(tx1 - 1, 1, Capture::OldAndNewValues, None)
        .unwrap()
        .records;
    assert_eq!(before, after);
    assert!(after[0]["valid_to"].is_null(), "{after:#?}");
}

/// F1: a functional-property supersede closes the old value with an UPDATE and
/// writes no retract record, so the replica keeps both values.
#[cfg(feature = "owl")]
#[test]
fn f1_functional_supersede_replays_as_one_current_value() {
    const ONT: &str = "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
                       @prefix ex: <http://example.org/> .\n\
                       ex:contentHash a owl:DatatypeProperty, owl:FunctionalProperty .\n";
    let mut source = Store::open_in_memory().unwrap();
    source.owl_config_mut().validate_on_write = true;
    source.load_ontology("t", ONT, TS1).unwrap();
    source.invalidate_owl_cache();
    let e = source.intern("http://example.org/doc1").unwrap();
    let a = source.intern("http://example.org/contentHash").unwrap();
    source
        .transact(
            &[datum(e, a, Value::Str("aaa".into()), TS1, Op::Assert)],
            TS1,
            None,
            None,
        )
        .unwrap();
    source
        .transact(
            &[datum(e, a, Value::Str("bbb".into()), TS2, Op::Assert)],
            TS2,
            None,
            None,
        )
        .unwrap();
    assert_replica_equals_source(&source);
}

/// F5: a page must name its source store and log epoch, so a follower can
/// detect a restored or cloned primary.
#[test]
fn f5_pages_carry_source_identity() {
    let source = Store::open_in_memory().unwrap();
    let page = source
        .changes_after(0, 10, Capture::NewValues, None)
        .unwrap()
        .to_json();
    assert!(page["source"]["store_id"].is_string(), "{page}");
    assert!(page["source"]["log_epoch"].is_string(), "{page}");
}

/// The epoch is minted once and kept across reopen; a store that predates it
/// gets one on its first open; minting a new one changes what pages report.
#[test]
fn f5_log_epoch_is_stable_until_minted_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let path = path.to_str().unwrap();
    let (store_id, first) = {
        let store = Store::open(path).unwrap();
        (store.store_id().unwrap(), store.log_epoch().unwrap())
    };
    let store = Store::open(path).unwrap();
    assert_eq!(store.log_epoch().unwrap(), first, "epoch changed on reopen");
    let page = store
        .changes_after(0, 1, Capture::NewValues, None)
        .unwrap()
        .to_json();
    assert_eq!(page["source"]["store_id"], store_id);
    assert_eq!(page["source"]["log_epoch"], first);

    let minted = store.mint_log_epoch("test").unwrap();
    assert_ne!(minted, first);
    let page = store
        .changes_after(0, 1, Capture::NewValues, None)
        .unwrap()
        .to_json();
    assert_eq!(page["source"]["log_epoch"], minted);
    assert_eq!(
        page["source"]["store_id"], store_id,
        "store identity is not the epoch"
    );

    store.conn.execute("DELETE FROM log_epoch", []).unwrap();
    drop(store);
    let upgraded = Store::open(path).unwrap();
    let late = upgraded.log_epoch().unwrap();
    assert!(late.starts_with("urn:uuid:"), "{late}");
    assert_ne!(late, minted);
}
