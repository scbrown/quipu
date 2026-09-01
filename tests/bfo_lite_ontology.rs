#![cfg(feature = "owl")]

use std::collections::{HashMap, HashSet};

use quipu::{Ontology, QueryResult, Store, Value, ingest_rdf, sparql_query};
use regex::Regex;

const AEGIS: &str = "http://aegis.gastown.local/ontology/";
const CONTINUANT: &str = "http://purl.obolibrary.org/obo/BFO_0000002";
const OCCURRENT: &str = "http://purl.obolibrary.org/obo/BFO_0000003";
const BFO_LITE: &str = include_str!("../ontologies/aegis-bfo-lite.ttl");

const DOMAIN_SHAPES: &[&str] = &[
    include_str!("../shapes/aegis-ontology.shapes.ttl"),
    include_str!("../shapes/code-entities.ttl"),
    include_str!("../shapes/governance.ttl"),
];

fn aegis_terms(pattern: &str, texts: &[&str]) -> HashSet<String> {
    let re = Regex::new(pattern).unwrap();
    texts
        .iter()
        .flat_map(|text| re.captures_iter(text))
        .map(|caps| format!("{AEGIS}{}", &caps[1]))
        .collect()
}

fn reaches(start: &str, target: &str, parents: &HashMap<String, HashSet<String>>) -> bool {
    let mut seen = HashSet::new();
    let mut todo = vec![start.to_owned()];
    while let Some(class) = todo.pop() {
        if class == target {
            return true;
        }
        if seen.insert(class.clone()) {
            todo.extend(parents.get(&class).into_iter().flatten().cloned());
        }
    }
    false
}

#[test]
fn every_governed_aegis_class_reaches_exactly_one_bfo_root() {
    let mut texts = DOMAIN_SHAPES.to_vec();
    texts.push(BFO_LITE);

    let mut governed = aegis_terms(
        r"sh:targetClass\s+(?:aegis|bobbin):([A-Za-z_][\w-]*)",
        DOMAIN_SHAPES,
    );
    governed.extend(aegis_terms(
        r"rdfs:subClassOf\s+aegis:([A-Za-z_][\w-]*)",
        DOMAIN_SHAPES,
    ));

    let edge = Regex::new(
        r"(?m)(?:aegis:([A-Za-z_][\w-]*)|obo:(BFO_\d+))\s+rdfs:subClassOf\s+(?:aegis:([A-Za-z_][\w-]*)|obo:(BFO_\d+))",
    )
    .unwrap();
    let mut parents: HashMap<String, HashSet<String>> = HashMap::new();
    for text in texts {
        for caps in edge.captures_iter(text) {
            let child = caps
                .get(1)
                .map(|m| format!("{AEGIS}{}", m.as_str()))
                .or_else(|| {
                    caps.get(2)
                        .map(|m| format!("http://purl.obolibrary.org/obo/{}", m.as_str()))
                })
                .unwrap();
            let parent = caps
                .get(3)
                .map(|m| format!("{AEGIS}{}", m.as_str()))
                .or_else(|| {
                    caps.get(4)
                        .map(|m| format!("http://purl.obolibrary.org/obo/{}", m.as_str()))
                })
                .unwrap();
            parents.entry(child).or_default().insert(parent);
        }
    }

    let mut missing = Vec::new();
    let mut doubled = Vec::new();
    for class in governed {
        let continuant = reaches(&class, CONTINUANT, &parents);
        let occurrent = reaches(&class, OCCURRENT, &parents);
        match (continuant, occurrent) {
            (false, false) => missing.push(class),
            (true, true) => doubled.push(class),
            _ => {}
        }
    }
    missing.sort();
    doubled.sort();
    assert!(
        missing.is_empty(),
        "governed classes outside the BFO split: {missing:?}"
    );
    assert!(
        doubled.is_empty(),
        "classes under both disjoint BFO roots: {doubled:?}"
    );
}

#[test]
fn canonical_relation_backbone_is_parseable_and_complete() {
    let ontology = Ontology::from_turtle(BFO_LITE).expect("BFO-lite ontology parses");
    let summary = ontology.axiom_summary();

    assert_eq!(summary["disjoint_with"], 1);
    assert_eq!(summary["inverse_of"], 2);
    assert_eq!(summary["domains"], 2);
    assert_eq!(summary["ranges"], 2);
    for relation in [
        "BFO_0000050",
        "BFO_0000051",
        "BFO_0000055",
        "BFO_0000056",
        "BFO_0000057",
        "RO_0000087",
    ] {
        assert!(
            BFO_LITE.contains(relation),
            "missing canonical relation {relation}"
        );
    }
}

#[test]
fn exported_domain_instances_partition_into_continuants_and_occurrents() {
    let mut store = Store::open_in_memory().unwrap();
    for ttl in DOMAIN_SHAPES.iter().chain([&BFO_LITE]) {
        ingest_rdf(
            &mut store,
            ttl.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-08-31T00:00:00Z",
            None,
            None,
        )
        .unwrap();
    }
    ingest_rdf(
        &mut store,
        &br#"@prefix aegis: <http://aegis.gastown.local/ontology/> .
            aegis:service-example a aegis:DatabaseService .
            aegis:incident-example a aegis:Incident ."#[..],
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-31T00:00:01Z",
        None,
        None,
    )
    .unwrap();

    // Exercise the same canonical RDF boundary a share uses: the BFO axioms
    // and domain facts must survive export/import before the reader query runs.
    let export = quipu::export_rdf(&store, oxrdfio::RdfFormat::NTriples).unwrap();
    let mut imported = Store::open_in_memory().unwrap();
    ingest_rdf(
        &mut imported,
        export.as_slice(),
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-08-31T00:00:02Z",
        None,
        None,
    )
    .unwrap();

    let members = |root: &str| {
        let query = format!(
            "SELECT ?s WHERE {{ ?s a/<http://www.w3.org/2000/01/rdf-schema#subClassOf>* <{root}> }}"
        );
        match sparql_query(&imported, &query).unwrap() {
            QueryResult::Select { rows, .. } => rows
                .into_iter()
                .filter_map(|row| match row.get("s") {
                    Some(Value::Ref(id)) => imported.resolve(*id).ok(),
                    _ => None,
                })
                .collect::<HashSet<_>>(),
            other => panic!("expected SELECT, got {other:?}"),
        }
    };

    let continuants = members(CONTINUANT);
    let occurrents = members(OCCURRENT);
    assert!(continuants.contains(&format!("{AEGIS}service-example")));
    assert!(!continuants.contains(&format!("{AEGIS}incident-example")));
    assert!(occurrents.contains(&format!("{AEGIS}incident-example")));
    assert!(!occurrents.contains(&format!("{AEGIS}service-example")));
}
