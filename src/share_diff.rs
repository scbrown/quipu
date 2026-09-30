//! Semantic, entity-grouped diffs of share payloads (aegis-fxpbys.1).
//!
//! A share payload is canonical N-Triples/N-Quads: correct for hashing and
//! useless for review. A one-literal edit shows in a line diff as two lines of
//! full IRIs, and RDFC-1.0 may relabel every blank node between two versions
//! even when nothing about them changed. This module reads two payloads into a
//! [`Snapshot`] each and compares FACTS, not lines:
//!
//! * facts are grouped by subject entity and shown by `rdfs:label` when the
//!   entity has one, otherwise by a compact name;
//! * a slot `(subject, predicate, graph)` that holds exactly one value on both
//!   sides and whose value differs is one `predicate: old -> new` change;
//! * blank nodes are identified by their STRUCTURE, never their label (see
//!   [`Snapshot::new`]), so a pure relabel is zero changes.
//!
//! [`render_textconv`] prints one snapshot in the same stable, labelled,
//! entity-grouped form, so `git diff` with a `textconv` driver shows readable
//! diffs without Quipu having to be the diff engine.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use oxrdf::{BlankNode, GraphName, NamedOrBlankNode, Quad, Term};
use oxrdfio::{RdfFormat, RdfParser};
use serde::Serialize;

use crate::error::{Error, Result};

const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
/// Inline blank-node rendering stops here; identity (the signature) does not.
const MAX_INLINE_DEPTH: usize = 4;
/// Signature recursion bound: a guard against pathological cyclic inputs.
const MAX_SIGNATURE_DEPTH: usize = 64;

/// Well-known vocabularies rendered as `prefix:local`. Deliberately a fixed
/// table: a prefix inferred from the data would differ between the two sides of
/// a diff and make every line of a textconv rendering change at once.
const PREFIXES: &[(&str, &str)] = &[
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("dct", "http://purl.org/dc/terms/"),
    ("dcat", "http://www.w3.org/ns/dcat#"),
    ("sh", "http://www.w3.org/ns/shacl#"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("schema", "https://schema.org/"),
];

/// Parse an N-Quads or N-Triples payload (N-Triples is a subset of N-Quads).
pub fn parse_payload(bytes: &[u8], what: &str) -> Result<Vec<Quad>> {
    RdfParser::from_format(RdfFormat::NQuads)
        .for_reader(bytes)
        .map(|q| q.map_err(|e| Error::InvalidValue(format!("{what}: N-Quads parse: {e}"))))
        .collect()
}

/// Read a pack's payload: a standard-artifact directory (`payload.nq`), a
/// legacy share directory (`export.nt`), or a single N-Triples/N-Quads file.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_payload(path: &std::path::Path) -> Result<Vec<Quad>> {
    let file = if path.is_dir() {
        ["payload.nq", "export.nt"]
            .iter()
            .map(|name| path.join(name))
            .find(|p| p.is_file())
            .ok_or_else(|| {
                Error::InvalidValue(format!(
                    "{}: no payload.nq or export.nt in pack directory",
                    path.display()
                ))
            })?
    } else {
        path.to_path_buf()
    };
    let bytes = std::fs::read(&file)
        .map_err(|e| Error::InvalidValue(format!("{}: {e}", file.display())))?;
    parse_payload(&bytes, &file.display().to_string())
}

/// A fact with every term reduced to a comparison key. IRIs key as `<iri>`,
/// literals as their N-Triples form, blank nodes as `_:` + structural signature.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Fact {
    subject: String,
    predicate: String,
    object: String,
    graph: String,
}

/// How a term key is shown. IRIs are resolved at render time so a label known
/// to either side of a diff can be used.
#[derive(Debug, Clone)]
enum Shown {
    Iri(String),
    Text(String),
}

/// One side of a diff: keyed facts plus what is needed to display them.
pub struct Snapshot {
    facts: BTreeSet<Fact>,
    shown: HashMap<String, Shown>,
    labels: HashMap<String, String>,
}

struct Blanks<'a> {
    out: HashMap<&'a str, Vec<(&'a str, &'a Term)>>,
    memo: HashMap<&'a str, String>,
}

impl<'a> Blanks<'a> {
    /// Structural signature: a hash over the sorted `(predicate, object)` pairs
    /// the node carries, with nested blank nodes replaced by THEIR signatures.
    /// The label never enters it. A cycle back onto the current path contributes
    /// a fixed token, and only cycle-free results are memoized, so the value is
    /// independent of traversal order.
    fn signature(&mut self, id: &'a str, path: &mut Vec<&'a str>) -> (String, bool) {
        if let Some(sig) = self.memo.get(id) {
            return (sig.clone(), false);
        }
        if path.contains(&id) || path.len() >= MAX_SIGNATURE_DEPTH {
            return ("cycle".into(), true);
        }
        path.push(id);
        let mut cyclic = false;
        let edges = self.out.get(id).cloned().unwrap_or_default();
        let mut parts: Vec<String> = edges
            .into_iter()
            .map(|(p, o)| {
                let key = match o {
                    Term::BlankNode(b) => {
                        let (sig, c) = self.signature(b.as_str(), path);
                        cyclic |= c;
                        format!("_:{sig}")
                    }
                    other => other.to_string(),
                };
                format!("{p} {key}")
            })
            .collect();
        path.pop();
        parts.sort_unstable();
        parts.dedup();
        let full = crate::share::sha256(parts.join("\n").as_bytes());
        let sig = full.trim_start_matches("sha256:")[..16].to_string();
        if !cyclic {
            self.memo.insert(id, sig.clone());
        }
        (sig, cyclic)
    }

    fn key(&mut self, b: &'a BlankNode) -> String {
        format!("_:{}", self.signature(b.as_str(), &mut Vec::new()).0)
    }

    fn inline(&self, id: &str, names: &Names<'_>, depth: usize, path: &mut Vec<String>) -> String {
        if path.iter().any(|p| p == id) {
            return "[cycle]".into();
        }
        if depth >= MAX_INLINE_DEPTH {
            return "[...]".into();
        }
        path.push(id.to_string());
        let mut parts: Vec<String> = self
            .out
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .map(|(p, o)| {
                        let value = match o {
                            Term::BlankNode(b) => self.inline(b.as_str(), names, depth + 1, path),
                            Term::NamedNode(n) => names.iri(n.as_str()),
                            Term::Literal(l) => literal(l),
                            // RDF 1.2 triple terms: oxrdf's rdf-12 arrives with the
                            // shacl dependency tree (same gate as `src/rdf.rs`).
                            #[cfg(feature = "shacl")]
                            Term::Triple(t) => t.to_string(),
                        };
                        format!("{} {value}", names.pred(p))
                    })
                    .collect()
            })
            .unwrap_or_default();
        path.pop();
        parts.sort_unstable();
        parts.dedup();
        if parts.is_empty() {
            "[]".into()
        } else {
            format!("[ {} ]", parts.join(" ; "))
        }
    }
}

impl Snapshot {
    /// Build a snapshot. A blank node that appears as an object is folded into
    /// the fact that references it (shown inline, keyed by signature), so its
    /// own triples are not listed again. A blank node never referenced is an
    /// entity of its own, keyed by its signature.
    ///
    /// LIMIT: two blank nodes with identical structure have the same key, so
    /// two identical anonymous values on one slot count once (RDF set semantics
    /// already holds for identical IRIs and literals).
    pub fn new(quads: &[Quad]) -> Self {
        let mut blanks = Blanks {
            out: HashMap::new(),
            memo: HashMap::new(),
        };
        let mut referenced = HashSet::new();
        let mut labels: HashMap<String, String> = HashMap::new();
        for q in quads {
            if let NamedOrBlankNode::BlankNode(b) = &q.subject {
                blanks
                    .out
                    .entry(b.as_str())
                    .or_default()
                    .push((q.predicate.as_str(), &q.object));
            }
            if let Term::BlankNode(b) = &q.object {
                referenced.insert(b.as_str());
            }
            if let (NamedOrBlankNode::NamedNode(s), Term::Literal(l)) = (&q.subject, &q.object)
                && q.predicate.as_str() == RDFS_LABEL
            {
                let entry = labels.entry(s.as_str().to_string()).or_default();
                if entry.is_empty() || l.value() < entry.as_str() {
                    *entry = l.value().to_string();
                }
            }
        }
        let mut facts = BTreeSet::new();
        let mut shown = HashMap::new();
        let mut blank_ids: Vec<(String, &str)> = Vec::new();
        for q in quads {
            let subject = match &q.subject {
                NamedOrBlankNode::NamedNode(n) => {
                    shown.insert(format!("<{}>", n.as_str()), Shown::Iri(n.as_str().into()));
                    format!("<{}>", n.as_str())
                }
                NamedOrBlankNode::BlankNode(b) if referenced.contains(b.as_str()) => continue,
                NamedOrBlankNode::BlankNode(b) => {
                    let key = blanks.key(b);
                    blank_ids.push((key.clone(), b.as_str()));
                    key
                }
            };
            let object = match &q.object {
                Term::NamedNode(n) => {
                    let key = format!("<{}>", n.as_str());
                    shown.insert(key.clone(), Shown::Iri(n.as_str().into()));
                    key
                }
                Term::BlankNode(b) => {
                    let key = blanks.key(b);
                    blank_ids.push((key.clone(), b.as_str()));
                    key
                }
                Term::Literal(l) => {
                    let key = l.to_string();
                    shown.insert(key.clone(), Shown::Text(literal(l)));
                    key
                }
                // RDF 1.2 triple terms (same feature gate as `src/rdf.rs`).
                #[cfg(feature = "shacl")]
                Term::Triple(t) => {
                    let key = t.to_string();
                    shown.insert(key.clone(), Shown::Text(key.clone()));
                    key
                }
            };
            let graph = match &q.graph_name {
                GraphName::DefaultGraph => String::new(),
                GraphName::NamedNode(n) => {
                    shown.insert(format!("<{}>", n.as_str()), Shown::Iri(n.as_str().into()));
                    format!("<{}>", n.as_str())
                }
                // LIMIT: a blank graph name keys by its label; RDFC may relabel it.
                GraphName::BlankNode(b) => format!("_:{}", b.as_str()),
            };
            facts.insert(Fact {
                subject,
                predicate: q.predicate.as_str().to_string(),
                object,
                graph,
            });
        }
        let names = Names {
            labels: vec![&labels],
        };
        for (key, id) in blank_ids {
            shown
                .entry(key)
                .or_insert_with(|| Shown::Text(blanks.inline(id, &names, 0, &mut Vec::new())));
        }
        Snapshot {
            facts,
            shown,
            labels,
        }
    }
}

/// Resolves IRIs to display names from one or more label tables, in order.
struct Names<'a> {
    labels: Vec<&'a HashMap<String, String>>,
}

impl Names<'_> {
    fn label(&self, iri: &str) -> Option<&str> {
        self.labels
            .iter()
            .find_map(|l| l.get(iri))
            .map(String::as_str)
    }
    /// Predicates read as vocabulary words, so they compact to the local name.
    fn pred(&self, iri: &str) -> String {
        self.label(iri)
            .map_or_else(|| local_name(iri), String::from)
    }
    fn iri(&self, iri: &str) -> String {
        self.label(iri).map_or_else(|| compact(iri), String::from)
    }
    /// Entity header: `Label (compact)` so a label never hides which entity.
    fn entity(&self, key: &str, snap: &[&Snapshot]) -> String {
        match key.strip_prefix('<').and_then(|k| k.strip_suffix('>')) {
            Some(iri) => match self.label(iri) {
                Some(label) => format!("{label} ({})", compact(iri)),
                None => compact(iri),
            },
            None => format!("(blank node) {}", self.term(key, snap)),
        }
    }
    fn term(&self, key: &str, snap: &[&Snapshot]) -> String {
        match snap.iter().find_map(|s| s.shown.get(key)) {
            Some(Shown::Iri(iri)) => self.iri(iri),
            Some(Shown::Text(t)) => t.clone(),
            None => key.to_string(),
        }
    }
}

/// `prefix:local` for a well-known vocabulary; otherwise the IRI's last two
/// path segments (`ability/aaa-tracking`), or its fragment when it has one.
/// Derived from the IRI alone, so it is stable across versions.
pub fn compact(iri: &str) -> String {
    for (prefix, ns) in PREFIXES {
        if let Some(local) = iri.strip_prefix(ns)
            && !local.is_empty()
        {
            return format!("{prefix}:{local}");
        }
    }
    if let Some((_, fragment)) = iri.rsplit_once('#')
        && !fragment.is_empty()
    {
        return fragment.to_string();
    }
    let parts: Vec<&str> = iri
        .trim_end_matches('/')
        .rsplit('/')
        .take(2)
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [last, parent] if !parent.is_empty() && !parent.contains(':') => {
            format!("{parent}/{last}")
        }
        [last, ..] if !last.is_empty() => last.to_string(),
        _ => format!("<{iri}>"),
    }
}

/// A predicate's short name: `prefix:local` for a well-known vocabulary,
/// otherwise the fragment or last path segment.
pub fn local_name(iri: &str) -> String {
    let full = compact(iri);
    if full.contains(':') || !full.contains('/') {
        return full;
    }
    full.rsplit('/').next().unwrap_or(&full).to_string()
}

fn literal(l: &oxrdf::Literal) -> String {
    let quoted = serde_json::to_string(l.value()).unwrap_or_else(|_| l.value().to_string());
    if let Some(lang) = l.language() {
        format!("{quoted}@{lang}")
    } else if l.datatype().as_str() == XSD_STRING {
        quoted
    } else {
        format!("{quoted}^^{}", compact(l.datatype().as_str()))
    }
}

/// A fact as shown to a reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactView {
    pub predicate: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

/// A single-valued slot whose value changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub predicate: String,
    pub old: String,
    pub new: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

/// Whether an entity is new, gone, or edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityStatus {
    Added,
    Removed,
    Modified,
}

/// Every difference for one subject entity.
#[derive(Debug, Clone, Serialize)]
pub struct EntityDiff {
    /// The comparison key: `<iri>`, or `_:<signature>` for a blank node.
    pub id: String,
    pub name: String,
    pub status: EntityStatus,
    pub changed: Vec<Change>,
    pub added: Vec<FactView>,
    pub removed: Vec<FactView>,
}

/// A semantic diff between two payloads.
#[derive(Debug, Clone, Serialize)]
pub struct PackDiff {
    pub entities: Vec<EntityDiff>,
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
}

type Slot = (String, String, String);

fn slot_counts(facts: &BTreeSet<Fact>) -> HashMap<Slot, usize> {
    let mut counts = HashMap::new();
    for f in facts {
        *counts
            .entry((f.subject.clone(), f.predicate.clone(), f.graph.clone()))
            .or_insert(0) += 1;
    }
    counts
}

/// Compare two snapshots.
pub fn diff(old: &Snapshot, new: &Snapshot) -> PackDiff {
    let names = Names {
        labels: vec![&new.labels, &old.labels],
    };
    let (old_counts, new_counts) = (slot_counts(&old.facts), slot_counts(&new.facts));
    let old_subjects: HashSet<&str> = old.facts.iter().map(|f| f.subject.as_str()).collect();
    let new_subjects: HashSet<&str> = new.facts.iter().map(|f| f.subject.as_str()).collect();
    // Per subject: slot -> (removed facts, added facts).
    type Sides<'f> = (Vec<&'f Fact>, Vec<&'f Fact>);
    let mut grouped: BTreeMap<&str, BTreeMap<Slot, Sides<'_>>> = BTreeMap::new();
    for f in old.facts.difference(&new.facts) {
        let slot = (f.subject.clone(), f.predicate.clone(), f.graph.clone());
        grouped
            .entry(&f.subject)
            .or_default()
            .entry(slot)
            .or_default()
            .0
            .push(f);
    }
    for f in new.facts.difference(&old.facts) {
        let slot = (f.subject.clone(), f.predicate.clone(), f.graph.clone());
        grouped
            .entry(&f.subject)
            .or_default()
            .entry(slot)
            .or_default()
            .1
            .push(f);
    }
    let graph = |g: &str| (!g.is_empty()).then(|| names.term(g, &[new, old]));
    let view = |f: &Fact, side: &Snapshot| FactView {
        predicate: names.pred(&f.predicate),
        value: names.term(&f.object, &[side]),
        graph: graph(&f.graph),
    };
    let mut out = PackDiff {
        entities: Vec::new(),
        changed: 0,
        added: 0,
        removed: 0,
    };
    for (subject, slots) in grouped {
        let status = match (
            old_subjects.contains(subject),
            new_subjects.contains(subject),
        ) {
            (false, _) => EntityStatus::Added,
            (_, false) => EntityStatus::Removed,
            _ => EntityStatus::Modified,
        };
        let mut e = EntityDiff {
            id: subject.to_string(),
            name: names.entity(subject, &[new, old]),
            status,
            changed: Vec::new(),
            added: Vec::new(),
            removed: Vec::new(),
        };
        for (slot, (removed, added)) in slots {
            let functional = old_counts.get(&slot) == Some(&1) && new_counts.get(&slot) == Some(&1);
            if functional && removed.len() == 1 && added.len() == 1 {
                e.changed.push(Change {
                    predicate: names.pred(&slot.1),
                    old: names.term(&removed[0].object, &[old]),
                    new: names.term(&added[0].object, &[new]),
                    graph: graph(&slot.2),
                });
                continue;
            }
            e.removed.extend(removed.iter().map(|f| view(f, old)));
            e.added.extend(added.iter().map(|f| view(f, new)));
        }
        // Display order, so a reader scans predicates alphabetically.
        e.changed
            .sort_by(|a, b| (&a.predicate, &a.old).cmp(&(&b.predicate, &b.old)));
        e.added
            .sort_by(|a, b| (&a.predicate, &a.value).cmp(&(&b.predicate, &b.value)));
        e.removed
            .sort_by(|a, b| (&a.predicate, &a.value).cmp(&(&b.predicate, &b.value)));
        out.changed += e.changed.len();
        out.added += e.added.len();
        out.removed += e.removed.len();
        out.entities.push(e);
    }
    out
}

// Text, Markdown and textconv renderings live beside the model.
#[path = "share_diff_render.rs"]
mod render;
pub use render::{render_markdown, render_text, render_textconv};

#[cfg(test)]
#[path = "share_diff_tests.rs"]
mod tests;
