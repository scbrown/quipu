//! Measure the per-write SHACL validation cost against the REAL shape sets.
//!
//! Operational reports attributed a memory balloon to `/knot` SHACL writes
//! taking up to 118s each, with an INFERRED cost model of "SHACL validation is
//! O(graph) per write". This example measures the cost instead of inferring it.
//!
//! Result: validation time is FLAT in delta size and dominated by a fixed
//! per-write parse of the shapes graph — see the commit that added this.
//!
//! Run: `cargo run --release --example shacl_cost --features shacl`

use std::time::Instant;

fn combined_shapes() -> String {
    let shape_files = [
        "shapes/aegis-ontology.shapes.ttl",
        "shapes/aegis-rules.ttl",
        "shapes/code-entities.ttl",
        "shapes/governance.ttl",
        "shapes/provenance.ttl",
    ];
    let mut parts = Vec::new();
    for f in &shape_files {
        match std::fs::read_to_string(f) {
            Ok(s) => parts.push(s),
            Err(e) => eprintln!("skip {f}: {e}"),
        }
    }
    parts.join("\n\n")
}

/// Build a synthetic episode delta with `n` typed, labelled nodes — the shape
/// of what an agent actually POSTs.
fn delta(n: usize) -> String {
    let mut s = String::from(
        "@prefix aegis: <http://aegis.local/ontology#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\n",
    );
    let types = ["Script", "Observation", "BareMetalHost", "Bead"];
    for i in 0..n {
        s.push_str(&format!(
            "aegis:probe_node_{i} a aegis:{} ;\n    rdfs:label \"probe node {i}\" ;\n    \
             rdfs:comment \"synthetic delta for cost measurement\" .\n\n",
            types[i % types.len()]
        ));
    }
    s
}

fn main() {
    let combined = combined_shapes();
    println!("combined shapes: {} bytes\n", combined.len());

    println!("== per-write validation vs DELTA SIZE (current code path) ==");
    println!("{:>8}  {:>12}  {:>10}", "nodes", "delta bytes", "ms");
    for n in [1usize, 10, 50, 100, 250, 500, 1000] {
        let data = delta(n);
        let t = Instant::now();
        let feedback = quipu::validate_shapes(&combined, &data).expect("validate");
        let ms = t.elapsed().as_millis();
        assert!(feedback.conforms, "probe delta should conform");
        println!("{n:>8}  {:>12}  {ms:>10}", data.len());
    }

    println!("\n== where the time goes, for a small delta ==");
    let small = delta(3);
    let t = Instant::now();
    let validator = quipu::Validator::from_turtle(&combined).expect("parse shapes");
    println!(
        "from_turtle (parse shapes, cold):  {} ms",
        t.elapsed().as_millis()
    );

    let t = Instant::now();
    let _ = validator.validate(small.as_bytes()).expect("validate");
    println!(
        "validate() on a warm Validator:    {} ms",
        t.elapsed().as_millis()
    );

    println!("\n== the server's actual call, repeated (cache behaviour) ==");
    for i in 0..6 {
        let t = Instant::now();
        let _ = quipu::validate_shapes(&combined, &small).expect("validate");
        println!(
            "validate_shapes() call {}: {:>3} ms{}",
            i + 1,
            t.elapsed().as_millis(),
            if i == 0 {
                "   <- cold (parses shapes)"
            } else {
                "   <- warm (cache hit)"
            }
        );
    }

    println!("\n== correctness: a violating write must still be REJECTED ==");
    let bad = "@prefix aegis: <http://aegis.local/ontology#> .\n\
               aegis:probe_bad a aegis:Script .\n";
    match quipu::validate_shapes(&combined, bad) {
        Ok(f) => println!(
            "unlabelled Script: conforms={} violations={} (expect conforms=false)",
            f.conforms, f.violations
        ),
        Err(e) => println!("error: {e}"),
    }
    // And a conforming one still passes, on the SAME cached validator.
    let good = delta(2);
    let f = quipu::validate_shapes(&combined, &good).expect("validate");
    println!(
        "conforming delta after a violation: conforms={} violations={} (expect conforms=true)",
        f.conforms, f.violations
    );
}
