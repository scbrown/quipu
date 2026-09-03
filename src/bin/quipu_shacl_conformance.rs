//! Machine-readable adapter for the W3C SHACL conformance harness.

use std::env;
use std::fs;

fn main() {
    let mut args = env::args().skip(1);
    let mut shapes = None;
    let mut data = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--shapes" => shapes = args.next(),
            "--data" => data = args.next(),
            _ => {
                eprintln!("unknown argument: {arg}");
                std::process::exit(2);
            }
        }
    }
    let (Some(shapes), Some(data)) = (shapes, data) else {
        eprintln!("usage: quipu-shacl-conformance --shapes <ttl> --data <ttl>");
        std::process::exit(2);
    };
    let shapes = fs::read_to_string(shapes).unwrap_or_else(|error| {
        eprintln!("shapes read error: {error}");
        std::process::exit(2);
    });
    let data = fs::read_to_string(data).unwrap_or_else(|error| {
        eprintln!("data read error: {error}");
        std::process::exit(2);
    });
    match quipu::validate_shapes(&shapes, &data) {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("serializable report")
        ),
        Err(error) => {
            eprintln!("validation error: {error}");
            std::process::exit(1);
        }
    }
}
