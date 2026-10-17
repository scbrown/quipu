//! Produce an isolated declaration for the endpoint SERVICE acceptance demo.
use quipu::{
    Store,
    share::{PointerTarget, ShareDestination, ShareOptions, SharePointer},
};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "endpoint_pointer_fixture OUTPUT_DIR ENDPOINT"
    );
    let mut store = Store::open_in_memory().unwrap();
    let catalogue = store.overlay_create("urn:test:catalogue", 0).unwrap();
    quipu::rdf::ingest_rdf_to_graph(
        &mut store,
        include_bytes!("../tests/fixtures/share-catalogue.ttl").as_slice(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-10-17T00:00:00Z",
        None,
        Some("test-catalogue"),
        catalogue,
    )
    .unwrap();
    quipu::share::share(
        &store,
        &args[1],
        &ShareOptions {
            no_shapes: true,
            destination: ShareDestination::Internal,
            pointers: vec![SharePointer {
                graph: "urn:remote".into(),
                target: PointerTarget::Endpoint {
                    endpoint: args[2].clone(),
                },
            }],
            ..Default::default()
        },
    )
    .unwrap();
}
