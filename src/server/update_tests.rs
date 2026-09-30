//! Differential tests for the sliced `/update` path (aegis-jm1lcl).
//!
//! Every update runs on two identically seeded stores, one forced through the
//! full-copy path and one through the planner, and both the resulting current
//! facts (per graph) and the transacted datums must be identical.

use std::sync::Arc;

use quipu::store::Datum;
use quipu::{Op, Store, Value};

use super::super::{SharedStore, StoreHandle};
use super::update_slice::{Plan, plan};
use super::{UpdatePath, apply_update_as};

const PREFIXES: &str = "BASE <http://localhost/update>\n\
    PREFIX p: <http://ex.org/p/> PREFIX e: <http://ex.org/e/> \
    PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";

const SEED: &str = r#"INSERT DATA {
    e:s1 p:claimedBy "agentA" ; p:status "open" ; p:priority 2 ;
         p:due "2026-01-01"^^xsd:date ; p:title "hello"@en .
    e:s2 p:claimedBy "agentC" ; p:status "closed" ; p:priority 5 .
    e:s3 p:status "open" ; p:dependsOn e:s1 .
    e:s1 p:dependsOn e:s2 .
    e:s9 p:unrelated "noise" .
    GRAPH <http://ex.org/g/1> { e:s1 p:claimedBy "agentG" . e:s4 p:status "open" }
    GRAPH <http://ex.org/g/2> { e:s5 p:status "done" . e:s5 p:priority 1 }
}"#;

fn seeded() -> SharedStore {
    let store = Store::open_in_memory().unwrap();
    // g/3 is registered and stays empty: graph existence must not leak into results.
    for g in [
        "http://ex.org/g/1",
        "http://ex.org/g/2",
        "http://ex.org/g/3",
    ] {
        store.graph_create(g).unwrap();
    }
    let shared: SharedStore = Arc::new(StoreHandle::writer_only(store));
    apply_update_as(&shared, &format!("{PREFIXES}{SEED}"), true).unwrap();
    let mut store = shared.lock();
    let bnode = store.intern("_:b1").unwrap();
    let status = store.intern("http://ex.org/p/status").unwrap();
    store
        .transact(
            &[Datum {
                entity: bnode,
                attribute: status,
                value: Value::Str("open".into()),
                valid_from: "2026-01-01T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-01-01T00:00:00Z",
            Some("test"),
            Some("test"),
        )
        .unwrap();
    drop(store);
    shared
}

fn term(store: &Store, value: &Value) -> String {
    quipu::rdf::value_to_term(store, value).unwrap().to_string()
}

fn graph_iri(store: &Store, g: i64) -> String {
    if g == 0 {
        "default".into()
    } else {
        store.resolve(g).unwrap()
    }
}

/// Every current fact in every dataset graph, as sorted strings.
fn state(shared: &SharedStore) -> Vec<String> {
    let store = shared.lock();
    let mut graphs = vec![0];
    graphs.extend(store.all_named_graph_ids().unwrap());
    let mut out = Vec::new();
    for g in graphs {
        for f in store.current_facts_in_graph(g).unwrap() {
            out.push(format!(
                "{} {} {} {}",
                graph_iri(&store, g),
                store.resolve(f.entity).unwrap(),
                store.resolve(f.attribute).unwrap(),
                term(&store, &f.value)
            ));
        }
    }
    out.sort();
    out
}

fn datums(shared: &SharedStore, changes: &[(i64, Vec<Datum>)]) -> Vec<String> {
    let store = shared.lock();
    let mut out: Vec<String> = changes
        .iter()
        .flat_map(|(g, ds)| ds.iter().map(move |d| (*g, d)))
        .map(|(g, d)| {
            format!(
                "{} {} {} {} {:?}",
                graph_iri(&store, g),
                store.resolve(d.entity).unwrap(),
                store.resolve(d.attribute).unwrap(),
                term(&store, &d.value),
                d.op
            )
        })
        .collect();
    out.sort();
    out
}

/// Run `updates` in order through both paths; return the planner's paths.
fn differential(updates: &[&str]) -> Vec<UpdatePath> {
    let (full, sliced) = (seeded(), seeded());
    assert_eq!(state(&full), state(&sliced), "seeds differ");
    let mut paths = Vec::new();
    for update in updates {
        let text = format!("{PREFIXES}{update}");
        let a = apply_update_as(&full, &text, true).unwrap();
        let b = apply_update_as(&sliced, &text, false).unwrap();
        assert_eq!(a.path, UpdatePath::Full);
        assert_eq!(
            datums(&full, &a.changes),
            datums(&sliced, &b.changes),
            "datums differ for {update}"
        );
        assert_eq!(state(&full), state(&sliced), "state differs after {update}");
        paths.push(b.path);
    }
    paths
}

const SLICED_CORPUS: &[&str] = &[
    // CAS claim: succeeds, then the same CAS fails its FILTER and changes nothing.
    r#"DELETE { e:s1 p:claimedBy ?o } INSERT { e:s1 p:claimedBy "agentB" }
       WHERE { e:s1 p:claimedBy ?o . FILTER(?o = "agentA") }"#,
    r#"DELETE { e:s1 p:claimedBy ?o } INSERT { e:s1 p:claimedBy "agentB" }
       WHERE { e:s1 p:claimedBy ?o . FILTER(?o = "agentA") }"#,
    // Release.
    "DELETE WHERE { e:s1 p:claimedBy ?o }",
    // OPTIONAL.
    r#"INSERT { ?s p:unclaimed true } WHERE {
         ?s p:status ?st OPTIONAL { ?s p:claimedBy ?c } FILTER(!BOUND(?c)) }"#,
    // FILTER NOT EXISTS.
    r#"INSERT { ?s p:flag "noprio" } WHERE {
         ?s p:status ?x FILTER NOT EXISTS { ?s p:priority ?p } }"#,
    // MINUS.
    "DELETE { ?s p:flag ?f } WHERE { ?s p:flag ?f MINUS { ?s p:dependsOn ?d } }",
    // GRAPH <g>.
    r#"DELETE { GRAPH <http://ex.org/g/1> { ?s p:claimedBy ?o } }
       INSERT { GRAPH <http://ex.org/g/1> { ?s p:claimedBy "agentH" } }
       WHERE { GRAPH <http://ex.org/g/1> { ?s p:claimedBy ?o } }"#,
    // GRAPH ?g around a triple pattern.
    r#"DELETE { GRAPH ?g { ?s p:status "open" } } INSERT { GRAPH ?g { ?s p:status "triaged" } }
       WHERE { GRAPH ?g { ?s p:status "open" } }"#,
    // WITH <g>.
    r#"WITH <http://ex.org/g/2> DELETE { ?s p:status "done" } INSERT { ?s p:status "archived" }
       WHERE { ?s p:status "done" }"#,
    // USING NAMED restricts which graphs GRAPH ?g ranges over.
    r#"INSERT { ?s p:inNamed ?v } USING NAMED <http://ex.org/g/1> USING NAMED <http://ex.org/g/3>
       WHERE { GRAPH ?g { ?s p:status ?v } }"#,
    // USING <g>: read the named graph, write the default graph.
    "INSERT { ?s p:copied ?v } USING <http://ex.org/g/1> WHERE { ?s p:status ?v }",
    // A ';' sequence whose second operation reads the first one's write.
    r#"INSERT DATA { e:s6 p:claimedBy "x" } ;
       DELETE { e:s6 p:claimedBy ?o } INSERT { e:s6 p:claimedBy "y" } WHERE { e:s6 p:claimedBy ?o }"#,
    // INSERT DATA of a present fact; DELETE DATA of an absent one.
    r#"INSERT DATA { e:s2 p:status "closed" }"#,
    r#"DELETE DATA { e:s2 p:status "nonexistent" }"#,
    // An aggregate in a subquery.
    r#"INSERT { e:summary p:closedCount ?n } WHERE {
         SELECT (COUNT(?s) AS ?n) WHERE { ?s p:status "closed" } }"#,
    // A blank-node subject.
    r#"DELETE { ?s p:status "open" } INSERT { ?s p:status "seen" }
       WHERE { ?s p:status "open" FILTER(isBlank(?s)) }"#,
    // Typed and language-tagged literals.
    r#"DELETE { e:s1 p:due ?d } INSERT { e:s1 p:due "2026-02-01"^^xsd:date }
       WHERE { e:s1 p:due ?d FILTER(?d = "2026-01-01"^^xsd:date) }"#,
    r#"DELETE { e:s1 p:title "hello"@en } INSERT { e:s1 p:title "bonjour"@fr }
       WHERE { e:s1 p:title "hello"@en }"#,
    // A one-or-more path and EXISTS inside BIND.
    "INSERT { ?a p:reaches ?b } WHERE { ?a p:dependsOn+ ?b }",
    "INSERT { ?s p:hasPrio ?h } WHERE { ?s p:status ?x BIND(EXISTS { ?s p:priority ?p } AS ?h) }",
];

#[test]
fn sliced_path_matches_full_path() {
    let paths = differential(SLICED_CORPUS);
    assert!(
        paths.iter().all(|p| *p == UpdatePath::Sliced),
        "every corpus update should slice: {paths:?}"
    );
}

const FALLBACK_CORPUS: &[(&str, &str)] = &[
    (
        "variable-predicate",
        r#"INSERT { e:x p:y "z" } WHERE { e:s1 ?p ?o }"#,
    ),
    (
        "variable-predicate",
        r#"INSERT { ?s ?p "z" } WHERE { ?s p:status ?x BIND(p:made AS ?p) }"#,
    ),
    (
        "zero-length-path",
        "INSERT { ?a p:reach ?b } WHERE { ?a p:dependsOn* ?b }",
    ),
    (
        "zero-length-path",
        "INSERT { ?a p:reach ?b } WHERE { ?a p:dependsOn? ?b }",
    ),
    (
        "negated-property-set",
        "INSERT { ?a p:other ?b } WHERE { ?a !p:status ?b }",
    ),
    (
        "graph-existence",
        r#"INSERT { e:g p:seen ?g } WHERE { GRAPH ?g { OPTIONAL { ?s p:none ?o } } }"#,
    ),
    (
        "graph-existence",
        "INSERT { e:g3 p:exists true } WHERE { GRAPH <http://ex.org/g/3> { } }",
    ),
    ("clear", "CLEAR GRAPH <http://ex.org/g/2>"),
    ("drop", "DROP GRAPH <http://ex.org/g/1>"),
];

#[test]
fn fallback_triggers_take_the_full_path() {
    for (reason, update) in FALLBACK_CORPUS {
        assert_eq!(
            plan(&format!("{PREFIXES}{update}")),
            Plan::Full(reason),
            "{update}"
        );
        // And the full path still produces the forced result.
        assert_eq!(differential(&[update]), vec![UpdatePath::Full], "{update}");
    }
}

#[test]
fn cas_slices_to_one_subject() {
    let Plan::Sliced(touched) = plan(&format!("{PREFIXES}{}", SLICED_CORPUS[0])) else {
        panic!("CAS must slice");
    };
    assert_eq!(touched.len(), 1);
    assert_eq!(
        touched["http://ex.org/p/claimedBy"],
        super::update_slice::Subjects::Only(["http://ex.org/e/s1".to_string()].into())
    );
}

#[test]
fn metrics_count_both_paths() {
    differential(&[SLICED_CORPUS[0]]);
    let mut body = String::new();
    super::render_update_paths(&mut body);
    assert!(body.contains("# TYPE quipu_sparql_update_evaluations_total counter"));
    for path in ["sliced", "full"] {
        let line = body
            .lines()
            .find(|l| {
                l.starts_with(&format!(
                    "quipu_sparql_update_evaluations_total{{path=\"{path}\"}}"
                ))
            })
            .unwrap();
        let count: u64 = line.rsplit(' ').next().unwrap().parse().unwrap();
        assert!(count >= 1, "{line}");
    }
}

// Review probes: paths with constant endpoints, cross-graph re-inserts,
// GRAPH ?g joined with VALUES or outer patterns, operators inside GRAPH,
// EXISTS reaching into a named graph, never-interned predicates, and a
// delete-then-read sequence.
const PROBE_CORPUS: &[&str] = &[
    // reverse / sequence / alternative paths with constant endpoints
    "INSERT { ?x p:rev true } WHERE { e:s1 ^p:dependsOn ?x }",
    "INSERT { ?x p:two true } WHERE { e:s3 p:dependsOn/p:dependsOn ?x }",
    "INSERT { e:s1 p:alt ?x } WHERE { e:s1 (p:status|p:claimedBy) ?x }",
    "INSERT { ?x p:revplus true } WHERE { e:s2 ^p:dependsOn+ ?x }",
    // WHERE reads one subject, template writes another subject of the same predicate
    r#"INSERT { e:s2 p:claimedBy "q" } WHERE { e:s1 p:claimedBy ?o }"#,
    // re-insert of a fact present in ANOTHER graph only
    r#"INSERT DATA { e:s4 p:status "open" }"#,
    r#"INSERT DATA { GRAPH <http://ex.org/g/2> { e:s1 p:claimedBy "agentA" } }"#,
    // GRAPH ?g joined with VALUES and with an outer pattern
    r#"INSERT { e:v p:g ?g } WHERE { VALUES ?g { <http://ex.org/g/2> <http://ex.org/g/3> } GRAPH ?g { ?s p:priority ?p } }"#,
    r#"INSERT { ?s p:alsoNamed ?g } WHERE { ?s p:status ?v . GRAPH ?g { ?s p:claimedBy ?c } }"#,
    // EXISTS / OPTIONAL / UNION / MINUS inside GRAPH, with a required triple
    r#"INSERT { GRAPH ?g { ?s p:noprio true } } WHERE { GRAPH ?g { ?s p:status ?v FILTER NOT EXISTS { ?s p:priority ?p } } }"#,
    r#"INSERT { e:o p:opt ?g } WHERE { GRAPH ?g { ?s p:status ?v OPTIONAL { ?s p:priority ?p } } }"#,
    r#"INSERT { e:u p:uni ?g } WHERE { GRAPH ?g { { ?s p:status ?v } UNION { ?s p:priority ?v } } }"#,
    r#"INSERT { e:m p:min ?s } WHERE { GRAPH ?g { ?s p:status ?v MINUS { ?s p:priority ?p } } }"#,
    // EXISTS reaching into a GRAPH from the default graph
    r#"INSERT { ?s p:namedToo true } WHERE { ?s p:status ?v FILTER EXISTS { GRAPH ?h { ?s p:claimedBy ?c } } }"#,
    // aggregate over an empty / never-interned predicate
    r#"INSERT { e:z p:count ?n } WHERE { SELECT (COUNT(*) AS ?n) WHERE { ?s p:neverInterned ?o } }"#,
    // DELETE WHERE across all named graphs
    r#"DELETE WHERE { GRAPH ?g { ?s p:status "done" } }"#,
    // one-or-more path inside GRAPH
    r#"INSERT { e:gp p:reach ?x } WHERE { GRAPH ?g { e:s1 p:dependsOn+ ?x } }"#,
    // multi-op: delete then read-after-delete
    r#"DELETE DATA { e:s3 p:status "open" } ; INSERT { e:s3 p:gone true } WHERE { FILTER NOT EXISTS { e:s3 p:status ?v } }"#,
    // numeric object equivalence is not narrowed by the planner
    r#"DELETE { ?s p:priority ?p } INSERT { ?s p:priority 3 } WHERE { ?s p:priority ?p FILTER(?p = 2.0) }"#,
    // blank-node subject used in template with another predicate
    r#"INSERT { ?b p:tagged true } WHERE { ?b p:status "open" FILTER(isBlank(?b)) }"#,
];

#[test]
fn review_probes_match_full_path() {
    for update in PROBE_CORPUS {
        differential(&[update]);
    }
    // And as one cumulative sequence.
    differential(PROBE_CORPUS);
}

// Templates whose predicate/subject the WHERE never reads: only the template
// rule puts the target in the slice (aegis-jm1lcl).
const TEMPLATE_ONLY: &[&str] = &[
    r#"DELETE { e:s1 p:status "open" } WHERE { e:s1 p:claimedBy ?c }"#,
    r#"DELETE { ?s p:priority ?p } WHERE { ?s p:status "closed" . e:s2 p:priority ?p }"#,
    r#"INSERT { e:s2 p:status "closed" } WHERE { e:s1 p:claimedBy ?c }"#,
    r#"DELETE { GRAPH <http://ex.org/g/2> { e:s5 p:priority 1 } } WHERE { e:s1 p:claimedBy ?c }"#,
];

#[test]
fn template_only_targets_are_in_the_slice() {
    // Each of these fails if either template rule in `update_slice` is
    // dropped: the WHERE clause never reads what the template writes.
    let paths = differential(TEMPLATE_ONLY);
    assert!(paths.iter().all(|p| *p == UpdatePath::Sliced), "{paths:?}");
}

mod generated {
    //! Randomised differential over a small alphabet: a generator is cheap
    //! at finding the pattern/template combination a hand corpus misses.
    use proptest::prelude::*;

    const SUBJECTS: &[&str] = &["e:s1", "e:s2", "e:s3", "?s"];
    const PREDICATES: &[&str] = &["p:status", "p:claimedBy", "p:priority", "p:dependsOn"];
    const OBJECTS: &[&str] = &["\"open\"", "\"closed\"", "\"agentA\"", "2", "e:s1", "?o"];

    fn triple() -> impl Strategy<Value = String> {
        (
            prop::sample::select(SUBJECTS),
            prop::sample::select(PREDICATES),
            prop::sample::select(OBJECTS),
        )
            .prop_map(|(s, p, o)| format!("{s} {p} {o}"))
    }

    /// A triple, optionally scoped to a named graph.
    fn quad() -> impl Strategy<Value = String> {
        (triple(), any::<bool>()).prop_map(|(t, named)| {
            if named {
                format!("GRAPH <http://ex.org/g/1> {{ {t} }}")
            } else {
                t
            }
        })
    }

    fn update() -> impl Strategy<Value = String> {
        (
            prop::collection::vec(quad(), 0..3),
            prop::collection::vec(quad(), 0..3),
            prop::collection::vec(quad(), 1..3),
            prop::option::of(quad()),
            0..3u8,
        )
            .prop_map(|(delete, insert, wheres, extra, shape)| {
                let body = wheres.join(" . ");
                let body = match (extra, shape) {
                    (Some(x), 0) => format!("{body} OPTIONAL {{ {x} }}"),
                    (Some(x), 1) => format!("{body} FILTER NOT EXISTS {{ {x} }}"),
                    (Some(x), _) => format!("{{ {body} }} UNION {{ {x} }}"),
                    (None, _) => body,
                };
                format!(
                    "DELETE {{ {} }} INSERT {{ {} }} WHERE {{ {body} }}",
                    delete.join(" . "),
                    insert.join(" . ")
                )
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn random_updates_match_full_path(updates in prop::collection::vec(update(), 1..4)) {
            let refs: Vec<&str> = updates.iter().map(String::as_str).collect();
            let paths = super::differential(&refs);
            prop_assert!(paths.iter().all(|p| *p == super::UpdatePath::Sliced), "{paths:?}");
        }
    }
}
