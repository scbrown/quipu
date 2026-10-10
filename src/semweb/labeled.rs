//! Stream the ROOT label/type scan without materializing SPARQL binding tables.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::sparql::ProgressGuard;
use crate::time::{Deadline, Stopwatch};
use crate::{Error, Result, Store, types::Value};

use super::LabeledEntity;

/// Fetch current ROOT entities with plain-string labels and their asserted types.
///
/// Named graphs (including attached packs) are deliberately excluded, just as
/// they are from a SPARQL query without an explicit dataset.
pub fn fetch_labeled_entities(store: &Store) -> Result<Vec<LabeledEntity>> {
    fetch_labeled_entities_until(store, None)
}

/// Fetch labeled entities under a deadline, retaining only the type lookup and
/// final entities. A cold Spotlight fill must not also retain two whole tables
/// of variable-name/value maps for the same corpus.
pub fn fetch_labeled_entities_until(
    store: &Store,
    deadline: Option<Deadline>,
) -> Result<Vec<LabeledEntity>> {
    let started = Stopwatch::start();
    let deadline = deadline.or_else(crate::time::request_deadline).or_else(|| {
        let ms = store.search_config().query_timeout_ms;
        (ms > 0).then(|| Deadline::after_millis(ms))
    });
    let check = || {
        if deadline.is_some_and(|d| d.passed()) {
            Err(Error::QueryTimeout {
                elapsed_ms: started.elapsed_ms(),
                limit_ms: deadline
                    .map(|d| d.millis_from(&started))
                    .unwrap_or_default(),
            })
        } else {
            Ok(())
        }
    };
    check()?;
    let _guard = deadline
        .map(|d| ProgressGuard::install(&store.conn, d))
        .transpose()?;
    let result = fetch(store, &check);
    // Normalize SQLITE_INTERRUPT, and check even when a scan returned no rows.
    check()?;
    result
}

fn fetch(store: &Store, check: &impl Fn() -> Result<()>) -> Result<Vec<LabeledEntity>> {
    // The connection's TEMP alias table is stable for the duration of a read.
    // Load it once instead of issuing a canonical-id query for every row.
    let mut aliases = HashMap::new();
    if !store.attachments().is_empty() {
        let mut stmt = store.prepare("SELECT alias_id, canonical_id FROM term_alias")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            check()?;
            aliases.insert(row.get::<_, i64>(0)?, row.get::<_, i64>(1)?);
        }
    }
    let mut type_names: HashMap<i64, Arc<str>> = HashMap::new();
    let mut types: HashMap<i64, Vec<Arc<str>>> = HashMap::new();
    scan(
        store,
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        check,
        &aliases,
        false,
        |entity, value, _| {
            if let Value::Ref(id) = value {
                let name = type_names
                    .entry(id)
                    .or_insert_with(|| store.resolve(id).unwrap_or_default().into());
                types.entry(entity).or_default().push(name.clone());
            }
            Ok(())
        },
    )?;
    let mut entities = Vec::new();
    scan(
        store,
        "http://www.w3.org/2000/01/rdf-schema#label",
        check,
        &aliases,
        true,
        |entity, value, iri| {
            let Value::Str(label) = value else {
                return Ok(());
            };
            let iri: Arc<str> = iri
                .unwrap_or_else(|| store.resolve(entity).unwrap_or_default())
                .into();
            let label: Arc<str> = label.into();
            match types.get(&entity) {
                Some(types) if !types.is_empty() => {
                    entities.extend(types.iter().map(|entity_type| LabeledEntity {
                        iri: iri.clone(),
                        label: label.clone(),
                        entity_type: entity_type.clone(),
                    }));
                }
                _ => entities.push(LabeledEntity {
                    iri,
                    label,
                    entity_type: Arc::from(""),
                }),
            }
            Ok(())
        },
    )?;
    Ok(entities)
}

fn scan(
    store: &Store,
    predicate: &str,
    check: &impl Fn() -> Result<()>,
    aliases: &HashMap<i64, i64>,
    with_iri: bool,
    mut visit: impl FnMut(i64, Value, Option<String>) -> Result<()>,
) -> Result<()> {
    let Some(attribute) = store.lookup(predicate)? else {
        return Ok(());
    };
    // Match the query engine's distinct triples and scan order: Spotlight's
    // adjacent annotation deduplication observes the order of tied types.
    let sql = format!(
        "SELECT DISTINCT e, a, v FROM {} WHERE op = 1 AND a = ?1 \
         AND g = 0 AND valid_to IS NULL",
        store.facts_source()
    );
    // Keep DISTINCT in a subquery so the join only decorates the original
    // scan order. Reading the IRI here avoids filling the per-connection term
    // memo with every label subject during a cold Spotlight fetch.
    let sql = if with_iri {
        format!(
            "SELECT f.e, f.a, f.v, names.iri FROM ({sql}) AS f LEFT JOIN main.terms AS names ON names.id = f.e"
        )
    } else {
        sql
    };
    let mut stmt = store.prepare(&sql)?;
    let mut rows = stmt.query([attribute])?;
    let mut count = 0;
    let mut seen = HashSet::new();
    let cap = store.search_config().max_join_rows;
    while let Some(row) = rows.next()? {
        check()?;
        let raw_entity: i64 = row.get(0)?;
        let entity = aliases.get(&raw_entity).copied().unwrap_or(raw_entity);
        let blob: Vec<u8> = row.get(2)?;
        let value = match Value::from_bytes(&blob)? {
            Value::Ref(id) => Value::Ref(aliases.get(&id).copied().unwrap_or(id)),
            value => value,
        };
        if !seen.insert((entity, value.to_bytes())) {
            continue;
        }
        count += 1;
        if cap > 0 && count > cap {
            return Err(Error::QueryComplexity { limit: cap });
        }
        let iri = if with_iri && raw_entity == entity {
            row.get(3)?
        } else {
            None
        };
        visit(entity, value, iri)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
