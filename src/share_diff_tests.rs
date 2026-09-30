use super::*;

fn snap(nt: &str) -> Snapshot {
    Snapshot::new(&parse_payload(nt.as_bytes(), "test").unwrap())
}

#[test]
fn compact_names_are_derived_from_the_iri_alone() {
    assert_eq!(
        compact("http://www.w3.org/2000/01/rdf-schema#label"),
        "rdfs:label"
    );
    assert_eq!(compact("http://example.org/vocab#age"), "age");
    assert_eq!(compact("http://example.org/people/alice"), "people/alice");
    assert_eq!(compact("urn:x"), "urn:x");
}

#[test]
fn nested_blank_nodes_match_by_structure_not_label() {
    let a =
        snap("<http://e/s> <http://e/p> _:a .\n_:a <http://e/q> _:b .\n_:b <http://e/r> \"v\" .\n");
    let b = snap(
        "<http://e/s> <http://e/p> _:zz .\n_:zz <http://e/q> _:yy .\n_:yy <http://e/r> \"v\" .\n",
    );
    assert!(diff(&a, &b).entities.is_empty());
    assert_eq!(render_textconv(&a), render_textconv(&b));
}

#[test]
fn a_nested_blank_node_edit_is_one_change_on_the_referencing_slot() {
    let a = snap("<http://e/s> <http://e/p> _:a .\n_:a <http://e/r> \"v\" .\n");
    let b = snap("<http://e/s> <http://e/p> _:a .\n_:a <http://e/r> \"w\" .\n");
    let d = diff(&a, &b);
    assert_eq!(d.changed, 1, "{}", render_text(&d));
    assert_eq!(d.entities[0].changed[0].old, "[ r \"v\" ]");
    assert_eq!(d.entities[0].changed[0].new, "[ r \"w\" ]");
}

#[test]
fn blank_node_cycles_are_label_independent() {
    let a =
        snap("<http://e/s> <http://e/p> _:a .\n_:a <http://e/q> _:b .\n_:b <http://e/q> _:a .\n");
    let b =
        snap("<http://e/s> <http://e/p> _:m .\n_:m <http://e/q> _:n .\n_:n <http://e/q> _:m .\n");
    assert!(diff(&a, &b).entities.is_empty());
    assert_eq!(render_textconv(&a), render_textconv(&b));
}

#[test]
fn multi_valued_slots_are_not_reported_as_changes() {
    let a = snap("<http://e/s> <http://e/p> \"1\" .\n<http://e/s> <http://e/p> \"2\" .\n");
    let b = snap("<http://e/s> <http://e/p> \"1\" .\n<http://e/s> <http://e/p> \"3\" .\n");
    let d = diff(&a, &b);
    assert_eq!((d.changed, d.added, d.removed), (0, 1, 1));
}

#[test]
fn named_graphs_are_part_of_a_fact() {
    let a = snap("<http://e/s> <http://e/p> \"1\" <http://e/g1> .\n");
    let b = snap("<http://e/s> <http://e/p> \"1\" <http://e/g2> .\n");
    let d = diff(&a, &b);
    assert_eq!((d.added, d.removed), (1, 1));
    assert!(render_text(&d).contains("@ e/g2"), "{}", render_text(&d));
}
