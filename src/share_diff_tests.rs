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

#[test]
fn colliding_predicate_names_carry_their_iri() {
    const LABELS: &str = "<http://example.org/ex/name> <http://www.w3.org/2000/01/rdf-schema#label> \"name\" .\n<https://schema.org/name> <http://www.w3.org/2000/01/rdf-schema#label> \"name\" .\n";
    let a = snap(&format!(
        "{LABELS}<http://e/s> <http://example.org/ex/name> \"Alice\" .\n"
    ));
    let b = snap(&format!(
        "{LABELS}<http://e/s> <https://schema.org/name> \"Alice\" .\n"
    ));
    let d = diff(&a, &b);
    assert_eq!((d.changed, d.added, d.removed), (0, 1, 1));
    let text = render_text(&d);
    assert!(text.contains("  - name (ex/name): \"Alice\"\n"), "{text}");
    assert!(
        text.contains("  + name (schema:name): \"Alice\"\n"),
        "{text}"
    );
    let md = render_markdown(&d);
    assert!(md.contains("**name (ex/name)**") && md.contains("**name (schema:name)**"));
    let json = serde_json::to_string(&d).unwrap();
    assert!(json.contains("name (ex/name)") && json.contains("name (schema:name)"));
    // Both on one entity in a textconv; and a label-free collision whose
    // compact forms also collide falls back to the full IRI.
    let both = snap(&format!(
        "{LABELS}<http://e/s> <http://example.org/ex/name> \"A\" .\n<http://e/s> <https://schema.org/name> \"A\" .\n<http://e/s> <http://a.org/x/v> \"1\" .\n<http://e/s> <http://b.org/x/v> \"1\" .\n"
    ));
    let tc = render_textconv(&both);
    for line in [
        "  name (ex/name): \"A\"",
        "  name (schema:name): \"A\"",
        "  v (<http://a.org/x/v>): \"1\"",
        "  v (<http://b.org/x/v>): \"1\"",
    ] {
        assert!(tc.contains(line), "{line} missing from:\n{tc}");
    }
    // CONTROL: a predicate with no collision keeps its plain name.
    let one = snap("<http://e/s> <http://a.org/x/v> \"1\" .\n");
    assert!(render_textconv(&one).contains("  v: \"1\""));
}

#[test]
fn a_duplicate_identical_blank_node_is_a_visible_change() {
    let one = "<http://e/s> <http://e/p> _:a .\n_:a <http://e/r> \"v\" .\n";
    let two = format!("{one}<http://e/s> <http://e/p> _:b .\n_:b <http://e/r> \"v\" .\n");
    let d = diff(&snap(one), &snap(&two));
    let text = render_text(&d);
    assert_eq!(d.changed, 1, "{text}");
    assert!(
        text.contains("  ~ p: [ r \"v\" ] x1 -> [ r \"v\" ] x2\n"),
        "{text}"
    );
    assert!(render_textconv(&snap(&two)).contains("  p: [ r \"v\" ] x2\n"));
    assert!(!render_textconv(&snap(one)).contains(" x"));
    // Relabelling the same two nodes is still no change at all.
    let relabelled = two.replace("_:a", "_:m").replace("_:b", "_:n");
    assert!(diff(&snap(&two), &snap(&relabelled)).entities.is_empty());
    // Adding a duplicate where none existed shows the count on the new fact.
    let d = diff(&snap("<http://e/s> <http://e/q> \"x\" .\n"), &snap(&two));
    assert!(
        render_text(&d).contains("  + p: [ r \"v\" ] x2\n"),
        "{}",
        render_text(&d)
    );
}
