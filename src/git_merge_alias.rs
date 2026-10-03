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

fn entities(graph: &Graph) -> BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> {
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

pub(crate) fn propose(base: &Graph, ours: &Graph, theirs: &Graph) -> Vec<AliasProposal> {
    let (base, ours, theirs) = (entities(base), entities(ours), entities(theirs));
    let mut proposals = Vec::new();
    for (o, (ot, ol)) in &ours {
        if base.contains_key(o) || theirs.contains_key(o) {
            continue;
        }
        let mut candidates = Vec::new();
        for (t, (tt, tl)) in &theirs {
            if base.contains_key(t) || ours.contains_key(t) || ot.is_disjoint(tt) {
                continue;
            }
            let score = ol
                .iter()
                .flat_map(|a| tl.iter().map(move |b| strsim::jaro_winkler(a, b)))
                .fold(0.0_f64, f64::max);
            if score >= 0.90 {
                candidates.push(AliasProposal {
                    ours: o.clone(),
                    theirs: t.clone(),
                    similarity: score,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.similarity
                .total_cmp(&a.similarity)
                .then(a.theirs.cmp(&b.theirs))
        });
        proposals.extend(candidates.into_iter().take(5));
    }
    proposals
}
