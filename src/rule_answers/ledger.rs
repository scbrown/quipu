//! Bounded current ROOT ledger reads, before invoking native seal verification.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;

use crate::{Error, Result, Store, Value};

pub(super) fn invalid(message: &str) -> Error {
    Error::InvalidValue(format!("rule judgment: {message}"))
}

pub(super) fn entity(store: &Store, iri: &str) -> Result<Vec<(String, Value)>> {
    let Some(id) = store.lookup(iri)? else {
        return Err(invalid("missing referenced entity"));
    };
    let mut statement = store.conn.prepare(
        "SELECT a, length(v), v FROM facts WHERE g = 0 AND e = ?1 AND op = 1 \
         AND valid_to IS NULL ORDER BY a, v, tx LIMIT 65",
    )?;
    let mut rows = statement.query(params![id])?;
    let mut physical_count = 0;
    let mut bytes = 0;
    let mut facts = BTreeMap::new();
    while let Some(row) = rows.next()? {
        physical_count += 1;
        let length: i64 = row.get(1)?;
        let length = usize::try_from(length).map_err(|_| invalid("invalid entity byte length"))?;
        bytes += length;
        if physical_count > 64 || bytes > 16384 {
            return Err(invalid("entity size cap exceeded"));
        }
        let blob: Vec<u8> = row.get(2)?;
        let predicate = store.resolve(row.get(0)?)?;
        let value = Value::from_bytes(&blob)?;
        facts.insert((predicate, value.term_key()), value);
    }
    Ok(facts
        .into_iter()
        .map(|((predicate, _), value)| (predicate, value))
        .collect())
}

pub(super) fn subjects(store: &Store, predicate: &str, value: &Value) -> Result<Vec<String>> {
    subjects_matching(store, predicate, value, false)
}

/// Retained markers locate withdrawn judgments too. Removing a marker cannot
/// hide an admitted disposition behind a fresh `none_recorded` answer.
pub(super) fn retained_subjects(
    store: &Store,
    predicate: &str,
    value: &Value,
) -> Result<Vec<String>> {
    subjects_matching(store, predicate, value, true)
}

fn subjects_matching(
    store: &Store,
    predicate: &str,
    value: &Value,
    retained: bool,
) -> Result<Vec<String>> {
    let Some(attribute) = store.lookup(predicate)? else {
        return Ok(Vec::new());
    };
    let mut subjects = BTreeSet::new();
    let mut physical_count = 0;
    for blob in value.physical_aliases() {
        let sql = if retained {
            "SELECT e FROM facts WHERE g = 0 AND a = ?1 AND v = ?2 \
             AND op = 1 ORDER BY e, tx LIMIT 17"
        } else {
            "SELECT e FROM facts WHERE g = 0 AND a = ?1 AND v = ?2 \
             AND op = 1 AND valid_to IS NULL ORDER BY e, tx LIMIT 17"
        };
        let mut statement = store.conn.prepare(sql)?;
        let mut rows = statement.query(params![attribute, blob])?;
        while let Some(row) = rows.next()? {
            physical_count += 1;
            if physical_count > 16 {
                return Err(invalid("ledger reference cap exceeded"));
            }
            let iri = store.resolve(row.get(0)?)?;
            oxrdf::NamedNode::new(&iri).map_err(|_| invalid("non-IRI ledger identity"))?;
            subjects.insert(iri);
        }
    }
    Ok(subjects.into_iter().collect())
}

pub(super) fn scalar(facts: &[(String, Value)], predicate: &str) -> Result<String> {
    let values: Vec<_> = facts.iter().filter(|(p, _)| p == predicate).collect();
    match values.as_slice() {
        [(_, Value::Str(value))] => Ok(value.clone()),
        _ => Err(invalid("required string is missing or ambiguous")),
    }
}

pub(super) fn reference(facts: &[(String, Value)], predicate: &str) -> Result<i64> {
    let values: Vec<_> = facts.iter().filter(|(p, _)| p == predicate).collect();
    match values.as_slice() {
        [(_, Value::Ref(value))] => Ok(*value),
        _ => Err(invalid("required reference is missing or ambiguous")),
    }
}

pub(super) fn ns(local: &str) -> String {
    format!("{}{local}", crate::namespace::DEFAULT_BASE_NS)
}
