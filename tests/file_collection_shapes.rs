#![cfg(feature = "shacl")]

const SHAPES: &str = include_str!("../shapes/aegis-ontology.shapes.ttl");

// aegis:FileCollection (aegis-1v555n): a folder on an export, summarised.
fn file_collection_fixture(body: &str) -> String {
    format!(
        r#"
            @prefix aegis: <http://aegis.gastown.local/ontology/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            aegis:koror__datalore_unknown a aegis:NFSExport ; rdfs:label "koror:/datalore/unknown" .
            aegis:a-host rdfs:label "a host" .
            {body}
        "#
    )
}

fn file_collection_violations(body: &str) -> Vec<(String, Option<String>)> {
    quipu::validate_shapes(SHAPES, &file_collection_fixture(body))
        .unwrap()
        .results
        .into_iter()
        .map(|r| (r.focus_node, r.path))
        .collect()
}

const FC_GOOD: &str = r#"
    aegis:fc-backups a aegis:FileCollection ; rdfs:label "unknown/backups" ;
        aegis:inExport aegis:koror__datalore_unknown ;
        aegis:relativePath "backups" ;
        aegis:fileCount "212"^^xsd:integer ; aegis:byteCount "9876543"^^xsd:integer ;
        aegis:newestMtime "2024-05-01T12:00:00Z" ;
        aegis:hasExtension ".z64", ".sav", ".txt" ;
        aegis:extensionCounts ".sav=120 .z64=80 .txt=12" .
    aegis:fc-backups-n64 a aegis:FileCollection ; rdfs:label "unknown/backups/n64" ;
        aegis:inExport aegis:koror__datalore_unknown ;
        aegis:parentCollection aegis:fc-backups ;
        aegis:relativePath "backups/n64" ; aegis:hasExtension ".z64" .
"#;

#[test]
fn file_collection_accepts_a_real_collection_and_its_child() {
    assert_eq!(
        file_collection_violations(""),
        vec![],
        "control: the fixture alone conforms"
    );
    assert_eq!(file_collection_violations(FC_GOOD), vec![]);
}

#[test]
fn file_collection_refuses_malformed_values() {
    for (why, bad) in [
        (
            "extension without a dot",
            r#"aegis:fc-x a aegis:FileCollection ; rdfs:label "x" ; aegis:hasExtension "z64" ."#,
        ),
        (
            "upper-case extension",
            r#"aegis:fc-x a aegis:FileCollection ; rdfs:label "x" ; aegis:hasExtension ".Z64" ."#,
        ),
        (
            "negative count",
            r#"aegis:fc-x a aegis:FileCollection ; rdfs:label "x" ; aegis:fileCount "-1"^^xsd:integer ."#,
        ),
        (
            "two counts",
            r#"aegis:fc-x a aegis:FileCollection ; rdfs:label "x" ; aegis:fileCount "1"^^xsd:integer, "2"^^xsd:integer ."#,
        ),
        (
            "export that is not an NFSExport",
            r#"aegis:fc-x a aegis:FileCollection ; rdfs:label "x" ; aegis:inExport aegis:a-host ."#,
        ),
        (
            "mtime not UTC Z",
            r#"aegis:fc-x a aegis:FileCollection ; rdfs:label "x" ; aegis:newestMtime "2024-05-01 12:00" ."#,
        ),
        (
            "no label",
            r#"aegis:fc-x a aegis:FileCollection ; aegis:relativePath "x" ."#,
        ),
    ] {
        let v = file_collection_violations(bad);
        assert!(!v.is_empty(), "{why} must not conform");
        assert!(
            v.iter().all(|(f, _)| f.ends_with("fc-x")),
            "{why}: only the collection itself may be the focus, got {v:?}"
        );
    }
}

#[test]
fn media_library_is_a_file_collection() {
    // sattler: no second media concept. A MediaLibrary is a FileCollection,
    // so the FileCollection constraints apply to it too.
    assert!(SHAPES.contains("aegis:MediaLibrary rdfs:subClassOf aegis:FileCollection ."));
    let plex = r#"aegis:ml-plex-movies a aegis:MediaLibrary ; rdfs:label "Movies" ; aegis:platform "plex" ."#;
    assert_eq!(
        file_collection_violations(plex),
        vec![],
        "a folderless library still conforms"
    );
    let bad = r#"aegis:ml-x a aegis:MediaLibrary ; rdfs:label "x" ; aegis:hasExtension "MKV" ."#;
    assert!(
        !file_collection_violations(bad).is_empty(),
        "FileCollection's constraints must reach a MediaLibrary"
    );
}
