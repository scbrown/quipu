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

fn registry(mode: &str) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let namespaces = match mode {
        "old" | "subclass" => vec![(LEGACY, LEGACY, LEGACY)],
        "new" | "public-subclass" => vec![(PUBLIC, PUBLIC, PUBLIC)],
        "mixed" => vec![(LEGACY, PUBLIC, LEGACY)],
        "both" => vec![(LEGACY, LEGACY, LEGACY), (PUBLIC, PUBLIC, PUBLIC)],
        _ => unreachable!(),
    };
    for (class, name, value) in namespaces {
        let kind = if mode.ends_with("subclass") {
            "urn:registration:child".to_string()
        } else {
            format!("{class}VerifierRegistration")
        };
        if mode.ends_with("subclass") {
            ingest(
                &mut store,
                &format!(
                    "<{kind}> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <{class}VerifierRegistration> ."
                ),
            );
        }
        ingest(
            &mut store,
            &format!(
                "<urn:registry:one> a <{kind}> ; <{name}verifier> \"verifier\" ; <{value}attests> \"predicate\" ; <{value}publicKey> \"key\" ."
            ),
        );
    }
    ingest(
        &mut store,
        "<urn:registry:foreign> a <https://example.org/foreign#VerifierRegistration> ; <https://example.org/foreign#verifier> \"foreign\" ; <https://example.org/foreign#attests> \"predicate\" ; <https://example.org/foreign#publicKey> \"foreign-key\" .",
    );
    store
}

#[test]
fn verifier_registration_dual_namespace_reader() {
    for mode in ["old", "new", "mixed", "both", "subclass", "public-subclass"] {
        let store = registry(mode);
        assert!(
            is_registered_verifier(&store, "verifier", "predicate").unwrap(),
            "{mode}"
        );
        assert!(
            !is_registered_verifier(&store, "verifier", "other").unwrap(),
            "{mode}"
        );
        assert!(
            !is_registered_verifier(&store, "foreign", "predicate").unwrap(),
            "{mode}"
        );
    }
}

#[test]
fn public_key_dual_namespace_reader_refuses_conflicts() {
    for mode in ["old", "new", "mixed", "both", "subclass", "public-subclass"] {
        let mut store = registry(mode);
        assert_eq!(
            registered_public_key(&store, "verifier")
                .unwrap()
                .as_deref(),
            Some("key")
        );
        assert_eq!(registered_public_key(&store, "foreign").unwrap(), None);
        ingest(
            &mut store,
            &format!("<urn:registry:one> <{PUBLIC}publicKey> \"different-key\" ."),
        );
        assert!(
            registered_public_key(&store, "verifier").is_err(),
            "{mode}: conflicting aliases cannot choose a key"
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
