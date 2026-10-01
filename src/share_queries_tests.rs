//! aegis-fxpbys.2: a share carries its stored queries as `queries.ttl`.

use std::collections::BTreeMap;
use std::path::Path;

use super::*;
use crate::share::{ShareOptions, share};
use crate::share_import::{PromoteImportRequest, ShareImportRequest, import_share, promote_import};

const EX: &str = "http://example.test/";
const SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
@prefix ex: <http://example.test/> .\n\
ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person .\n\
ex:TeamShape a sh:NodeShape ; sh:targetClass ex:Team .\n";
const DATA: &str = "@prefix ex: <http://example.test/> .\n\
ex:alice a ex:Person ; ex:name \"Alice\" ; ex:memberOf ex:red .\n\
ex:bob a ex:Person ; ex:name \"Bob\" ; ex:memberOf ex:blue .\n\
ex:carol a ex:Person ; ex:name \"Carol\" ; ex:memberOf ex:red .\n\
ex:red a ex:Team . ex:blue a ex:Team .\n";

fn q(name: &str, template: &str, params: Vec<StoredParam>) -> StoredQuery {
    StoredQuery {
        name: name.into(),
        description: format!("{name} description"),
        template: template.into(),
        dataset: None,
        params,
    }
}

fn param(name: &str, kind: &str, default: Option<&str>) -> StoredParam {
    StoredParam {
        name: name.into(),
        kind: kind.into(),
        required: default.is_none(),
        default: default.map(String::from),
        description: format!("the {name}"),
    }
}

/// The four fixture queries, one of them parameterized by IRI and one by text.
fn fixture_queries() -> Vec<StoredQuery> {
    vec![
        q(
            "people",
            "SELECT ?p ?name WHERE { ?p a <http://example.test/Person> ; \
             <http://example.test/name> ?name } ORDER BY ?name",
            vec![],
        ),
        q(
            "members-of",
            "SELECT ?p WHERE { ?p <http://example.test/memberOf> <{team}> } ORDER BY ?p",
            vec![param("team", "iri", None)],
        ),
        q(
            "has-team",
            "ASK { ?t a <http://example.test/Team> }",
            vec![],
        ),
        q(
            "named",
            "SELECT ?p WHERE { ?p <http://example.test/name> '{name}' }",
            vec![param("name", "text", Some("Bob"))],
        ),
    ]
}

fn ingest(store: &mut Store, turtle: &str, ts: &str) {
    crate::rdf::ingest_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        ts,
        None,
        Some("test"),
    )
    .unwrap();
}

fn producer() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    crate::share_scrub::seed_test_catalogue(&mut store);
    store
        .load_shapes("people", SHAPES, "2026-09-01T00:00:00Z")
        .unwrap();
    ingest(&mut store, DATA, "2026-09-01T00:00:01Z");
    for query in fixture_queries() {
        store.query_load(&query, "2026-09-01T00:00:02Z").unwrap();
    }
    store
}

fn receiver() -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .load_shapes("people", SHAPES, "2026-09-01T00:00:00Z")
        .unwrap();
    store
}

fn write(store: &Store, dir: &Path, name: &str, opts: &ShareOptions) -> std::path::PathBuf {
    let out = dir.join(name);
    share(store, out.to_str().unwrap(), opts).unwrap();
    out
}

fn read(dir: &Path, namespace: &str) -> ShareImportRequest {
    let mut request = crate::share_transport::read_local(dir.to_str().unwrap()).unwrap();
    request.query_namespace = Some(namespace.into());
    request
}

fn ask(store: &Store, name: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
    crate::mcp::named_query::tool_ask(
        store,
        &serde_json::json!({"name": name, "params": params.clone()}),
    )
}

fn opts() -> ShareOptions {
    ShareOptions::default()
}

/// The merge with main put two independent `manifest.ttl` additions side by
/// side: `merge_parents` (`prov:wasDerivedFrom` on the dataset) and the queries
/// distribution. A merge share that ALSO carries queries must render both, as
/// Turtle that parses, without either displacing the other.
#[test]
fn a_merge_manifest_with_queries_keeps_both_parents_and_the_queries_distribution() {
    let temp = tempfile::tempdir().unwrap();
    let dir = write(&producer(), temp.path(), "s", &opts());
    let mut manifest: crate::share::ShareManifest =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert!(
        manifest.queries_hash.is_some(),
        "control: this share carries queries"
    );
    manifest.merge_parents = vec![
        format!("sha256:{}", "a".repeat(64)),
        format!("sha256:{}", "b".repeat(64)),
    ];
    let ttl = crate::share::manifest_turtle(&manifest);
    let triples: Vec<_> = oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle)
        .for_reader(ttl.as_bytes())
        .map(|t| t.unwrap())
        .collect();
    let count = |p: &str| triples.iter().filter(|t| t.predicate.as_str() == p).count();
    assert_eq!(count("http://www.w3.org/ns/dcat#distribution"), 3, "{ttl}");
    assert_eq!(
        count("http://www.w3.org/ns/prov#wasDerivedFrom"),
        2,
        "{ttl}"
    );
    let hash = manifest
        .queries_hash
        .as_deref()
        .unwrap()
        .trim_start_matches("sha256:");
    assert!(
        triples.iter().any(|t| t.object.to_string().contains(hash)),
        "queries checksum present: {ttl}"
    );
}

#[test]
fn a_share_carries_every_registered_query_as_rdf() {
    let temp = tempfile::tempdir().unwrap();
    let dir = write(&producer(), temp.path(), "s", &opts());
    let manifest: crate::share::ShareManifest =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let member = std::fs::read_to_string(dir.join(QUERIES_FILE)).unwrap();
    let carried = from_turtle(&member).unwrap();
    assert_eq!(carried.len(), 4, "N stored queries -> N in queries.ttl");
    assert_eq!(
        manifest.queries_hash.as_deref(),
        Some(crate::share::sha256(member.as_bytes()).as_str())
    );
    assert_eq!(manifest.files.queries.as_deref(), Some(QUERIES_FILE));
    let ttl = std::fs::read_to_string(dir.join("manifest.ttl")).unwrap();
    assert!(ttl.contains("<payload:queries.ttl>"), "{ttl}");
    let distributions = oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle)
        .for_reader(ttl.as_bytes())
        .map(|t| t.unwrap())
        .filter(|t| t.predicate.as_str() == "http://www.w3.org/ns/dcat#distribution")
        .count();
    assert_eq!(
        distributions, 3,
        "manifest.ttl parses and lists queries.ttl"
    );
    let people = carried.iter().find(|s| s.query.name == "people").unwrap();
    assert_eq!(people.form, "SELECT");
    assert_eq!(people.targets, vec![format!("{EX}Person")]);
    let members = carried
        .iter()
        .find(|s| s.query.name == "members-of")
        .unwrap();
    assert_eq!(members.query.params, fixture_queries()[1].params);
    assert_eq!(
        carried
            .iter()
            .find(|s| s.query.name == "has-team")
            .unwrap()
            .form,
        "ASK"
    );
}

#[test]
fn one_query_text_changes_the_share_id_and_nothing_else() {
    let temp = tempfile::tempdir().unwrap();
    let store = producer();
    let before = share(&store, temp.path().join("a").to_str().unwrap(), &opts()).unwrap();
    let mut changed = fixture_queries().remove(0);
    changed.template.push_str(" LIMIT 10");
    store.query_load(&changed, "2026-09-01T00:00:03Z").unwrap();
    let after = share(&store, temp.path().join("b").to_str().unwrap(), &opts()).unwrap();
    assert_ne!(before.share_id, after.share_id);
    assert_ne!(before.queries_hash, after.queries_hash);
    assert_eq!(before.graph_hash, after.graph_hash);
    assert_eq!(before.shapes_hash, after.shapes_hash);
}

#[test]
fn a_share_without_queries_has_no_member_and_the_old_manifest_shape() {
    let temp = tempfile::tempdir().unwrap();
    let store = producer();
    let none = ShareOptions {
        queries: Some(Vec::new()),
        ..opts()
    };
    let dir = write(&store, temp.path(), "none", &none);
    assert!(!dir.join(QUERIES_FILE).exists());
    let json = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    assert!(!json.contains("queries"), "{json}");
    let ttl = std::fs::read_to_string(dir.join("manifest.ttl")).unwrap();
    assert!(!ttl.contains("queries.ttl"));
    // Imports exactly as before: no queries report at all.
    let result = import_share(&mut receiver(), &read(&dir, "demo"), "2026-09-02", None).unwrap();
    assert!(result.queries.is_none());
}

#[test]
fn explicit_names_select_and_an_unknown_name_refuses() {
    let temp = tempfile::tempdir().unwrap();
    let store = producer();
    let dir = write(
        &store,
        temp.path(),
        "one",
        &ShareOptions {
            queries: Some(vec!["has-team".into()]),
            ..opts()
        },
    );
    let carried = from_turtle(&std::fs::read_to_string(dir.join(QUERIES_FILE)).unwrap()).unwrap();
    assert_eq!(carried.len(), 1);
    let err = share(
        &store,
        temp.path().join("bad").to_str().unwrap(),
        &ShareOptions {
            queries: Some(vec!["nope".into()]),
            ..opts()
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("no such stored query: nope"),
        "{err}"
    );
}

#[test]
fn registered_against_follows_the_dataset_scope() {
    let mut store = producer();
    store.graph_create("urn:test:g").unwrap();
    store
        .dataset_create(
            "urn:test:ds",
            &[crate::store::datasets::DatasetMember::new("urn:test:g")],
            "2026-09-01T00:00:03Z",
            None,
        )
        .unwrap();
    let mut scoped = q("scoped", "SELECT ?s WHERE { ?s ?p ?o }", vec![]);
    scoped.dataset = Some("urn:test:ds".into());
    store.query_load(&scoped, "2026-09-01T00:00:04Z").unwrap();
    let names = |scope: &ShareScope| -> Vec<String> {
        select(&store, scope, None)
            .unwrap()
            .into_iter()
            .map(|s| s.query.name)
            .collect()
    };
    let root = names(&ShareScope::Root);
    assert_eq!(root.len(), 4);
    assert!(!root.contains(&"scoped".to_string()));
    let graph = names(&ShareScope::Graph("urn:test:g".into()));
    assert!(graph.contains(&"scoped".to_string()));
    assert_eq!(graph.len(), 5);
    assert!(!names(&ShareScope::Graph("urn:test:other".into())).contains(&"scoped".to_string()));
    // The producer-local dataset IRI never travels.
    let carried = select(&store, &ShareScope::Graph("urn:test:g".into()), None).unwrap();
    assert!(carried.iter().all(|s| s.query.dataset.is_none()));
}

#[test]
fn a_sparql_update_is_refused_at_share_time() {
    let temp = tempfile::tempdir().unwrap();
    let store = producer();
    // The registry refuses an Update at load, so plant one the way a restored
    // or hand-edited store could hold it.
    store
        .conn
        .execute(
            "INSERT INTO queries (name, description, template, dataset, valid_from, valid_to, tx) \
             VALUES ('wipe', '', 'DELETE WHERE { ?s ?p ?o }', NULL, '2026-09-01T00:00:05Z', NULL, 0)",
            [],
        )
        .unwrap();
    for selection in [None, Some(vec!["wipe".to_string()])] {
        let err = share(
            &store,
            temp.path()
                .join(format!("x{}", selection.is_some()))
                .to_str()
                .unwrap(),
            &ShareOptions {
                queries: selection,
                ..opts()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("is a SPARQL Update"), "{err}");
    }
}

/// Re-seal a share after editing its `queries.ttl`, as a hostile producer would.
fn reseal(request: &mut ShareImportRequest, member: String) {
    request.manifest.queries_hash = Some(crate::share::sha256(member.as_bytes()));
    request.queries_turtle = Some(member);
    request.manifest.share_id =
        crate::share::sha256(&crate::share::manifest_bytes(&request.manifest, false).unwrap());
}

#[test]
fn a_sparql_update_is_refused_at_import_before_anything_is_staged() {
    let temp = tempfile::tempdir().unwrap();
    let dir = write(&producer(), temp.path(), "s", &opts());
    let mut request = read(&dir, "demo");
    let member = request.queries_turtle.clone().unwrap().replace(
        "quipu:sparqlTemplate \"\"\"ASK { ?t a <http://example.test/Team> }\"\"\"",
        "quipu:sparqlTemplate \"\"\"DELETE WHERE { ?t a <http://example.test/Team> }\"\"\"",
    );
    assert!(member.contains("DELETE WHERE"), "fixture edit must apply");
    reseal(&mut request, member);
    crate::share_import::verify_share(&request).unwrap();
    let mut target = receiver();
    let err = import_share(&mut target, &request, "2026-09-02", None).unwrap_err();
    assert!(err.to_string().contains("is a SPARQL Update"), "{err}");
    assert!(target.query_list().unwrap().is_empty());
    assert!(
        target
            .lookup(&format!(
                "urn:quipu:import:staging:{}",
                &request.manifest.share_id[7..]
            ))
            .unwrap()
            .is_none(),
        "a refused member leaves nothing staged"
    );
}

#[test]
fn a_member_cannot_smuggle_data_or_misstate_its_form() {
    let base = to_turtle(&[inspect(fixture_queries().remove(2)).unwrap()]);
    let stray = format!("{base}<http://example.test/x> <http://example.test/p> \"smuggled\" .\n");
    let err = from_turtle(&stray).unwrap_err().to_string();
    assert!(
        err.contains("not a stored query") || err.contains("description does not have"),
        "{err}"
    );
    // A stray node using only query vocabulary is still not a query.
    let untyped = format!("{base}<http://example.test/x> quipu:queryName \"x\" .\n");
    let err = from_turtle(&untyped).unwrap_err().to_string();
    assert!(err.contains("not a stored query"), "{err}");
    // An extra predicate ON a query node is refused in every build, and by
    // the compiled-in SHACL shapes where that feature is on.
    let extra = base.replace(
        "quipu:queryForm \"ASK\" ;",
        "quipu:queryForm \"ASK\" ; quipu:payload \"smuggled\" ;",
    );
    assert_ne!(extra, base, "fixture edit must apply");
    assert!(from_turtle(&extra).is_err());
    #[cfg(feature = "shacl")]
    {
        assert!(conform(&base).is_ok());
        let err = conform(&extra).unwrap_err().to_string();
        assert!(err.contains("does not conform"), "{err}");
        let bad_kind = base.replace("quipu:queryForm \"ASK\"", "quipu:queryForm \"UPDATE\"");
        assert!(conform(&bad_kind).is_err(), "sh:in on queryForm");
    }
    let misstated = base.replace("quipu:queryForm \"ASK\"", "quipu:queryForm \"SELECT\"");
    assert!(from_turtle(&misstated).is_err());
    assert_eq!(from_turtle(&base).unwrap().len(), 1);
}

#[test]
fn import_namespaces_by_pack_and_reports_collisions() {
    let temp = tempfile::tempdir().unwrap();
    let dir = write(&producer(), temp.path(), "s", &opts());
    let mut target = receiver();
    // A local query with the same bare name, and a differing one already at a
    // pack-scoped name.
    let local = q("people", "SELECT ?x WHERE { ?x ?y ?z }", vec![]);
    target.query_load(&local, "2026-09-01T00:00:00Z").unwrap();
    let squatter = q("demo/has-team", "ASK { ?a ?b ?c }", vec![]);
    target
        .query_load(&squatter, "2026-09-01T00:00:00Z")
        .unwrap();

    let result = import_share(&mut target, &read(&dir, "demo"), "2026-09-02", None).unwrap();
    let report = result.queries.unwrap();
    assert_eq!(report.namespace, "demo");
    assert_eq!(
        report.installed,
        vec!["demo/members-of", "demo/named", "demo/people"]
    );
    assert_eq!(report.collisions.len(), 1);
    assert_eq!(report.collisions[0].local, "demo/has-team");
    assert_eq!(target.query_get("people").unwrap().unwrap(), local);
    assert_eq!(
        target.query_get("demo/has-team").unwrap().unwrap(),
        squatter
    );

    // Idempotent: a second import of the same share changes nothing.
    let again = import_share(&mut target, &read(&dir, "demo"), "2026-09-03", None).unwrap();
    let again = again.queries.unwrap();
    assert!(again.installed.is_empty());
    assert_eq!(again.unchanged.len(), 3);

    // The default namespace keys on the producer store, not the share.
    let mut default = crate::share_transport::read_local(dir.to_str().unwrap()).unwrap();
    default.query_namespace = None;
    let other = import_share(&mut receiver(), &default, "2026-09-02", None).unwrap();
    let ns = other.queries.unwrap().namespace;
    assert_eq!(ns, default_namespace(&default.manifest.store_id));
    assert!(ns.starts_with("pack-") && ns.len() == 17, "{ns}");

    let mut bad = read(&dir, "../escape");
    bad.query_namespace = Some("a/b".into());
    assert!(import_share(&mut receiver(), &bad, "2026-09-02", None).is_err());
}

#[test]
fn a_query_whose_vocabulary_the_receiver_lacks_is_quarantined_with_the_pack() {
    let temp = tempfile::tempdir().unwrap();
    let store = producer();
    let widget = q(
        "widgets",
        "SELECT ?w WHERE { ?w a <http://example.test/Widget> }",
        vec![],
    );
    store.query_load(&widget, "2026-09-01T00:00:03Z").unwrap();
    let dir = write(&store, temp.path(), "s", &opts());
    let mut target = receiver();
    let result = import_share(&mut target, &read(&dir, "demo"), "2026-09-02", None).unwrap();
    assert_eq!(result.outcome, "quarantined");
    assert!(
        result
            .promotion
            .blockers
            .contains(&"query_off_vocabulary".to_string()),
        "{:?}",
        result.promotion.blockers
    );
    let report = result.queries.unwrap();
    assert!(report.installed.is_empty());
    assert_eq!(report.quarantined.len(), 5, "held back WITH the pack");
    let held = report
        .quarantined
        .iter()
        .find(|h| h.name == "widgets")
        .unwrap();
    assert_eq!(held.off_vocabulary, vec![format!("{EX}Widget")]);
    assert!(target.query_list().unwrap().is_empty());
}

#[test]
fn import_runs_no_query() {
    // Evaluating this errors (MD5 is an unsupported builtin and fails loudly)
    // whenever a row reaches the FILTER. If share or import evaluated it, they
    // would fail; they succeed, and only an explicit ask errors.
    let bomb = q(
        "bomb",
        "SELECT ?s WHERE { ?s ?p ?o FILTER(MD5(STR(?o))) }",
        vec![],
    );
    let store = producer();
    store.query_load(&bomb, "2026-09-01T00:00:03Z").unwrap();
    assert!(
        ask(&store, "bomb", &serde_json::json!({})).is_err(),
        "control: it does explode"
    );
    let temp = tempfile::tempdir().unwrap();
    let dir = write(&store, temp.path(), "s", &opts());
    let mut target = receiver();
    ingest(&mut target, DATA, "2026-09-01T00:00:01Z");
    let result = import_share(&mut target, &read(&dir, "demo"), "2026-09-02", None).unwrap();
    assert!(
        result
            .queries
            .unwrap()
            .installed
            .contains(&"demo/bomb".to_string())
    );
    assert!(ask(&target, "demo/bomb", &serde_json::json!({})).is_err());
}

#[test]
fn round_trip_share_import_ask_matches_the_producer() {
    let temp = tempfile::tempdir().unwrap();
    let store = producer();
    let dir = write(&store, temp.path(), "s", &opts());
    let mut target = receiver();
    let staged = import_share(&mut target, &read(&dir, "demo"), "2026-09-02", None).unwrap();
    assert_eq!(staged.outcome, "staged", "{:?}", staged.promotion);
    promote_import(
        &mut target,
        &PromoteImportRequest {
            share_id: staged.share_id.clone(),
            actor: None,
        },
        "2026-09-02T00:00:01Z",
        None,
    )
    .unwrap();
    let calls: [(&str, serde_json::Value); 5] = [
        ("people", serde_json::json!({})),
        (
            "members-of",
            serde_json::json!({"team": format!("{EX}red")}),
        ),
        ("has-team", serde_json::json!({})),
        ("named", serde_json::json!({})),
        ("named", serde_json::json!({"name": "Carol"})),
    ];
    for (name, params) in calls {
        let there = ask(&store, name, &params).unwrap();
        let here = ask(&target, &format!("demo/{name}"), &params).unwrap();
        assert_eq!(there["columns"], here["columns"], "{name} {params}");
        assert_eq!(there["rows"], here["rows"], "{name} {params}");
        assert_eq!(there["sparql"], here["sparql"], "{name} {params}");
    }
    // Non-vacuous: the answers are real answers.
    let people = ask(&target, "demo/people", &serde_json::json!({})).unwrap();
    assert_eq!(people["count"], 3);
    let red = ask(
        &target,
        "demo/members-of",
        &serde_json::json!({"team": format!("{EX}red")}),
    )
    .unwrap();
    assert_eq!(red["count"], 2);
}

#[test]
fn a_delta_adds_replaces_and_removes_queries() {
    let temp = tempfile::tempdir().unwrap();
    let store = producer();
    let parent = write(&store, temp.path(), "parent", &opts());
    let mut target = receiver();
    let first = import_share(&mut target, &read(&parent, "demo"), "2026-09-02", None).unwrap();
    assert_eq!(first.queries.unwrap().installed.len(), 4);

    let mut people = fixture_queries().remove(0);
    people.template.push_str(" LIMIT 2");
    store.query_load(&people, "2026-09-01T00:00:10Z").unwrap();
    store
        .query_remove("has-team", "2026-09-01T00:00:10Z")
        .unwrap();
    let added = q(
        "teams",
        "SELECT ?t WHERE { ?t a <http://example.test/Team> }",
        vec![],
    );
    store.query_load(&added, "2026-09-01T00:00:10Z").unwrap();

    let delta = temp.path().join("delta");
    crate::share_delta::write_delta(
        &store,
        parent.to_str().unwrap(),
        delta.to_str().unwrap(),
        &opts(),
    )
    .unwrap();
    assert!(delta.join(QUERIES_FILE).is_file());
    let mut request =
        crate::share_delta::materialize(parent.to_str().unwrap(), delta.to_str().unwrap()).unwrap();
    request.query_namespace = Some("demo".into());
    let names: BTreeMap<String, String> = from_turtle(request.queries_turtle.as_deref().unwrap())
        .unwrap()
        .into_iter()
        .map(|s| (s.query.name, s.query.template))
        .collect();
    assert_eq!(
        names.keys().collect::<Vec<_>>(),
        vec!["members-of", "named", "people", "teams"]
    );
    assert!(names["people"].ends_with("LIMIT 2"));

    // Without replace: nothing is overwritten or closed, all of it reported.
    let cautious = import_share(&mut target, &request, "2026-09-03", None).unwrap();
    let cautious = cautious.queries.unwrap();
    assert_eq!(cautious.installed, vec!["demo/teams"]);
    assert_eq!(cautious.collisions[0].local, "demo/people");
    assert_eq!(cautious.stale, vec!["demo/has-team"]);

    // With replace: the namespace follows the pack.
    request.replace_queries = true;
    let synced = import_share(&mut target, &request, "2026-09-04", None).unwrap();
    let synced = synced.queries.unwrap();
    assert_eq!(synced.replaced, vec!["demo/people"]);
    assert_eq!(synced.removed, vec!["demo/has-team"]);
    assert!(target.query_get("demo/has-team").unwrap().is_none());
    assert!(
        target
            .query_get("demo/people")
            .unwrap()
            .unwrap()
            .template
            .ends_with("LIMIT 2")
    );

    // The delta's queries member is sealed: tamper and it refuses.
    std::fs::write(delta.join(QUERIES_FILE), "# edited\n").unwrap();
    let tampered =
        crate::share_delta::materialize(parent.to_str().unwrap(), delta.to_str().unwrap()).unwrap();
    assert!(crate::share_import::verify_share(&tampered).is_err());
}

#[test]
fn an_undeclared_or_missing_member_refuses() {
    let temp = tempfile::tempdir().unwrap();
    let dir = write(&producer(), temp.path(), "s", &opts());
    let mut missing = read(&dir, "demo");
    missing.queries_turtle = None;
    assert!(crate::share_import::verify_share(&missing).is_err());
    let none = write(
        &producer(),
        temp.path(),
        "none",
        &ShareOptions {
            queries: Some(Vec::new()),
            ..opts()
        },
    );
    let mut extra = read(&none, "demo");
    extra.queries_turtle = Some(String::new());
    assert!(crate::share_import::verify_share(&extra).is_err());
}
