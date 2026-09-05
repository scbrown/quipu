//! Store-backed enumeration: what concepts does one named graph hold?
//!
//! [`propose`](super::propose::propose) is a pure core over two concept lists.
//! This is the half that reads a store, and it is deliberately the only part of
//! candidate generation that touches SQL — so the properties that matter
//! (determinism, suppression) stay testable without a database.
//!
//! ## Graph-scoped, because alignment is about two graphs
//!
//! Every query here carries `g = ?`. `resolve_entity` cannot express this
//! candidate space, which is the reason alignment enumerates at all rather than
//! reusing resolution wholesale.
//!
//! ## A concept whose name is ambiguous is REPORTED, never guessed
//!
//! `propose` matches on one label per concept. An entity carrying two
//! `rdfs:label`s in the same graph therefore has no single answer, and the
//! three ways out are not equal:
//!
//! * pick one (first, shortest, lexicographically least) — alignment then
//!   matches on a name the operator cannot predict, in a design whose whole
//!   premise is that a human can argue with the rule;
//! * emit one concept per label — [`propose`](super::propose::propose) loops
//!   over the lists pairwise, so the same pair would be proposed more than
//!   once, with different scores, and `MappingSet::sort` would not merge them;
//! * exclude it and say so.
//!
//! The third is the only one that neither invents a judgement nor corrupts the
//! candidate set, so [`Enumeration::ambiguous`] carries those entities with all
//! their labels and a caller can surface them. This is the same shape as
//! `verify`'s three-state verdict: "I could not decide" is a distinct outcome
//! from "there was nothing", and collapsing it into either is the bug.

use std::collections::BTreeMap;

use crate::error::Result;
use crate::namespace::{RDF_TYPE, RDFS_LABEL};
use crate::store::Store;
use crate::types::Value;

use super::propose::Concept;

/// One entity that could not be enumerated because its name is ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    /// The entity's IRI.
    pub iri: String,
    /// Every `rdfs:label` it carries in this graph, sorted.
    pub labels: Vec<String>,
}

/// What one graph holds, plus what could not be decided about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Enumeration {
    /// Concepts with exactly one label, sorted by IRI.
    pub concepts: Vec<Concept>,
    /// Entities excluded because they carry more than one label. Never guessed.
    pub ambiguous: Vec<Ambiguous>,
}

impl Enumeration {
    /// Did this enumeration examine anything at all?
    ///
    /// An empty graph and a graph of nothing but ambiguous entities are
    /// different findings, and a caller that reports "0 concepts" for both
    /// hides the second.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.concepts.is_empty() && self.ambiguous.is_empty()
    }
}

/// Enumerate the concepts of one named graph.
///
/// Returns `Ok` with an empty [`Enumeration`] when the graph IRI is not
/// registered — an absent graph holds no concepts, which is not an error, and
/// the caller distinguishes it by asking the store whether the graph exists.
///
/// # Errors
///
/// Propagates store errors.
pub fn enumerate(store: &Store, graph_iri: &str) -> Result<Enumeration> {
    let Some(graph) = store.lookup(graph_iri)? else {
        return Ok(Enumeration::default());
    };
    let Some(label_attr) = store.lookup(RDFS_LABEL)? else {
        // Nothing in this store has ever carried a label.
        return Ok(Enumeration::default());
    };

    // Labels first: an entity with no label is not a concept alignment can
    // match on, so the label query defines the candidate set.
    let mut labels: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    {
        let mut stmt = store.conn.prepare(
            "SELECT e, v FROM facts \
             WHERE a = ?1 AND g = ?2 AND op = 1 AND valid_to IS NULL",
        )?;
        let rows = stmt.query_map([label_attr, graph], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (entity, raw) = row?;
            if let Ok(Value::Str(s)) = Value::from_bytes(&raw) {
                let slot = labels.entry(entity).or_default();
                if !slot.contains(&s) {
                    slot.push(s);
                }
            }
        }
    }

    // Types are IRI-valued, so they arrive as `Value::Ref` and must be
    // resolved. Decoding them as `Value::Str` yields no types at all, and a
    // link spec with `require_shared_type` would then silently propose
    // nothing — an empty result that reads exactly like a correct one.
    let mut types: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    if let Some(type_attr) = store.lookup(RDF_TYPE)? {
        let mut stmt = store.conn.prepare(
            "SELECT e, v FROM facts \
             WHERE a = ?1 AND g = ?2 AND op = 1 AND valid_to IS NULL",
        )?;
        let rows = stmt.query_map([type_attr, graph], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (entity, raw) = row?;
            let iri = match Value::from_bytes(&raw) {
                Ok(Value::Ref(id)) => store.resolve(id)?,
                Ok(Value::Str(s)) => s,
                _ => continue,
            };
            let slot = types.entry(entity).or_default();
            if !slot.contains(&iri) {
                slot.push(iri);
            }
        }
    }

    let mut out = Enumeration::default();
    for (entity, mut entity_labels) in labels {
        entity_labels.sort();
        let iri = store.resolve(entity)?;
        if entity_labels.len() > 1 {
            out.ambiguous.push(Ambiguous {
                iri,
                labels: entity_labels,
            });
            continue;
        }
        let mut entity_types = types.remove(&entity).unwrap_or_default();
        entity_types.sort();
        out.concepts.push(Concept {
            iri,
            // `len() == 1` above; `swap_remove` avoids cloning the only label.
            label: entity_labels.swap_remove(0),
            types: entity_types,
        });
    }

    // BTreeMap orders by interned id, which is insertion order in disguise and
    // says nothing about the graph. Sort by IRI so the bytes do not depend on
    // the order facts were written — the same property `propose` is tested for.
    out.concepts.sort_by(|a, b| a.iri.cmp(&b.iri));
    out.ambiguous.sort_by(|a, b| a.iri.cmp(&b.iri));
    Ok(out)
}
