//! Deterministic, local-only label proposals for cross-branch new entities.
use crate::share_merge::Graph;
use oxrdf::Term;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AliasProposal {
    pub ours: String,
    pub theirs: String,
    pub similarity: f64,
}

/// Minimum Jaro-Winkler similarity between normalized labels for a proposal.
pub(crate) const ALIAS_THRESHOLD: f64 = 0.90;
/// At most this many candidates per new entity, best first.
const MAX_PER_ENTITY: usize = 5;

/// Per IRI subject: its `rdf:type` set and its normalized (whitespace-collapsed,
/// lowercase) `rdfs:label` set.
pub(crate) type Entities = BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>;

pub(crate) fn entities(graph: &Graph) -> Entities {
    let mut out = BTreeMap::new();
    for t in graph {
        // Blank-node labels are snapshot-local, not entity identities.
        if !matches!(t.subject, oxrdf::NamedOrBlankNode::NamedNode(_)) {
            continue;
        }
        let entry = out
            .entry(t.subject.to_string())
            .or_insert_with(|| (BTreeSet::new(), BTreeSet::new()));
        match (t.predicate.as_str(), &t.object) {
            ("http://www.w3.org/1999/02/22-rdf-syntax-ns#type", Term::NamedNode(n)) => {
                entry.0.insert(n.to_string());
            }
            ("http://www.w3.org/2000/01/rdf-schema#label", Term::Literal(l)) => {
                entry.1.insert(
                    l.value()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase(),
                );
            }
            _ => {}
        }
    }
    out
}

/// Best label similarity between two entities, or `None` when they share no
/// `rdf:type` (the proposer only ever compares same-type entities).
pub(crate) fn similarity(
    a: &(BTreeSet<String>, BTreeSet<String>),
    b: &(BTreeSet<String>, BTreeSet<String>),
) -> Option<f64> {
    if a.0.is_disjoint(&b.0) {
        return None;
    }
    Some(
        a.1.iter()
            .flat_map(|x| b.1.iter().map(move |y| strsim::jaro_winkler(x, y)))
            .fold(0.0_f64, f64::max),
    )
}

/// Keep the best [`MAX_PER_ENTITY`] candidates, highest similarity first.
pub(crate) fn best(mut candidates: Vec<AliasProposal>) -> Vec<AliasProposal> {
    candidates.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then(a.theirs.cmp(&b.theirs))
    });
    candidates.truncate(MAX_PER_ENTITY);
    candidates
}

pub(crate) fn propose(base: &Graph, ours: &Graph, theirs: &Graph) -> Vec<AliasProposal> {
    let (base, ours, theirs) = (entities(base), entities(ours), entities(theirs));
    let mut proposals = Vec::new();
    for (o, oe) in &ours {
        if base.contains_key(o) || theirs.contains_key(o) {
            continue;
        }
        let mut candidates = Vec::new();
        for (t, te) in &theirs {
            if base.contains_key(t) || ours.contains_key(t) {
                continue;
            }
            if let Some(score) = similarity(oe, te)
                && score >= ALIAS_THRESHOLD
            {
                candidates.push(AliasProposal {
                    ours: o.clone(),
                    theirs: t.clone(),
                    similarity: score,
                });
            }
        }
        proposals.extend(best(candidates));
    }
    proposals
}
