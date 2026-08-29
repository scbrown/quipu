//! Triples, graphs, and the two serialisations the baselines are measured on.
//!
//! The unit of change is the canonical triple. A graph is therefore a SET, and
//! the canonical serialisation is that set sorted — which is what makes the
//! canonical line-merge baseline (arm `git-canonical`) meaningful and the
//! non-canonical one (arm `git-turtle`) an honest picture of what git does to
//! RDF as it is normally written.

use std::collections::{BTreeMap, BTreeSet};

use crate::shapes::NS;

/// An RDF term as written in N-Triples: `<iri>` or `"literal"`.
pub type Term = String;

/// A canonical triple. No blank nodes: the benchmark's graphs are ground, so
/// canonicalisation is sorting and RDFC-1.0 is not exercised here. That
/// boundary is stated in the build report rather than papered over.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Triple {
    /// Subject IRI.
    pub s: Term,
    /// Predicate IRI.
    pub p: Term,
    /// Object term.
    pub o: Term,
}

impl Triple {
    /// Build a triple from an IRI subject, IRI predicate, and object term.
    pub fn new(s: impl Into<String>, p: impl Into<String>, o: impl Into<String>) -> Self {
        Self { s: s.into(), p: p.into(), o: o.into() }
    }

    /// The `(subject, predicate)` slot this triple occupies — the unit a
    /// cardinality constraint is declared over, and therefore the unit a
    /// conflict is reported over.
    #[must_use]
    pub fn slot(&self) -> Slot {
        (self.s.clone(), self.p.clone())
    }

    /// One N-Triples line, without the trailing newline.
    #[must_use]
    pub fn to_nt(&self) -> String {
        format!("<{}> <{}> {} .", self.s, self.p, self.o)
    }

    /// Parse one N-Triples line. Returns `None` for anything that is not a
    /// well-formed triple — which is how the line-merge arms' broken output is
    /// counted rather than silently dropped.
    #[must_use]
    pub fn from_nt(line: &str) -> Option<Self> {
        let line = line.trim();
        let body = line.strip_suffix('.')?.trim_end();
        let rest = body.strip_prefix('<')?;
        let (s, rest) = rest.split_once('>')?;
        let rest = rest.trim_start().strip_prefix('<')?;
        let (p, rest) = rest.split_once('>')?;
        let o = rest.trim();
        if o.is_empty() {
            return None;
        }
        if o.starts_with('<') && !(o.ends_with('>') && o.len() > 1) {
            return None;
        }
        if o.starts_with('"') && !(o.ends_with('"') && o.len() > 1) {
            return None;
        }
        if !o.starts_with('<') && !o.starts_with('"') {
            return None;
        }
        let o = o.strip_prefix('<').and_then(|v| v.strip_suffix('>')).map_or_else(
            || o.to_string(),
            |iri| format!("<{iri}>"),
        );
        Some(Self::new(s, p, o))
    }
}

/// The `(subject, predicate)` pair a constraint and a conflict are scoped to.
pub type Slot = (String, String);

/// A set of triples. `BTreeSet` because the canonical form is the sorted set —
/// no ordering or serialisation artefact can reach a metric.
pub type Graph = BTreeSet<Triple>;

/// An IRI term for the object position.
#[must_use]
pub fn iri(v: &str) -> Term {
    format!("<{v}>")
}

/// A plain-literal term for the object position.
#[must_use]
pub fn lit(v: &str) -> Term {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Canonical serialisation: the sorted triple set, one N-Triples line each.
///
/// This is the form a share bundle's `export.nt` takes, and the form the
/// `git-canonical` baseline merges.
#[must_use]
pub fn to_canonical_nt(g: &Graph) -> String {
    let mut out = String::new();
    for t in g {
        out.push_str(&t.to_nt());
        out.push('\n');
    }
    out
}

/// Non-canonical Turtle: subject-grouped, predicate-object lists, a prefix
/// block, and blank lines between subjects — RDF as a person or a serialiser
/// actually writes it.
///
/// `perm = 0` writes one stable, sorted order — the form all three inputs of
/// the `git-turtle-stable` arm share, so the only differences git sees are the
/// real edits. Any other `perm` permutes subject order and the predicate order
/// within each subject: not noise injected to make a baseline look bad, but
/// what two independently written or re-serialised copies of the same graph
/// look like, and precisely what canonicalisation removes.
///
/// The permutation must not depend on how MANY triples a subject has, or the
/// "stable" arm silently stops being stable whenever one side adds a value —
/// which it did until this was measured against git's own output.
#[must_use]
pub fn to_turtle(g: &Graph, perm: u64) -> String {
    let mut by_subject: BTreeMap<&str, Vec<&Triple>> = BTreeMap::new();
    for t in g {
        by_subject.entry(&t.s).or_default().push(t);
    }
    let mut subjects: Vec<&str> = by_subject.keys().copied().collect();
    rotate(&mut subjects, perm);
    let within = |s: &str| if perm == 0 { 0 } else { perm.wrapping_add(s.len() as u64) };

    let mut out = String::from("@prefix bench: <http://example.org/bench#> .\n");
    out.push_str("@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\n");
    for s in subjects {
        let mut triples = by_subject[s].clone();
        rotate(&mut triples, within(s));
        out.push_str(&format!("{} \n", shorten(s, true)));
        for (i, t) in triples.iter().enumerate() {
            let sep = if i + 1 == triples.len() { " ." } else { " ;" };
            out.push_str(&format!(
                "    {} {}{}\n",
                shorten(&t.p, true),
                shorten(&t.o, false),
                sep
            ));
        }
        out.push('\n');
    }
    out
}

/// Rotate a slice by a seed-derived offset — a stable, seed-controlled
/// permutation with no dependence on hash order. Seed 0 is the identity.
fn rotate<T>(v: &mut [T], seed: u64) {
    if seed == 0 || v.len() < 2 {
        return;
    }
    let k = (seed % v.len() as u64) as usize;
    v.rotate_left(k);
}

/// Abbreviate an IRI to `bench:local` where possible. `angle` says whether a
/// term that stays long needs its angle brackets (subjects and predicates
/// arrive bare; objects arrive already bracketed or quoted).
fn shorten(term: &str, angle: bool) -> String {
    let bare = term.strip_prefix('<').and_then(|t| t.strip_suffix('>')).unwrap_or(term);
    if let Some(local) = bare.strip_prefix(NS) {
        return format!("bench:{local}");
    }
    if let Some(local) = bare.strip_prefix("http://www.w3.org/1999/02/22-rdf-syntax-ns#") {
        return format!("rdf:{local}");
    }
    if term.starts_with('"') {
        return term.to_string();
    }
    if angle || !term.starts_with('<') {
        format!("<{bare}>")
    } else {
        term.to_string()
    }
}

/// `rdf:type`.
#[must_use]
pub fn rdf_type() -> String {
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".to_string()
}

/// Group a graph's triples by `(subject, predicate)` slot.
#[must_use]
pub fn by_slot(g: &Graph) -> BTreeMap<Slot, BTreeSet<Term>> {
    let mut m: BTreeMap<Slot, BTreeSet<Term>> = BTreeMap::new();
    for t in g {
        m.entry(t.slot()).or_default().insert(t.o.clone());
    }
    m
}
