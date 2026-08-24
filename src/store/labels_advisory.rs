//! Advisory label declarations — recommended floors and default datasets
//! (quipu #80).
//!
//! Split from [`super::labels`] to keep that module under the file-size
//! ratchet; the discipline is unchanged. Everything here is **advisory**:
//! read and surfaced, never applied. Enforcement reads `[quipu.labels]`
//! config and nothing else.

use crate::error::{Error, Result};
use crate::lattice::Freshness;
use crate::types::{Op, Value};

use super::{Datum, Store};

/// A producer's RECOMMENDED floor for consumers of a layer (quipu #80).
///
/// **Advisory. Never enforced.** See [`Store::recommended_floor`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecommendedFloor {
    /// Minimum freshness the producer considers safe.
    pub min_freshness: Option<Freshness>,
    /// Minimum trust value (an IRI) the producer considers safe.
    pub min_trust: Option<String>,
}

impl RecommendedFloor {
    /// Whether the producer recommended anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min_freshness.is_none() && self.min_trust.is_none()
    }

    /// A one-line rendering for the attach banner, phrased as a recommendation
    /// so it cannot be misread as something the store applied.
    #[must_use]
    pub fn line(&self, graph_iri: &str) -> String {
        let mut parts = Vec::new();
        if let Some(f) = self.min_freshness {
            parts.push(format!("freshness >= {f}"));
        }
        if let Some(t) = &self.min_trust {
            parts.push(format!("trust >= {t}"));
        }
        format!(
            "{graph_iri} RECOMMENDS {} — advisory only; enforcement is the \
             consumer's [quipu.labels] config",
            parts.join(", ")
        )
    }
}

impl Store {
    /// Declare a producer's recommended floor for a graph (quipu #80).
    ///
    /// Written as ordinary meta-graph facts, so it is queryable, bitemporal and
    /// governed exactly as a label is — and, like a label, requires authority
    /// over the meta-graph.
    ///
    /// # Errors
    /// As [`Store::set_graph_label`]; also when nothing is recommended.
    pub fn set_recommended_floor(
        &mut self,
        graph_iri: &str,
        floor: &RecommendedFloor,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<i64> {
        if floor.is_empty() {
            return Err(Error::InvalidValue(format!(
                "recommended floor for '{graph_iri}' declares nothing"
            )));
        }
        let meta_g = self.meta_graph_id()?;
        let subject = self.intern(graph_iri)?;
        let mut datums = Vec::new();
        if let Some(f) = floor.min_freshness {
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(crate::namespace::QUIPU_RECOMMENDS_FRESHNESS)?,
                value: Value::Str(f.as_str().to_string()),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        if let Some(t) = &floor.min_trust {
            let t_term = self.intern(t)?;
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(crate::namespace::QUIPU_RECOMMENDS_TRUST)?,
                value: Value::Ref(t_term),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        self.transact_to_graph(&datums, timestamp, actor, Some("recommended-floor"), meta_g)
    }

    /// Read a graph's recommended floor.
    ///
    /// ⚠️ **This is READ and SURFACED, never applied.** A pack that could
    /// tighten enforcement could `DoS` its consumer; one that could loosen it
    /// could bypass the consumer's own floor. So nothing in the query path
    /// consults this — [`Store::check_label_floor`] reads
    /// `[quipu.labels]` and nothing else, and a test asserts enforcement is
    /// byte-identical with and without a recommendation present.
    pub fn recommended_floor(&self, graph_iri: &str) -> Result<RecommendedFloor> {
        let Some(g) = self.lookup(graph_iri)? else {
            return Ok(RecommendedFloor::default());
        };
        let meta_g = self.meta_graph_id()?;
        let min_freshness = self
            .declared_str(g, crate::namespace::QUIPU_RECOMMENDS_FRESHNESS, meta_g)?
            .and_then(|s| Freshness::parse(&s));
        let min_trust = self.ref_iri(
            self.current_value(g, crate::namespace::QUIPU_RECOMMENDS_TRUST, meta_g)?
                .as_ref(),
        )?;
        Ok(RecommendedFloor {
            min_freshness,
            min_trust,
        })
    }

    /// Declare the dataset a graph expects to be activated with (quipu #80).
    ///
    /// # Errors
    /// As [`Store::set_graph_label`].
    pub fn set_default_dataset(
        &mut self,
        graph_iri: &str,
        dataset_iri: &str,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<i64> {
        let meta_g = self.meta_graph_id()?;
        let subject = self.intern(graph_iri)?;
        let attribute = self.intern(crate::namespace::QUIPU_DEFAULT_DATASET)?;
        let value = Value::Ref(self.intern(dataset_iri)?);
        self.transact_to_graph(
            &[Datum {
                entity: subject,
                attribute,
                value,
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            timestamp,
            actor,
            Some("default-dataset"),
            meta_g,
        )
    }

    /// The dataset IRI a graph expects to be activated with, if declared.
    ///
    /// Advisory like the floor: naming it does not activate it. The ROOT-alone
    /// default survives, and a dataset is never implicitly active (#69).
    pub fn default_dataset(&self, graph_iri: &str) -> Result<Option<String>> {
        let Some(g) = self.lookup(graph_iri)? else {
            return Ok(None);
        };
        let meta_g = self.meta_graph_id()?;
        self.ref_iri(
            self.current_value(g, crate::namespace::QUIPU_DEFAULT_DATASET, meta_g)?
                .as_ref(),
        )
    }
}
