//! Tests for the SSSOM model and its TSV serialisation.
//!
//! The properties under test are the ones the design's acceptance criteria
//! rest on: byte-identical output for the same set (criterion 2), a rejection
//! that is expressible and findable (criterion 3), and a knot derived only from
//! a decision the operator actually made (criteria 4 and 5).

use super::sssom::{Justification, Mapping, MappingSet, OWL_SAME_AS, Review};

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
        quipu_review: None,
        quipu_reviewed_by: None,
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
fn reviewed_reports_rulings_and_skips_the_unseen() {
    let mut set = MappingSet::new("urn:quipu:align:aaa:bbb");
    let mut accepted = mapping("http://a.example/1", "http://b.example/1");
    accepted.author_id = Some("malcolm".into());
    let mut rejected = mapping("http://a.example/2", "http://b.example/2");
    rejected.author_id = Some("malcolm".into());
    rejected.predicate_modifier_not = Some(true);
    let skipped = mapping("http://a.example/3", "http://b.example/3");
    set.mappings
        .extend([accepted.clone(), rejected.clone(), skipped.clone()]);

    let reviewed = set.reviewed();
    assert_eq!(
        reviewed.len(),
        2,
        "a skipped row was never seen and must be re-proposed"
    );
    assert!(reviewed.contains_key(&accepted.pair_key()));
    assert!(reviewed.contains_key(&rejected.pair_key()));
    assert!(!reviewed.contains_key(&skipped.pair_key()));
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

// ---------------------------------------------------------------------------
// The assumption `apply` rests on for NEGATIVE mappings.
//
// The design says a rejected row "derives nothing — it exists to suppress a
// future proposal". That is true but narrower than it needs to be: quipu
// ALREADY has a durable "these two are not the same" in
// `quipu:distinctFrom`, which `recorded_distinct_from` reads and `resolve_one`
// uses to exclude candidates. So a rejection can derive a distinctFrom exactly
// as an acceptance derives an owl:sameAs, and one operator judgement then
// suppresses the pair EVERYWHERE — the next `align propose`, `/episode` ingest
// hints, and import's own resolve path — instead of only inside alignment.
//
// That only works if a fact written into the alignment's own committed named
// graph is visible to `recorded_distinct_from`. Its SQL does not filter on `g`,
// so it should be — but "should be" is how a derivation goes inert while every
// write reports success, so this pins it. If someone adds a graph filter to
// that query, alignment rejections stop suppressing anything and nothing else
// would say so.

#[test]
fn a_distinct_from_in_a_committed_named_graph_suppresses_resolution() {
    let mut store = crate::Store::open_in_memory().unwrap();
    let g = store.graph_create("urn:quipu:align:test").unwrap();
    let triple = format!(
        "<http://a.example/x> <{}> <http://b.example/y> .\n",
        crate::namespace::QUIPU_DISTINCT_FROM
    );
    crate::rdf::ingest_rdf_to_graph(
        &mut store,
        triple.as_bytes(),
        oxrdfio::RdfFormat::NTriples,
        None,
        "2026-09-05T00:00:00Z",
        None,
        Some("align-test"),
        g,
    )
    .unwrap();

    let seen = crate::resolution::recorded_distinct_from(&store, "http://a.example/x").unwrap();
    assert!(
        seen.iter().any(|s| s == "http://b.example/y"),
        "a distinctFrom in the alignment graph must reach resolution, or a rejection \
         suppresses nothing outside alignment: {seen:?}"
    );
}

// ---------------------------------------------------------------------------
// wu's review finding: a reject is TWO outcomes, and only one of them asserts.
//
// `distinctFrom` is a positive claim about the world. Most rejects in a review
// loop mean "not enough evidence", not "definitely different" — and the
// consequence is asymmetric in the dangerous direction: a wrong sameAs merges
// two entities and looks wrong to the next reader, while a wrong distinctFrom
// suppresses the pair everywhere, forever, and invisibly, because the system's
// response to it is to stop mentioning the candidate.

#[test]
fn a_declined_row_suppresses_but_asserts_nothing() {
    let mut m = mapping("http://a.example/x", "http://b.example/y");
    m.quipu_review = Some(Review::Declined);
    m.quipu_reviewed_by = Some("malcolm".into());

    assert!(
        m.is_reviewed(),
        "a declined pair must not be proposed again"
    );
    assert!(
        !m.is_decided(),
        "declining asserts nothing, so it is not an SSSOM assertion"
    );
    assert!(!m.derives_knot());
    assert!(
        !m.derives_distinct_from(),
        "absence of evidence must not become an assertion of difference"
    );
}

#[test]
fn only_an_asserted_negative_derives_distinct_from() {
    let mut asserted = mapping("http://a.example/x", "http://b.example/y");
    asserted.author_id = Some("malcolm".into());
    asserted.predicate_modifier_not = Some(true);
    assert!(asserted.derives_distinct_from());

    let mut declined = mapping("http://a.example/p", "http://b.example/q");
    declined.quipu_review = Some(Review::Declined);
    assert!(!declined.derives_distinct_from());
}

#[test]
fn a_declined_row_carries_no_author_so_sssom_readers_see_no_assertion() {
    // The encoding is the safeguard, not just our accessors: an SSSOM consumer
    // that knows nothing about `quipu_review` must not read a decline as a
    // curated mapping. `author_id` empty is what guarantees that.
    let mut set = MappingSet::new("urn:quipu:align:aaa:bbb");
    let mut declined = mapping("http://a.example/x", "http://b.example/y");
    declined.quipu_review = Some(Review::Declined);
    declined.quipu_reviewed_by = Some("malcolm".into());
    set.mappings.push(declined);

    let tsv = set.to_tsv().unwrap();
    let header: Vec<&str> = tsv
        .lines()
        .find(|l| l.starts_with("subject_id"))
        .unwrap()
        .split('\t')
        .collect();
    let row: Vec<&str> = tsv.lines().last().unwrap().split('\t').collect();
    let cell = |name: &str| row[header.iter().position(|h| *h == name).unwrap()];

    assert_eq!(
        cell("author_id"),
        "",
        "a decline must not look authored to an SSSOM reader"
    );
    assert_eq!(
        cell("predicate_modifier"),
        "",
        "a decline is not a negative mapping"
    );
    assert_eq!(cell("quipu_review"), "declined");
    assert_eq!(cell("quipu_reviewed_by"), "malcolm");
}

#[test]
fn review_state_round_trips_through_tsv() {
    let mut set = MappingSet::new("urn:quipu:align:aaa:bbb");
    let mut declined = mapping("http://a.example/x", "http://b.example/y");
    declined.quipu_review = Some(Review::Declined);
    declined.quipu_reviewed_by = Some("malcolm".into());
    set.mappings.push(declined);
    assert_eq!(MappingSet::from_tsv(&set.to_tsv().unwrap()).unwrap(), set);
}

#[test]
fn an_unknown_review_state_is_refused_not_ignored() {
    // Same reasoning as predicate_modifier: reading an unknown state as "no
    // review" puts a declined candidate back into the proposal stream.
    let tsv = "subject_id\tobject_id\tpredicate_id\tmapping_justification\tquipu_review\n\
               http://a.example/x\thttp://b.example/y\towl:sameAs\tsemapv:LexicalMatching\tmaybe-later\n";
    let err = MappingSet::from_tsv(tsv).unwrap_err().to_string();
    assert!(err.contains("quipu_review"), "got: {err}");
}

#[test]
fn asserted_different_returns_only_the_asserted_negatives() {
    let mut set = MappingSet::new("urn:quipu:align:aaa:bbb");
    let mut asserted = mapping("http://a.example/1", "http://b.example/1");
    asserted.author_id = Some("malcolm".into());
    asserted.predicate_modifier_not = Some(true);
    let mut declined = mapping("http://a.example/2", "http://b.example/2");
    declined.quipu_review = Some(Review::Declined);
    let mut accepted = mapping("http://a.example/3", "http://b.example/3");
    accepted.author_id = Some("malcolm".into());
    set.mappings.extend([asserted.clone(), declined, accepted]);

    let different = set.asserted_different();
    assert_eq!(different.len(), 1);
    assert_eq!(different[0].subject_id, asserted.subject_id);
}
