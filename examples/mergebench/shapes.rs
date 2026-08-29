//! The benchmark's shapes graph — the single constraint contract that both
//! DEFINES conflicts and AUDITS the merge result.
//!
//! Deliberately self-contained and public: a synthetic `bench:` vocabulary
//! with no relation to any deployed ontology, so the whole synthetic arm is
//! publishable without a scrub pass (the scrub deliverable is Arm B's, where
//! the corpus carries real identifiers by construction).
//!
//! The shape declarations are the ONLY place cardinality lives. The merge
//! operator reads them; the generator reads them; the post-merge validator
//! reads them. A benchmark whose conflict oracle and whose merge policy read
//! the same file is not circular here for one reason: the oracle is defined
//! over the semantics of the two edit scripts, and the operator only sees the
//! three graphs. See `benchmark/mergebench/BUILD_REPORT.md` §3.

/// Vocabulary namespace for the benchmark graphs.
pub const NS: &str = "http://example.org/bench#";

/// A predicate the benchmark writes, with its cardinality bound.
pub struct Predicate {
    /// Local name under [`NS`].
    pub name: &'static str,
    /// `sh:maxCount`, when the shapes graph declares one.
    pub max_count: Option<usize>,
}

impl Predicate {
    /// The full predicate IRI.
    #[must_use]
    pub fn iri(&self) -> String {
        format!("{NS}{}", self.name)
    }
}

/// Functional predicates: `sh:maxCount 1`. Divergent values here are the
/// conflict class the paper is about.
pub const FUNCTIONAL: &[Predicate] = &[
    Predicate { name: "label", max_count: Some(1) },
    Predicate { name: "status", max_count: Some(1) },
    Predicate { name: "owner", max_count: Some(1) },
    Predicate { name: "version", max_count: Some(1) },
];

/// Multi-valued predicates. Concurrent additions union; no conflict.
pub const MULTI: &[Predicate] = &[
    Predicate { name: "note", max_count: None },
    Predicate { name: "relatedTo", max_count: None },
];

/// Bounded predicates: multi-valued but capped. Union CAN overflow the bound,
/// which is a conflict only a shape-aware operator can see — and a post-merge
/// violation any operator that unions blindly will admit.
pub const BOUNDED: &[Predicate] = &[Predicate { name: "tag", max_count: Some(4) }];

/// Every predicate the generator writes.
#[must_use]
pub fn all() -> Vec<&'static Predicate> {
    FUNCTIONAL.iter().chain(MULTI).chain(BOUNDED).collect()
}

/// Look up the declared bound for a predicate IRI. `None` means either
/// unbounded or not declared — the operator treats both the same way, which is
/// the honest behaviour for an open vocabulary.
#[must_use]
pub fn max_count(predicate_iri: &str) -> Option<usize> {
    let local = predicate_iri.strip_prefix(NS)?;
    all().into_iter().find(|p| p.name == local).and_then(|p| p.max_count)
}

/// The shapes graph in Turtle, generated from the tables above so the
/// constraint contract cannot drift from the operator's view of it.
#[must_use]
pub fn turtle() -> String {
    let mut s = String::from(
        "@prefix sh:    <http://www.w3.org/ns/shacl#> .\n\
         @prefix rdf:   <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
         @prefix rdfs:  <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix xsd:   <http://www.w3.org/2001/XMLSchema#> .\n\
         @prefix bench: <http://example.org/bench#> .\n\n\
         bench:Entity a rdfs:Class .\n\n\
         bench:EntityShape\n    a sh:NodeShape ;\n    sh:targetClass bench:Entity ;\n",
    );
    for p in all() {
        s.push_str("    sh:property [\n");
        s.push_str(&format!("        sh:path bench:{} ;\n", p.name));
        if let Some(n) = p.max_count {
            s.push_str(&format!("        sh:maxCount {n} ;\n"));
        }
        s.push_str("    ] ;\n");
    }
    s.push_str("    .\n");
    s
}
