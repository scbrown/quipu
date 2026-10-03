//! Display names for share diffs: IRIs, predicates, literals (aegis-fxpbys.1).
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::{Shown, Snapshot};

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

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

/// Resolves IRIs to display names from one or more label tables, in order.
pub(super) struct Names<'a> {
    pub(super) labels: Vec<&'a HashMap<String, String>>,
}

impl Names<'_> {
    fn label(&self, iri: &str) -> Option<&str> {
        self.labels
            .iter()
            .find_map(|l| l.get(iri))
            .map(String::as_str)
    }
    /// Predicates read as vocabulary words, so they compact to the local name.
    pub(super) fn pred(&self, iri: &str) -> String {
        self.label(iri)
            .map_or_else(|| local_name(iri), String::from)
    }
    /// Display names for the predicates rendered together under one entity.
    /// A label or local name can collide (`ex:name` and `schema:name` both
    /// labelled "name", or two unlabelled IRIs ending `/name`); every member of
    /// a colliding group then carries its compact IRI, or the full IRI when
    /// even the compact forms collide, so distinct predicates never read alike.
    pub(super) fn preds<'i>(
        &self,
        iris: impl IntoIterator<Item = &'i str>,
    ) -> HashMap<&'i str, String> {
        let mut by_name: BTreeMap<String, BTreeSet<&'i str>> = BTreeMap::new();
        for iri in iris {
            by_name.entry(self.pred(iri)).or_default().insert(iri);
        }
        let mut out = HashMap::new();
        for (name, group) in by_name {
            let compacts: HashSet<String> = group.iter().map(|i| compact(i)).collect();
            for iri in &group {
                let shown = match (group.len(), compacts.len() == group.len()) {
                    (1, _) => name.clone(),
                    (_, true) => format!("{name} ({})", compact(iri)),
                    (_, false) => format!("{name} (<{iri}>)"),
                };
                out.insert(*iri, shown);
            }
        }
        out
    }
    pub(super) fn iri(&self, iri: &str) -> String {
        self.label(iri).map_or_else(|| compact(iri), String::from)
    }
    /// Entity header: `Label (compact)` so a label never hides which entity.
    pub(super) fn entity(&self, key: &str, snap: &[&Snapshot]) -> String {
        match key.strip_prefix('<').and_then(|k| k.strip_suffix('>')) {
            Some(iri) => match self.label(iri) {
                Some(label) => format!("{label} ({})", compact(iri)),
                None => compact(iri),
            },
            None => format!("(blank node) {}", self.term(key, snap)),
        }
    }
    pub(super) fn term(&self, key: &str, snap: &[&Snapshot]) -> String {
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

pub(super) fn literal(l: &oxrdf::Literal) -> String {
    let quoted = serde_json::to_string(l.value()).unwrap_or_else(|_| l.value().to_string());
    if let Some(lang) = l.language() {
        format!("{quoted}@{lang}")
    } else if l.datatype().as_str() == XSD_STRING {
        quoted
    } else {
        format!("{quoted}^^{}", compact(l.datatype().as_str()))
    }
}

/// Multiplicity marker for a fact asserted by several structurally identical
/// blank nodes: empty for one occurrence, ` xN` otherwise.
pub(super) fn times(n: usize) -> String {
    if n > 1 {
        format!(" x{n}")
    } else {
        String::new()
    }
}
