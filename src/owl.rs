//! OWL 2 RL ontology layer — class hierarchy, materialization, and validation.
//!
//! Parses OWL ontologies from Turtle (they're just RDF with OWL vocabulary),
//! extracts axioms, materializes entailments into the EAVT fact log, and
//! validates write-time constraints (disjoint classes, functional properties).
//!
//! Built on top of the existing `oxttl` RDF parser and Quipu store rather than
//! an external OWL library, keeping the dependency footprint small.

#[path = "owl_parse.rs"]
mod owl_parse;

#[path = "owl_materialize.rs"]
mod owl_materialize;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;

use crate::error::{Error, Result};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

use owl_parse::{extract_axioms, parse_turtle_triples};

/// Transitive closure of a subclass/subproperty relation: term → all
/// transitive supers. Shared with `crate::explain`'s axiom-family resolution.
pub(crate) fn transitive_closure(pairs: &[(String, String)]) -> HashMap<String, HashSet<String>> {
    owl_parse::transitive_closure(pairs)
}

// ── OWL / RDF vocabulary IRIs ────────────────────────────────────────

const OWL_DISJOINT_WITH: &str = "http://www.w3.org/2002/07/owl#disjointWith";
const OWL_INVERSE_OF: &str = "http://www.w3.org/2002/07/owl#inverseOf";
const OWL_FUNCTIONAL_PROPERTY: &str = "http://www.w3.org/2002/07/owl#FunctionalProperty";
const OWL_SYMMETRIC_PROPERTY: &str = "http://www.w3.org/2002/07/owl#SymmetricProperty";
const OWL_TRANSITIVE_PROPERTY: &str = "http://www.w3.org/2002/07/owl#TransitiveProperty";
const OWL_EQUIVALENT_CLASS: &str = "http://www.w3.org/2002/07/owl#equivalentClass";
const OWL_EQUIVALENT_PROPERTY: &str = "http://www.w3.org/2002/07/owl#equivalentProperty";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUB_CLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_SUB_PROPERTY_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";
const RDFS_DOMAIN: &str = "http://www.w3.org/2000/01/rdf-schema#domain";
const RDFS_RANGE: &str = "http://www.w3.org/2000/01/rdf-schema#range";

// ── Types ────────────────────────────────────────────────────────────

/// An OWL ontology parsed from Turtle.
#[derive(Debug, Clone)]
pub struct Ontology {
    /// All raw triples from the ontology document.
    triples: Vec<(String, String, String)>,
    /// Extracted axioms.
    pub axioms: Axioms,
}

/// Extracted OWL/RDFS axioms from an ontology.
#[derive(Debug, Clone, Default)]
pub struct Axioms {
    /// `rdfs:subClassOf` pairs: (sub, super).
    pub subclass_of: Vec<(String, String)>,
    /// `rdfs:subPropertyOf` pairs: (sub, super).
    pub subproperty_of: Vec<(String, String)>,
    /// `owl:disjointWith` pairs (stored both directions).
    pub disjoint_with: HashSet<(String, String)>,
    /// `owl:inverseOf` pairs (stored both directions).
    pub inverse_of: Vec<(String, String)>,
    /// IRIs of `owl:FunctionalProperty` instances.
    pub functional_properties: HashSet<String>,
    /// IRIs of `owl:SymmetricProperty` instances.
    pub symmetric_properties: HashSet<String>,
    /// IRIs of `owl:TransitiveProperty` instances.
    pub transitive_properties: HashSet<String>,
    /// `owl:equivalentClass` pairs.
    pub equivalent_classes: Vec<(String, String)>,
    /// `owl:equivalentProperty` pairs.
    pub equivalent_properties: Vec<(String, String)>,
    /// `rdfs:domain` pairs: (property, class).
    pub domains: Vec<(String, String)>,
    /// `rdfs:range` pairs: (property, class).
    pub ranges: Vec<(String, String)>,
}

/// Report from materialization: counts of inferred facts by type.
#[derive(Debug, Clone, Default)]
pub struct MaterializeReport {
    /// Subclass type inferences (instance of parent class).
    pub subclass_inferences: usize,
    /// `rdfs:subPropertyOf` inferences (fact restated under the superproperty).
    pub sub_property_inferences: usize,
    /// Inverse property inferences.
    pub inverse_inferences: usize,
    /// Symmetric property inferences.
    pub symmetric_inferences: usize,
    /// Transitive property closure inferences.
    pub transitive_inferences: usize,
    /// `owl:equivalentProperty` inferences (fact restated under the equivalent).
    pub equivalent_property_inferences: usize,
    /// Equivalent class type inferences.
    pub equivalent_class_inferences: usize,
    /// Domain/range type inferences.
    pub domain_range_inferences: usize,
    /// Total new facts materialized.
    pub total: usize,
    /// Derivation passes actually run (aegis-2dp8e2).
    pub passes: usize,
    /// Premise facts READ across all passes (aegis-2dp8e2).
    ///
    /// This is the quantity the semi-naive change exists to bound, and it is
    /// asserted on directly rather than timed: a wall-clock benchmark is flaky
    /// under CI load and would measure the machine as much as the algorithm.
    /// Facts-read is deterministic, and it is what actually scaled with store
    /// size — the naive path read every current fact on every pass.
    pub premise_facts_read: usize,
    /// The pass budget was hit with work still pending, so the closure is
    /// PARTIAL. Reported rather than left silent: a fixpoint that stopped early
    /// is indistinguishable from one that finished, and the difference is a
    /// wrong answer that looks complete.
    pub pass_budget_exhausted: bool,
}

/// A constraint violation from OWL validation.
#[derive(Debug, Clone)]
pub struct OwlViolation {
    /// The kind of violation.
    pub kind: ViolationKind,
    /// The focus entity IRI.
    pub focus_node: String,
    /// Human-readable message.
    pub message: String,
}

/// The kind of OWL constraint violation.
#[derive(Debug, Clone)]
pub enum ViolationKind {
    /// Entity has types from disjoint classes.
    DisjointClass {
        /// The first class.
        class_a: String,
        /// The second (disjoint) class.
        class_b: String,
    },
    /// Functional property has multiple values.
    FunctionalProperty {
        /// The property IRI.
        property: String,
        /// Number of values found.
        count: usize,
    },
}

// ── Ontology loading ─────────────────────────────────────────────────

impl Ontology {
    /// Parse an OWL ontology from Turtle format.
    pub fn from_turtle(turtle: &str) -> Result<Self> {
        let triples = parse_turtle_triples(turtle)?;
        let axioms = extract_axioms(&triples);
        Ok(Self { triples, axioms })
    }

    /// Parse from a reader.
    pub fn from_reader(mut reader: impl Read) -> Result<Self> {
        let mut turtle = String::new();
        reader
            .read_to_string(&mut turtle)
            .map_err(|e| Error::InvalidValue(format!("read error: {e}")))?;
        Self::from_turtle(&turtle)
    }

    /// Summary of axiom counts for feedback.
    pub fn axiom_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "subclass_of": self.axioms.subclass_of.len(),
            "subproperty_of": self.axioms.subproperty_of.len(),
            "disjoint_with": self.axioms.disjoint_with.len() / 2,
            "inverse_of": self.axioms.inverse_of.len() / 2,
            "functional_properties": self.axioms.functional_properties.len(),
            "symmetric_properties": self.axioms.symmetric_properties.len(),
            "transitive_properties": self.axioms.transitive_properties.len(),
            "equivalent_classes": self.axioms.equivalent_classes.len(),
            "equivalent_properties": self.axioms.equivalent_properties.len(),
            "domains": self.axioms.domains.len(),
            "ranges": self.axioms.ranges.len(),
            "total_triples": self.triples.len(),
        })
    }

    /// Validate a set of proposed facts against OWL constraints.
    ///
    /// Checks:
    /// - Disjoint class violations: an entity cannot be typed with two disjoint classes.
    /// - Functional property violations: a functional property cannot have multiple values.
    ///
    /// Returns violations found. Empty vec means the facts are valid.
    pub fn validate(&self, store: &Store, proposed: &[Datum]) -> Result<Vec<OwlViolation>> {
        let mut violations = Vec::new();
        let rdf_type_id = store.intern(RDF_TYPE)?;

        // Collect proposed type assertions keyed by entity.
        let mut entity_types: HashMap<i64, BTreeSet<i64>> = HashMap::new();
        for d in proposed {
            if d.op == Op::Assert
                && d.attribute == rdf_type_id
                && let Value::Ref(class_id) = &d.value
            {
                entity_types.entry(d.entity).or_default().insert(*class_id);
            }
        }

        // Merge with existing types from the store.
        for (&entity_id, new_types) in &mut entity_types {
            let existing = store.entity_facts(entity_id)?;
            for f in &existing {
                if f.attribute == rdf_type_id
                    && let Value::Ref(class_id) = &f.value
                {
                    new_types.insert(*class_id);
                }
            }
        }

        // Check disjoint class constraints.
        for (&entity_id, types) in &entity_types {
            let type_iris: Vec<(i64, String)> = types
                .iter()
                .filter_map(|&id| store.resolve(id).ok().map(|iri| (id, iri)))
                .collect();

            for i in 0..type_iris.len() {
                for j in (i + 1)..type_iris.len() {
                    let a = &type_iris[i].1;
                    let b = &type_iris[j].1;
                    if self.axioms.disjoint_with.contains(&(a.clone(), b.clone())) {
                        let focus = store
                            .resolve(entity_id)
                            .unwrap_or_else(|_| format!("entity:{entity_id}"));
                        violations.push(OwlViolation {
                            kind: ViolationKind::DisjointClass {
                                class_a: a.clone(),
                                class_b: b.clone(),
                            },
                            focus_node: focus.clone(),
                            message: format!(
                                "{focus} cannot be both <{a}> and <{b}> (disjoint classes)"
                            ),
                        });
                    }
                }
            }
        }

        // Check functional property constraints.
        let mut prop_values: HashMap<(i64, i64), Vec<Value>> = HashMap::new();
        for d in proposed {
            if d.op == Op::Assert {
                prop_values
                    .entry((d.entity, d.attribute))
                    .or_default()
                    .push(d.value.clone());
            }
        }
        // Merge with existing facts.
        let keys: Vec<(i64, i64)> = prop_values.keys().copied().collect();
        for (entity_id, attr_id) in keys {
            let existing = store.entity_facts(entity_id)?;
            for f in &existing {
                if f.attribute == attr_id {
                    prop_values
                        .entry((entity_id, attr_id))
                        .or_default()
                        .push(f.value.clone());
                }
            }
        }
        // Dedup values and check count.
        for (&(entity_id, attr_id), values) in &prop_values {
            let Ok(attr_iri) = store.resolve(attr_id) else {
                continue;
            };
            if !self.axioms.functional_properties.contains(&attr_iri) {
                continue;
            }
            // Deduplicate values for counting.
            let unique: HashSet<Vec<u8>> = values.iter().map(Value::to_bytes).collect();
            if unique.len() > 1 {
                let focus = store
                    .resolve(entity_id)
                    .unwrap_or_else(|_| format!("entity:{entity_id}"));
                violations.push(OwlViolation {
                    kind: ViolationKind::FunctionalProperty {
                        property: attr_iri.clone(),
                        count: unique.len(),
                    },
                    focus_node: focus.clone(),
                    message: format!(
                        "{focus} has {} values for functional property <{attr_iri}> (max 1)",
                        unique.len()
                    ),
                });
            }
        }

        Ok(violations)
    }
}

#[cfg(test)]
#[path = "owl_tests.rs"]
mod tests;
