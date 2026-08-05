//! Tests for store-context SHACL (aegis-fp17f).
//!
//! The centrepiece is `the_same_triples_in_a_different_partition_give_the_same_verdict`
//! — the acceptance criterion for this bead, stated as an executable claim
//! rather than a property anyone has to take on trust.

use super::*;

/// The shapes that produced the live failure, reduced to the two constraints
/// that interact: a module needs its own fields, and a symbol's `definedIn`
/// must point at a module. Byte-parity with the constraint is what matters,
/// not with the file.
const SHAPES: &str = r#"
@prefix sh:     <http://www.w3.org/ns/shacl#> .
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .
@prefix bobbin: <http://aegis.gastown.local/ontology/> .

bobbin:CodeModuleShape a sh:NodeShape ;
    sh:targetClass bobbin:CodeModule ;
    sh:property [ sh:path bobbin:filePath ; sh:datatype xsd:string ; sh:minCount 1 ] ;
    sh:property [ sh:path bobbin:repo ; sh:datatype xsd:string ; sh:minCount 1 ] .

bobbin:CodeSymbolShape a sh:NodeShape ;
    sh:targetClass bobbin:CodeSymbol ;
    sh:property [ sh:path bobbin:name ; sh:datatype xsd:string ; sh:minCount 1 ] ;
    sh:property [ sh:path bobbin:definedIn ; sh:class bobbin:CodeModule ; sh:minCount 1 ] .
"#;

const MODULE: &str = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
<http://x/m.rs> a bobbin:CodeModule ;
    bobbin:filePath "m.rs" ; bobbin:repo "r" .
"#;

const SYMBOL: &str = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
<http://x/m.rs::f> a bobbin:CodeSymbol ;
    bobbin:name "f" ; bobbin:definedIn <http://x/m.rs> .
"#;

fn store_with_module() -> Store {
    let mut store = Store::open_in_memory().expect("store");
    crate::rdf::ingest_rdf(
        &mut store,
        MODULE.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-05T00:00:00Z",
        None,
        Some("test"),
    )
    .expect("module ingested");
    store
}

/// THE ACCEPTANCE CRITERION (aegis-fp17f, sattler): the same triples in a
/// different partition must give the same verdict.
///
/// The control arm is the load-bearing half. Without it, "both conform" is also
/// what a validator that checks nothing would say.
#[test]
fn the_same_triples_in_a_different_partition_give_the_same_verdict() {
    let store = store_with_module();
    let whole = format!("{MODULE}\n{SYMBOL}");

    // CONTROL: the context-free validator — the behaviour being replaced — must
    // give DIFFERENT verdicts for the two partitions, or this test is not
    // exercising the defect at all.
    let ctl_whole = crate::shacl::validate_shapes(SHAPES, &whole).expect("validated");
    let ctl_split = crate::shacl::validate_shapes(SHAPES, SYMBOL).expect("validated");
    assert!(
        ctl_whole.conforms,
        "CONTROL: the whole payload must conform even without context: {:?}",
        ctl_whole.results
    );
    assert!(
        !ctl_split.conforms,
        "CONTROL FAILED: the split payload must be REFUSED without store context, \
         else this fixture does not reproduce aegis-fp17f and the assertions below \
         prove nothing"
    );

    // THE FIX: same triples, both partitions, same verdict.
    let whole_v = validate_with_store_context(&store, SHAPES, &whole).expect("validated");
    let split_v = validate_with_store_context(&store, SHAPES, SYMBOL).expect("validated");
    assert!(
        whole_v.conforms,
        "whole payload refused: {:?}",
        whole_v.results
    );
    assert!(
        split_v.conforms,
        "split payload refused — the verdict still depends on partitioning: {:?}",
        split_v.results
    );
    assert_eq!(
        whole_v.conforms, split_v.conforms,
        "same triples, different partition, different verdict"
    );
}

/// The half that is easy to leave out, and inverts the fix when you do.
///
/// Adding `<m> a bobbin:CodeModule` as context makes `m` a target of
/// `CodeModuleShape`, whose `filePath`/`repo` are not in this payload. Without
/// payload-scoped targeting the write is refused for a violation on a node it
/// never asserted — strictly WORSE than the bug being fixed, because it turns a
/// referencing write into a failure about somebody else's data.
#[test]
fn context_does_not_make_a_referenced_node_a_target() {
    let mut store = Store::open_in_memory().expect("store");
    // Deliberately INCOMPLETE module in the store: typed, but missing the
    // fields its own shape requires.
    crate::rdf::ingest_rdf(
        &mut store,
        "<http://x/m.rs> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
         <http://aegis.gastown.local/ontology/CodeModule> .\n"
            .as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-05T00:00:00Z",
        None,
        Some("test"),
    )
    .expect("ingested");

    // CONTROL: the context really is being injected — without the filter this
    // payload would now carry a CodeModule with no filePath/repo.
    let scope = scope_of(SYMBOL).expect("scoped");
    let ctx = store_type_context(&store, &scope.untyped_references).expect("context");
    assert!(
        ctx.contains("CodeModule"),
        "CONTROL FAILED: no context was fetched, so the filter below is untested"
    );
    let unfiltered =
        crate::shacl::validate_shapes(SHAPES, &format!("{SYMBOL}\n{ctx}")).expect("validated");
    assert!(
        !unfiltered.conforms,
        "CONTROL FAILED: the injected context must newly target the module, else \
         the payload-scoping filter is guarding nothing"
    );

    // THE FIX: the write is answerable for its own subject only.
    let v = validate_with_store_context(&store, SHAPES, SYMBOL).expect("validated");
    assert!(
        v.conforms,
        "a write was refused for a shape violation on a node it does not assert: {:?}",
        v.results
    );
}

/// Strictly more permissive: a payload that was refused on its OWN merits stays
/// refused. Context must not become a way to launder a genuinely bad write.
#[test]
fn a_payload_that_violates_its_own_shape_is_still_refused() {
    let store = store_with_module();
    // Symbol with no `name` — its own shape's minCount, nothing to do with the
    // store.
    let bad = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
<http://x/m.rs::g> a bobbin:CodeSymbol ;
    bobbin:definedIn <http://x/m.rs> .
"#;
    let v = validate_with_store_context(&store, SHAPES, bad).expect("validated");
    assert!(
        !v.conforms,
        "a real violation was laundered by store context"
    );
    assert!(
        !v.results.is_empty(),
        "a refusal must always carry at least one reason"
    );
    assert_eq!(
        v.violations,
        v.results
            .iter()
            .filter(|r| r.severity.to_ascii_lowercase().contains("violation"))
            .count(),
        "the count must match the kept issues, not the pre-filter total"
    );
}

/// A reference the store has never seen must still fail. The fix supplies
/// context that EXISTS; it does not assume the referent is fine.
#[test]
fn an_unknown_reference_is_still_refused() {
    let store = store_with_module();
    let dangling = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
<http://x/other.rs::h> a bobbin:CodeSymbol ;
    bobbin:name "h" ; bobbin:definedIn <http://x/nowhere.rs> .
"#;
    let v = validate_with_store_context(&store, SHAPES, dangling).expect("validated");
    assert!(
        !v.conforms,
        "a reference to a node the store does not hold must not conform"
    );
}

#[test]
fn scope_separates_what_a_payload_describes_from_what_it_references() {
    let scope = scope_of(SYMBOL).expect("scoped");
    assert!(scope.subjects.contains("http://x/m.rs::f"));
    assert!(
        !scope.subjects.contains("http://x/m.rs"),
        "a node only referenced must not count as described"
    );
    assert!(scope.untyped_references.contains("http://x/m.rs"));

    // A node the payload types needs nothing from the store.
    let whole = format!("{MODULE}\n{SYMBOL}");
    let scope = scope_of(&whole).expect("scoped");
    assert!(
        !scope.untyped_references.contains("http://x/m.rs"),
        "a node typed in the payload must not be fetched from the store"
    );
}

/// The property the whole design now rests on, pinned directly: what is
/// reported is always a SUBSET of what the payload alone produces.
///
/// This is the guard against the mistake this module already made once — the
/// first implementation added context and then filtered by focus node, which
/// was carefully reasoned and still newly refused a real chunk. A subset check
/// cannot be reasoned wrong.
#[test]
fn reported_violations_are_always_a_subset_of_the_payload_alone_verdict() {
    let mut store = Store::open_in_memory().expect("store");
    // A module typed in the store but missing its own required fields — the
    // exact shape that produced the regression at scale.
    crate::rdf::ingest_rdf(
        &mut store,
        "<http://x/m.rs> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
         <http://aegis.gastown.local/ontology/CodeModule> .\n"
            .as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-05T00:00:00Z",
        None,
        Some("test"),
    )
    .expect("ingested");

    // A payload that MENTIONS the module as a subject (an edge) without
    // describing it — the case that broke the first implementation.
    let payload = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
<http://x/m.rs::f> a bobbin:CodeSymbol ;
    bobbin:name "f" ; bobbin:definedIn <http://x/m.rs> .
<http://x/m.rs> bobbin:imports <http://x/other.rs> .
"#;

    let old = crate::shacl::validate_shapes(SHAPES, payload).expect("validated");
    let new = validate_with_store_context(&store, SHAPES, payload).expect("validated");

    assert!(
        !old.conforms,
        "CONTROL FAILED: the payload must be refused WITHOUT context, else the \
         subset claim below is vacuous"
    );
    let old_keys: std::collections::BTreeSet<String> = old
        .results
        .iter()
        .map(|r| format!("{}|{}", r.focus_node, r.component))
        .collect();
    for r in &new.results {
        assert!(
            old_keys.contains(&format!("{}|{}", r.focus_node, r.component)),
            "reported an issue the payload-alone verdict did not: {r:?}"
        );
    }
    assert!(
        new.conforms,
        "the store context did not repair the sh:class violation: {:?}",
        new.results
    );
}

/// THE AT-SCALE MEASUREMENT sattler asked for: what would NEWLY REFUSE?
///
/// Reproduces the production scenario rather than simulating it — a real
/// projection, the repo's real shape sets, and a naive partition of the kind any
/// producer that does not compensate will make. Chunk 1 is ingested so the store
/// holds the modules, exactly as it did when chunk 2 was refused live.
///
/// The regression metric is `newly_refused`: chunks the OLD validator accepted
/// and the NEW one rejects. It must be ZERO — that is the "a tightening can
/// refuse writes that currently succeed" hazard, measured rather than reasoned
/// about. The control is `refused_old > 0`: if the old validator refuses nothing
/// on this input, the fixture is not reproducing aegis-fp17f and the zero above
/// means nothing.
///
///     QUIPU_SOAK_PAYLOAD=/tmp/hank-promote-….ttl \
///       cargo test --features shacl --release -- --ignored --nocapture partition_soak
#[test]
#[ignore = "needs QUIPU_SOAK_PAYLOAD; minutes, not milliseconds"]
fn partition_soak_no_payload_is_newly_refused() {
    let Ok(path) = std::env::var("QUIPU_SOAK_PAYLOAD") else {
        panic!("set QUIPU_SOAK_PAYLOAD to a real .ttl projection");
    };
    let payload = std::fs::read_to_string(&path).expect("read payload");

    // The repo's real shape sets, unioned the way the server loads them.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shapes");
    let mut shapes = String::new();
    for entry in std::fs::read_dir(&dir).expect("shapes dir") {
        let p = entry.expect("entry").path();
        if p.extension().and_then(|e| e.to_str()) == Some("ttl") {
            shapes.push_str(&std::fs::read_to_string(&p).expect("read shape"));
            shapes.push('\n');
        }
    }
    assert!(
        shapes.contains("sh:targetClass"),
        "CONTROL FAILED: no shapes were loaded"
    );

    // A NAIVE partition — blank-line boundaries, no compensation. This is what a
    // producer that has not read aegis-sd5fj will send.
    const LIMIT: usize = 1_500_000;
    let mut blocks = payload.split("\n\n");
    let header = blocks.next().unwrap_or_default();
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::from(header);
    for b in blocks {
        if b.trim().is_empty() {
            continue;
        }
        if header.len() + 2 + b.len() <= LIMIT {
            if cur.len() + 2 + b.len() > LIMIT {
                chunks.push(std::mem::replace(&mut cur, String::from(header)));
            }
            cur.push_str("\n\n");
            cur.push_str(b);
            continue;
        }
        // The edge/symbol sections are one contiguous block of statements with
        // no blank lines. Split them at statement boundaries, as any real
        // chunker must — this is what turns 5 boundaries into ~70 and puts one
        // between a module and its symbols.
        let mut piece = String::new();
        for line in b.lines() {
            if !piece.is_empty() {
                piece.push('\n');
            }
            piece.push_str(line);
            if line.trim_end().ends_with('.') && header.len() + 2 + piece.len() > LIMIT / 2 {
                if cur.len() + 2 + piece.len() > LIMIT {
                    chunks.push(std::mem::replace(&mut cur, String::from(header)));
                }
                cur.push_str("\n\n");
                cur.push_str(&piece);
                piece.clear();
            }
        }
        if !piece.trim().is_empty() {
            if cur.len() + 2 + piece.len() > LIMIT {
                chunks.push(std::mem::replace(&mut cur, String::from(header)));
            }
            cur.push_str("\n\n");
            cur.push_str(&piece);
        }
    }
    if cur.len() > header.len() {
        chunks.push(cur);
    }
    println!(
        "payload {} bytes -> {} naive chunks",
        payload.len(),
        chunks.len()
    );
    assert!(chunks.len() > 2, "need a real partition to measure");

    // Land chunk 1, so the store holds what later chunks reference.
    let mut store = Store::open_in_memory().expect("store");
    crate::rdf::ingest_rdf(
        &mut store,
        chunks[0].as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-05T00:00:00Z",
        None,
        Some("soak"),
    )
    .expect("chunk 1 ingested");

    let (mut refused_old, mut refused_new, mut newly_refused, mut repaired) = (0, 0, 0, 0);
    for (i, c) in chunks.iter().enumerate().skip(1) {
        let old = crate::shacl::validate_shapes(&shapes, c).expect("old validated");
        let new = validate_with_store_context(&store, &shapes, c).expect("new validated");
        if !old.conforms {
            refused_old += 1;
        }
        if !new.conforms {
            refused_new += 1;
        }
        if old.conforms && !new.conforms {
            newly_refused += 1;
            println!(
                "  REGRESSION chunk {}/{}: {:?}",
                i + 1,
                chunks.len(),
                new.results.first()
            );
        }
        if !old.conforms && new.conforms {
            repaired += 1;
        }
    }
    println!(
        "chunks 2..{}: refused_old={refused_old} refused_new={refused_new} \
         repaired={repaired} newly_refused={newly_refused}",
        chunks.len()
    );
    assert!(
        refused_old > 0,
        "CONTROL FAILED: the old validator refused nothing, so this input does \
         not reproduce aegis-fp17f and newly_refused=0 proves nothing"
    );
    assert_eq!(
        newly_refused, 0,
        "the change refuses writes that used to succeed"
    );
    assert!(
        repaired > 0,
        "the fix repaired nothing — it is not doing its job"
    );
}
