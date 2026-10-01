use super::*;

const PUBLIC: &str = "https://scbrown.github.io/quechua/ns#";
const LEGACY: &str = crate::namespace::DEFAULT_BASE_NS;

fn ingest(store: &mut Store, turtle: &str) {
    crate::rdf::ingest_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
}

/// A registry written in the legacy namespace, the Quechua one, a MIX (class
/// and verifier legacy, attests and key public), or BOTH at once, plus a
/// foreign-namespace registration that must never count. The registry matches
/// the registration class exactly (no subclass inference), as on main.
fn registry(mode: &str) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let namespaces = match mode {
        "old" => vec![(LEGACY, LEGACY, LEGACY)],
        "new" => vec![(PUBLIC, PUBLIC, PUBLIC)],
        "mixed" => vec![(LEGACY, LEGACY, PUBLIC)],
        "both" => vec![(LEGACY, LEGACY, LEGACY), (PUBLIC, PUBLIC, PUBLIC)],
        _ => unreachable!(),
    };
    for (class, name, value) in namespaces {
        ingest(
            &mut store,
            &format!(
                "<urn:registry:one> a <{class}VerifierRegistration> ; <{name}verifier> \"verifier\" ; <{value}attests> \"predicate\" ; <{value}publicKey> \"key\" ."
            ),
        );
    }
    ingest(
        &mut store,
        "<urn:registry:foreign> a <https://example.org/foreign#VerifierRegistration> ; <https://example.org/foreign#verifier> \"foreign\" ; <https://example.org/foreign#attests> \"predicate\" ; <https://example.org/foreign#publicKey> \"foreign-key\" .",
    );
    store
}

const MODES: [&str; 4] = ["old", "new", "mixed", "both"];

#[test]
fn verifier_registration_dual_namespace_reader() {
    for mode in MODES {
        let store = registry(mode);
        let now = Witness::now();
        assert!(
            is_registered_verifier(&store, "verifier", "predicate", &now).unwrap(),
            "{mode}"
        );
        assert!(
            !is_registered_verifier(&store, "verifier", "other", &now).unwrap(),
            "{mode}"
        );
        assert!(
            !is_registered_verifier(&store, "foreign", "predicate", &now).unwrap(),
            "{mode}: a foreign namespace must never authorize"
        );
    }
}

#[test]
fn public_key_dual_namespace_reader_counts_each_key_once() {
    for mode in MODES {
        let mut store = registry(mode);
        let keys = |store: &Store, who: &str| {
            registered_keys(store, who, Some("predicate"), &Witness::now(), Scope::Root).unwrap()
        };
        // "both" writes the same key in two namespaces: still ONE key.
        assert_eq!(keys(&store, "verifier"), vec!["key".to_string()], "{mode}");
        assert!(keys(&store, "foreign").is_empty(), "{mode}");
        // A second key in the Quechua namespace is a second key (rotation
        // semantics, as on main), never silently dropped.
        ingest(
            &mut store,
            &format!("<urn:registry:one> <{PUBLIC}publicKey> \"second-key\" ."),
        );
        let mut got = keys(&store, "verifier");
        got.sort();
        assert_eq!(
            got,
            vec!["key".to_string(), "second-key".to_string()],
            "{mode}"
        );
    }
}

#[test]
fn policy_scalar_dual_namespace_reader_preserves_text_and_refuses_conflicts() {
    for property in ["claim", "evidenceProbe"] {
        for namespaces in [vec![LEGACY], vec![PUBLIC], vec![LEGACY, PUBLIC]] {
            let mut store = Store::open_in_memory().unwrap();
            for ns in &namespaces {
                ingest(
                    &mut store,
                    &format!("<urn:policy> <{ns}{property}> \"ASK {{ $target ?p ?o }}\" ."),
                );
            }
            assert_eq!(
                fetch_scalar(&store, "urn:policy", property)
                    .unwrap()
                    .as_deref(),
                Some("ASK { $target ?p ?o }")
            );
            assert_eq!(fetch_scalar(&store, "urn:missing", property).unwrap(), None);
            ingest(
                &mut store,
                &format!("<urn:policy> <https://example.org/foreign#{property}> \"foreign\" ."),
            );
            assert_eq!(
                fetch_scalar(&store, "urn:policy", property)
                    .unwrap()
                    .as_deref(),
                Some("ASK { $target ?p ?o }")
            );
            ingest(
                &mut store,
                &format!("<urn:policy> <{PUBLIC}{property}> \"ASK {{}}\" ."),
            );
            assert!(
                fetch_scalar(&store, "urn:policy", property).is_err(),
                "{property}/{namespaces:?}"
            );
        }
    }
}
