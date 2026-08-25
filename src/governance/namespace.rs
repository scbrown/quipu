//! Namespace drift — which base-namespace predicates episode ingest minted
//! that no loaded shape mentions (`docs/design/statement-identity.md` §8).
//!
//! ## Why a report and not a gate
//!
//! Every key in an episode node's `properties` map becomes a predicate in the
//! base namespace via `sanitize_iri_local`, and no shape governs which keys are
//! admissible. Agents writing free-form properties mint predicates
//! indefinitely, and until this module existed nothing reported the drift.
//!
//! The design names the cheapest useful version explicitly: **a report, not a
//! block**. That is not timidity, it is the only version that can ship without
//! a migration. A refusal here would reject writes that every deployment is
//! already making — the ontology in the store today was grown by exactly this
//! path — so the gate would be switched off within a day and the drift would
//! go back to being invisible. A report an operator reads weekly beats a gate
//! nobody leaves on. Consequently [`check`] returns findings and never an
//! error verdict, and the CLI exits `0` whatever it finds.
//!
//! ## What "minted by episode ingest" means here, precisely
//!
//! A predicate is counted when all four hold of some current fact:
//!
//! 1. its subject carries `prov:wasGeneratedBy` pointing at a `{base}episode_…`
//!    activity — i.e. the subject is a node an episode wrote;
//! 2. the predicate IRI is in the configured base namespace;
//! 3. the object is a **literal**, which is what separates the `properties`
//!    map from the edge path (edge relations resolve to node references and go
//!    through `resolve_edge_predicate`, which is already a fence); and
//! 4. the predicate is not one episode ingest emits structurally.
//!
//! Anything the store cannot support is omitted rather than invented. In
//! particular "first/last seen" is the earliest and latest `valid_from` among
//! the facts using the predicate — the store keeps no separate mint timestamp,
//! and a predicate re-asserted with an older valid time genuinely moves its
//! first-seen backwards. It answers "since when has this predicate been in
//! use", not "when was this IRI first interned".
//!
//! ## What "no shape mentions it" means, precisely
//!
//! A predicate is governed if its IRI appears **anywhere** in any loaded
//! shape's graph — as an `sh:path`, a target, or any other position. That is
//! the widest possible reading of "mentions", chosen deliberately: this is a
//! report an operator is meant to act on, and a false alarm costs more here
//! than a missed one. A predicate that a shape names only inside a complex
//! SHACL path expression is still counted as mentioned, because its IRI is
//! still a node in the shapes graph.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

/// A base-namespace predicate minted by episode ingest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedPredicate {
    /// The full predicate IRI.
    pub iri: String,
    /// The local name — the sanitized `properties` key that minted it.
    pub local: String,
    /// Current facts using it on episode-written subjects.
    pub facts: usize,
    /// Distinct episode-written subjects carrying it.
    pub subjects: usize,
    /// Earliest `valid_from` among those facts. See the module doc: this is a
    /// valid-time floor, not a mint timestamp.
    pub first_seen: String,
    /// Latest `valid_from` among those facts.
    pub last_seen: String,
    /// Whether some loaded shape mentions the IRI.
    pub governed: bool,
}

/// What the namespace scan found.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Minted predicates no loaded shape mentions, by IRI.
    pub ungoverned: Vec<MintedPredicate>,
    /// Minted predicates some loaded shape does mention. Counted, not listed:
    /// the number is what says whether the ungoverned list is the tail of a
    /// governed ontology or the whole of an ungoverned one.
    pub governed: usize,
    /// Shapes loaded in the store at scan time.
    pub shapes_loaded: usize,
    /// Distinct IRIs the loaded shapes mention.
    pub shape_terms: usize,
    /// Episode-written subjects scanned.
    pub subjects_scanned: usize,
    /// The graph scanned, as an IRI (or the ROOT sentinel).
    pub graph: String,
}

impl Report {
    /// A one-line summary.
    ///
    /// Never says "clean": an empty result over a store with no shapes loaded
    /// and no episodes written is not a clean bill of health, it is an
    /// unmeasured one, and [`summary`](Self::summary) says which it is.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.subjects_scanned == 0 {
            return format!(
                "namespace: no episode-written subject in {graph}, so nothing has \
                 been minted to report on ({shapes} shape(s) loaded)",
                graph = self.graph,
                shapes = self.shapes_loaded,
            );
        }
        format!(
            "namespace: {ungoverned} ungoverned predicate(s), {governed} governed, \
             minted by episode ingest over {subjects} episode-written subject(s) \
             in {graph} against {shapes} loaded shape(s)",
            ungoverned = self.ungoverned.len(),
            governed = self.governed,
            subjects = self.subjects_scanned,
            graph = self.graph,
            shapes = self.shapes_loaded,
        )
    }

    /// Total minted predicates, governed and not.
    #[must_use]
    pub fn minted(&self) -> usize {
        self.ungoverned.len() + self.governed
    }
}

/// Predicates episode ingest emits itself, which no `properties` key minted.
///
/// These are structural: the episode activity's own bookkeeping. They are
/// excluded by name rather than by shape membership because reporting the
/// writer's own vocabulary as agent drift would put a permanent two-line floor
/// under every report, and a report with a floor is one operators learn to
/// discount.
const STRUCTURAL: [&str; 2] = ["groupId", "contentHash"];

/// Scan `graph` for base-namespace predicates minted by episode ingest.
///
/// Pass `graph = None` for the ROOT / default committed graph.
///
/// # Errors
/// Propagates store read errors and shape parse errors.
pub fn check(store: &Store, base_ns: &str, graph: Option<&str>) -> Result<Report> {
    let (g, graph_name) = resolve_graph(store, graph)?;
    let mentioned = shape_terms(store)?;
    let shapes_loaded = store.list_shapes()?.len();

    let facts = store.current_facts_in_graph(g)?;
    let mut iris: HashMap<i64, String> = HashMap::new();
    let iri_of = |store: &Store, id: i64, iris: &mut HashMap<i64, String>| -> Option<String> {
        if let Some(known) = iris.get(&id) {
            return Some(known.clone());
        }
        let resolved = store.resolve(id).ok()?;
        iris.insert(id, resolved.clone());
        Some(resolved)
    };

    // Pass 1 — which subjects an episode wrote.
    let generated_by = format!("{}wasGeneratedBy", crate::namespace::PROV);
    let episode_prefix = format!("{base_ns}episode_");
    let mut written: BTreeSet<i64> = BTreeSet::new();
    for fact in &facts {
        let Some(attr) = iri_of(store, fact.attribute, &mut iris) else {
            continue;
        };
        if attr != generated_by {
            continue;
        }
        // The object must be the episode activity itself. A `prov:wasGeneratedBy`
        // pointing anywhere else is somebody else's provenance, and counting it
        // would attribute their predicates to episode ingest.
        if let Value::Ref(target) = fact.value
            && iri_of(store, target, &mut iris).is_some_and(|iri| iri.starts_with(&episode_prefix))
        {
            written.insert(fact.entity);
        }
    }

    // Pass 2 — the base-namespace literal predicates on those subjects.
    let mut minted: BTreeMap<String, MintedPredicate> = BTreeMap::new();
    let mut subjects_of: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for fact in &facts {
        if !written.contains(&fact.entity) || matches!(fact.value, Value::Ref(_)) {
            continue;
        }
        let Some(attr) = iri_of(store, fact.attribute, &mut iris) else {
            continue;
        };
        let Some(local) = attr.strip_prefix(base_ns) else {
            continue;
        };
        if STRUCTURAL.contains(&local) {
            continue;
        }
        let entry = minted
            .entry(attr.clone())
            .or_insert_with(|| MintedPredicate {
                iri: attr.clone(),
                local: local.to_string(),
                facts: 0,
                subjects: 0,
                first_seen: fact.valid_from.clone(),
                last_seen: fact.valid_from.clone(),
                governed: mentioned.contains(&attr),
            });
        entry.facts += 1;
        if fact.valid_from < entry.first_seen {
            entry.first_seen.clone_from(&fact.valid_from);
        }
        if fact.valid_from > entry.last_seen {
            entry.last_seen.clone_from(&fact.valid_from);
        }
        subjects_of.entry(attr).or_default().insert(fact.entity);
    }

    let mut report = Report {
        shapes_loaded,
        shape_terms: mentioned.len(),
        subjects_scanned: written.len(),
        graph: graph_name,
        ..Report::default()
    };
    for (iri, mut pred) in minted {
        pred.subjects = subjects_of.get(&iri).map_or(0, BTreeSet::len);
        if pred.governed {
            report.governed += 1;
        } else {
            report.ungoverned.push(pred);
        }
    }
    // Most-used first: drift is worth reading in the order it is worth acting
    // on, and an alphabetical list buries the predicate on ten thousand nodes
    // under the one on a single node. IRI breaks ties so the order is stable.
    report
        .ungoverned
        .sort_by(|a, b| b.facts.cmp(&a.facts).then_with(|| a.iri.cmp(&b.iri)));
    Ok(report)
}

/// Every IRI any loaded shape mentions, in any position.
fn shape_terms(store: &Store) -> Result<BTreeSet<String>> {
    use crate::error::Error;
    use oxrdf::Term as OxTerm;
    use oxttl::TurtleParser;

    let mut terms: BTreeSet<String> = BTreeSet::new();
    for (name, turtle, _) in store.list_shapes()? {
        let parser = TurtleParser::new()
            .with_base_iri("http://example.org/")
            .map_err(|e| Error::InvalidValue(format!("shapes base IRI: {e}")))?;
        for triple in parser.for_reader(turtle.as_bytes()) {
            let triple = triple
                .map_err(|e| Error::InvalidValue(format!("shape '{name}' parse error: {e}")))?;
            if let oxrdf::NamedOrBlankNode::NamedNode(s) = &triple.subject {
                terms.insert(s.as_str().to_string());
            }
            terms.insert(triple.predicate.as_str().to_string());
            if let OxTerm::NamedNode(o) = &triple.object {
                terms.insert(o.as_str().to_string());
            }
        }
    }
    Ok(terms)
}

/// The graph id and display name to scan.
fn resolve_graph(store: &Store, graph: Option<&str>) -> Result<(i64, String)> {
    use crate::error::Error;

    let Some(iri) = graph else {
        return Ok((
            crate::schema::ROOT_GRAPH,
            crate::schema::ROOT_GRAPH_IRI.to_string(),
        ));
    };
    if iri == crate::schema::ROOT_GRAPH_IRI {
        return Ok((crate::schema::ROOT_GRAPH, iri.to_string()));
    }
    // An unknown graph is an error, not an empty result: "no drift in the graph
    // you named" and "there is no such graph" are different answers, and only
    // one of them should let an operator stop looking.
    let g = store
        .lookup(iri)?
        .ok_or_else(|| Error::InvalidValue(format!("no such graph: {iri}")))?;
    Ok((g, iri.to_string()))
}

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod tests;
