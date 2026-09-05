//! Tests for the SSSOM model and its TSV serialisation.
//!
//! The properties under test are the ones the design's acceptance criteria
//! rest on: byte-identical output for the same set (criterion 2), a rejection
//! that is expressible and findable (criterion 3), and a knot derived only from
//! a decision the operator actually made (criteria 4 and 5).

use super::sssom::{Justification, Mapping, MappingSet, OWL_SAME_AS};

fn mapping(subject: &str, object: &str) -> Mapping {
    Mapping {
        subject_id: subject.into(),
        subject_label: Some("bobbin-release".into()),
        predicate_id: OWL_SAME_AS.into(),
        object_id: object.into(),
        object_label: Some("Bobbin_release-artifact".into()),
        mapping_justification: Justification::LexicalSimilarityThresholdMatching,
        predicate_modifier_not: None,
        confidence: Some(0.93),
        author_id: None,
    }
}

#[test]
fn tsv_round_trips() {
    let mut set = MappingSet::new("urn:quipu:align:aaa:bbb");
    set.mappings
        .push(mapping("http://a.example/x", "http://b.example/y"));
    let parsed = MappingSet::from_tsv(&set.to_tsv().unwrap()).unwrap();
    assert_eq!(
        parsed, set,
        "a set must survive its own serialisation unchanged"
    );
}

#[test]
fn tsv_is_byte_identical_for_the_same_set() {
    // Acceptance criterion 2. Serialising twice is the weak half; the real
    // property is that a set BUILT in a different order serialises the same,
    // because `propose` cannot promise the order its candidate generator ran in.
    let mut a = MappingSet::new("urn:quipu:align:aaa:bbb");
    a.mappings
        .push(mapping("http://a.example/1", "http://b.example/1"));
    a.mappings
        .push(mapping("http://a.example/2", "http://b.example/2"));
    a.sort();

    let mut b = MappingSet::new("urn:quipu:align:aaa:bbb");
    b.mappings
        .push(mapping("http://a.example/2", "http://b.example/2"));
    b.mappings
        .push(mapping("http://a.example/1", "http://b.example/1"));
    b.sort();

    assert_eq!(a.to_tsv().unwrap(), b.to_tsv().unwrap());
}

#[test]
fn an_unauthored_row_is_undecided_and_derives_nothing() {
    let m = mapping("http://a.example/x", "http://b.example/y");
    assert!(
        !m.is_decided(),
        "absence of author_id is what marks a row undecided"
    );
    assert!(
        !m.derives_knot(),
        "an undecided row must never produce a knot"
    );
}

#[test]
fn a_negated_row_derives_nothing_but_is_a_decision() {
    let mut m = mapping("http://a.example/x", "http://b.example/y");
    m.author_id = Some("malcolm".into());
    m.predicate_modifier_not = Some(true);
    assert!(m.is_decided());
    assert!(m.is_negated());
    assert!(!m.derives_knot(), "a rejection must not assert sameAs");
}

#[test]
fn an_accepted_sameas_row_derives_a_knot() {
    let mut m = mapping("http://a.example/x", "http://b.example/y");
    m.author_id = Some("malcolm".into());
    m.mapping_justification = Justification::ManualMappingCuration;
    assert!(m.derives_knot());
}

#[test]
fn a_close_match_does_not_derive_a_sameas() {
    // skos:closeMatch says the concepts are NEAR, not identical. Deriving
    // owl:sameAs from one would assert something the operator did not, so the
    // row round-trips and derives nothing (design, open question 3).
    let mut m = mapping("http://a.example/x", "http://b.example/y");
    m.author_id = Some("malcolm".into());
    m.predicate_id = "skos:closeMatch".into();
    assert!(m.is_decided());
    assert!(!m.derives_knot());
}

#[test]
fn pair_key_is_unordered_so_a_rejection_suppresses_either_direction() {
    // Acceptance criterion 3 depends on this: a pair rejected as (A, B) must
    // not come back as (B, A).
    let forward = mapping("http://a.example/x", "http://b.example/y");
    let reverse = mapping("http://b.example/y", "http://a.example/x");
    assert_eq!(forward.pair_key(), reverse.pair_key());
}

#[test]
fn decisions_reports_rulings_and_skips_the_undecided() {
    let mut set = MappingSet::new("urn:quipu:align:aaa:bbb");
    let mut accepted = mapping("http://a.example/1", "http://b.example/1");
    accepted.author_id = Some("malcolm".into());
    let mut rejected = mapping("http://a.example/2", "http://b.example/2");
    rejected.author_id = Some("malcolm".into());
    rejected.predicate_modifier_not = Some(true);
    let skipped = mapping("http://a.example/3", "http://b.example/3");
    set.mappings
        .extend([accepted.clone(), rejected.clone(), skipped.clone()]);

    let decisions = set.decisions();
    assert_eq!(
        decisions.len(),
        2,
        "a skipped row is not a decision and must be re-proposed"
    );
    assert_eq!(decisions.get(&accepted.pair_key()), Some(&false));
    assert_eq!(decisions.get(&rejected.pair_key()), Some(&true));
    assert!(!decisions.contains_key(&skipped.pair_key()));
}

#[test]
fn columns_are_read_by_name_not_position() {
    // A set a human reordered by hand, or one sssom-py wrote with its own
    // column order, must still load.
    let tsv = "#mapping_set_id: \"urn:quipu:align:aaa:bbb\"\n\
               object_id\tmapping_justification\tsubject_id\tpredicate_id\n\
               http://b.example/y\tsemapv:LexicalMatching\thttp://a.example/x\towl:sameAs\n";
    let set = MappingSet::from_tsv(tsv).unwrap();
    assert_eq!(set.mappings.len(), 1);
    assert_eq!(set.mappings[0].subject_id, "http://a.example/x");
    assert_eq!(set.mappings[0].object_id, "http://b.example/y");
}

#[test]
fn a_missing_required_column_is_refused() {
    let tsv = "subject_id\tobject_id\n\
               http://a.example/x\thttp://b.example/y\n";
    let err = MappingSet::from_tsv(tsv).unwrap_err().to_string();
    assert!(
        err.contains("predicate_id") || err.contains("mapping_justification"),
        "got: {err}"
    );
}

#[test]
fn an_unknown_predicate_modifier_is_refused_not_ignored() {
    // Reading an unrecognised modifier as "absent" would silently turn a
    // rejection back into a live proposal — the one outcome this column exists
    // to prevent.
    let tsv = "subject_id\tobject_id\tpredicate_id\tmapping_justification\tpredicate_modifier\n\
               http://a.example/x\thttp://b.example/y\towl:sameAs\tsemapv:LexicalMatching\tMaybe\n";
    let err = MappingSet::from_tsv(tsv).unwrap_err().to_string();
    assert!(err.contains("predicate_modifier"), "got: {err}");
}

#[test]
fn a_tab_in_a_field_is_refused_rather_than_silently_stripped() {
    let mut set = MappingSet::new("urn:quipu:align:aaa:bbb");
    let mut m = mapping("http://a.example/x", "http://b.example/y");
    m.subject_label = Some("has\ta tab".into());
    set.mappings.push(m);
    let err = set.to_tsv().unwrap_err().to_string();
    assert!(err.contains("subject_label"), "got: {err}");
}

#[test]
fn matched_on_strings_map_to_the_semapv_vocabulary() {
    use Justification as J;
    assert_eq!(
        J::from_matched_on("canonical_name:exact"),
        J::LexicalMatching
    );
    assert_eq!(
        J::from_matched_on("canonical_name:jaro_winkler:0.95"),
        J::LexicalSimilarityThresholdMatching
    );
    assert_eq!(
        J::from_matched_on("embedding:0.91"),
        J::EmbeddingBasedMatching
    );
}

#[test]
fn justification_curies_round_trip() {
    for j in [
        Justification::LexicalMatching,
        Justification::LexicalSimilarityThresholdMatching,
        Justification::EmbeddingBasedMatching,
        Justification::CompositeMatching,
        Justification::ManualMappingCuration,
    ] {
        assert_eq!(Justification::parse(j.curie()).unwrap(), j);
    }
}
