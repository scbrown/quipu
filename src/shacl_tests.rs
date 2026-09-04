use super::*;

const PERSON_SHAPE: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
    ] ;
    sh:property [
        sh:path ex:age ;
        sh:datatype xsd:integer ;
        sh:minCount 0 ;
        sh:maxCount 1 ;
        sh:minInclusive 0 ;
        sh:maxInclusive 200 ;
    ] .
"#;

#[test]
fn valid_data_passes() {
    let data = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "30"^^xsd:integer .
"#;
    let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
    assert!(feedback.conforms, "expected valid data to conform");
    assert_eq!(feedback.violations, 0);
}

#[test]
fn missing_required_property_fails() {
    let data = r#"
@prefix ex: <http://example.org/> .

ex:alice a ex:Person .
"#;
    // Missing ex:name which has sh:minCount 1
    let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
    assert!(!feedback.conforms, "expected missing name to fail");
    assert!(feedback.violations > 0);
    assert!(!feedback.results.is_empty());
}

#[test]
fn wrong_datatype_fails() {
    let data = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "not-a-number" .
"#;
    // age should be xsd:integer, but "not-a-number" is xsd:string
    let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
    assert!(!feedback.conforms, "expected wrong datatype to fail");
    assert!(feedback.violations > 0);
}

#[test]
fn value_out_of_range_fails() {
    let data = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "300"^^xsd:integer .
"#;
    // age 300 exceeds sh:maxInclusive 200
    let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
    assert!(!feedback.conforms, "expected out-of-range to fail");
}

#[test]
fn too_many_values_fails() {
    let data = r#"
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:name "Also Alice" .
"#;
    // Two names but sh:maxCount is 1
    let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
    assert!(!feedback.conforms, "expected too many names to fail");
}

/// A second, DIFFERENT shape set — used to prove the cache keys correctly
/// and does not serve one shape set's validator for another's write.
const ANIMAL_SHAPE: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:AnimalShape a sh:NodeShape ;
    sh:targetClass ex:Animal ;
    sh:property [
        sh:path ex:species ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
    ] .
"#;

/// The cache must not turn a violating write into an accepted one. The
/// FIRST call parses and caches; every later call takes the warm path, and
/// that is the path a running server spends ~all its time on — so rejection
/// has to hold there, not just on the cold call.
#[test]
fn cached_validator_still_rejects_on_warm_path() {
    let bad = r#"
@prefix ex: <http://example.org/> .
ex:bob a ex:Person .
"#;
    // 5 consecutive calls: #1 cold, #2.. warm. All must reject.
    for i in 0..5 {
        let feedback = validate_shapes(PERSON_SHAPE, bad).unwrap();
        assert!(
            !feedback.conforms,
            "violating write accepted on call {} (warm-path regression)",
            i + 1
        );
        assert!(
            feedback.violations > 0,
            "no violations reported on call {}",
            i + 1
        );
    }
}

/// Data from a previous validation must not leak into the next one. The
/// reused rudof instance keeps its shapes but `read_data` replaces the data
/// graph; if that ever stopped holding, a conforming write could be failed
/// by a PREVIOUS write's violation, or vice versa.
#[test]
fn cached_validator_does_not_leak_data_between_writes() {
    let bad = "@prefix ex: <http://example.org/> .\nex:bob a ex:Person .\n";
    let good = "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .\n";

    // Interleave: a violation must not poison the following clean write,
    // and a clean write must not mask the following violation.
    for _ in 0..3 {
        assert!(!validate_shapes(PERSON_SHAPE, bad).unwrap().conforms);
        let clean = validate_shapes(PERSON_SHAPE, good).unwrap();
        assert!(clean.conforms, "clean write failed after a violating one");
        assert_eq!(
            clean.violations, 0,
            "violations leaked from a previous write"
        );
    }
}

/// Two different shape sets must get two different validators. A cache keyed
/// only by hash, or one that ignored the stored shapes, could serve the
/// wrong schema and silently validate against the wrong contract.
#[test]
fn cache_does_not_confuse_distinct_shape_sets() {
    // Conforms to PERSON_SHAPE, and is irrelevant to ANIMAL_SHAPE.
    let person =
        "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .\n";
    // Violates ANIMAL_SHAPE (no ex:species), untargeted by PERSON_SHAPE.
    let animal = "@prefix ex: <http://example.org/> .\nex:rex a ex:Animal .\n";

    for _ in 0..3 {
        assert!(
            validate_shapes(PERSON_SHAPE, person).unwrap().conforms,
            "person data should conform to the person shapes"
        );
        assert!(
            !validate_shapes(ANIMAL_SHAPE, animal).unwrap().conforms,
            "animal violation missed — wrong shape set served from cache"
        );
        // The animal data is untargeted by the person shapes, so it conforms
        // there; if the cache served ANIMAL_SHAPE here this would fail.
        assert!(
            validate_shapes(PERSON_SHAPE, animal).unwrap().conforms,
            "person shapes should not target ex:Animal"
        );
    }
}

/// The cached validator must retain the exact shapes it was built from, so
/// a hash collision cannot silently serve a different shape set.
///
/// Runs against its own cache, not the process-global one: parallel tests
/// share the global (capacity-bounded) cache and can evict the entry
/// between the two calls, which made the `ptr_eq` assertion flaky (#84).
#[test]
fn cached_validator_retains_its_shapes() {
    let cache: ValidatorCache = Mutex::new(HashMap::new());
    let v = cached_validator_in(&cache, PERSON_SHAPE).unwrap();
    assert_eq!(v.shapes_turtle(), PERSON_SHAPE);
    let v2 = cached_validator_in(&cache, PERSON_SHAPE).unwrap();
    assert!(Arc::ptr_eq(&v, &v2), "second call should hit the cache");
}

#[test]
fn validator_reuse() {
    let validator = Validator::from_turtle(PERSON_SHAPE).unwrap();

    let good = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:name "Alice" .
"#;
    assert!(validator.validate(good.as_bytes()).unwrap().conforms);

    let bad = r#"
@prefix ex: <http://example.org/> .
ex:bob a ex:Person .
"#;
    assert!(!validator.validate(bad.as_bytes()).unwrap().conforms);
}

#[test]
fn validate_or_reject_returns_error() {
    let validator = Validator::from_turtle(PERSON_SHAPE).unwrap();

    let bad = r#"
@prefix ex: <http://example.org/> .
ex:bob a ex:Person .
"#;
    let err = validator.validate_or_reject(bad.as_bytes()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("SHACL violation"), "got: {msg}");
}

#[test]
fn feedback_has_structured_details() {
    let data = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person .
"#;
    let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
    assert!(!feedback.conforms);

    let issue = &feedback.results[0];
    assert!(!issue.focus_node.is_empty());
    assert!(!issue.component.is_empty());
    assert!(!issue.severity.is_empty());
}

#[test]
fn custom_shape_message_and_node_has_value_survive_feedback_projection() {
    let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:RequiredNode a sh:NodeShape ;
    sh:targetNode "invalid" ;
    sh:hasValue "required" ;
    sh:message "the governed message" .
"#;
    let feedback = validate_shapes(shapes, "").unwrap();
    assert!(!feedback.conforms);
    let issue = &feedback.results[0];
    assert_eq!(issue.focus_node, "invalid");
    assert_eq!(issue.value.as_deref(), Some("invalid"));
    assert_eq!(issue.message.as_deref(), Some("the governed message"));
}
