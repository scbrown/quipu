#![cfg(feature = "shacl")]

const SHAPES: &str = include_str!("../shapes/aegis-ontology.shapes.ttl");

fn conforms(value: &str) -> bool {
    let prefix = SHAPES
        .lines()
        .find(|line| line.starts_with("@prefix aegis:"))
        .unwrap();
    let data = format!("{prefix}\n<urn:example:episode> aegis:aboutWorkItem {value} .");
    quipu::validate_shapes(SHAPES, &data).unwrap().conforms
}

#[test]
fn work_item_link_accepts_an_external_graph_reference_without_copying_its_type() {
    assert!(conforms("<urn:example:work-item>"));
}

#[test]
fn work_item_link_rejects_literals_and_blank_nodes() {
    for value in [r#""task-123""#, "42", "true", "[]"] {
        assert!(!conforms(value), "accepted {value}");
    }
}

#[test]
fn work_item_link_has_one_primary_target() {
    assert!(!conforms("<urn:example:first>, <urn:example:second>"));
}

#[test]
fn historical_episodes_without_a_work_item_link_remain_valid() {
    assert!(
        quipu::validate_shapes(
            SHAPES,
            "<urn:example:episode> <urn:example:label> \"old\" ."
        )
        .unwrap()
        .conforms
    );
}
