//! RDF export command parsing.

use std::io::{self, Write};

use oxrdfio::RdfFormat;

pub fn cmd_export(args: &[String], db_path: &str) {
    let format = args
        .windows(2)
        .find(|w| w[0] == "--format")
        .map_or("ntriples", |w| w[1].as_str());
    let rdf_format = match format {
        "ntriples" | "nt" => RdfFormat::NTriples,
        "turtle" | "ttl" => RdfFormat::Turtle,
        _ => {
            eprintln!("unknown format: {format} (try: ntriples, turtle)");
            std::process::exit(1);
        }
    };
    let value = |flag| {
        args.windows(2)
            .find(|w| w[0] == flag)
            .map(|w| w[1].as_str())
    };
    let graph = value("--graph");
    let group = value("--group-id");
    let construct = value("--construct");
    if [graph.is_some(), group.is_some(), construct.is_some()]
        .into_iter()
        .filter(|v| *v)
        .count()
        > 1
    {
        eprintln!("export accepts only one of --graph, --group-id, or --construct");
        std::process::exit(1);
    }
    let store = crate::cli_open::open_store(db_path);
    let exported = match (graph, group, construct) {
        (Some(iri), None, None) => {
            quipu::export_rdf_subset(&store, rdf_format, Some(iri)).map(|v| v.0)
        }
        (None, Some(id), None) => quipu::export_rdf_group(&store, rdf_format, id).map(|v| v.0),
        (None, None, Some(query)) => {
            quipu::export_rdf_construct(&store, rdf_format, query).map(|v| v.0)
        }
        (None, None, None) => quipu::export_rdf(&store, rdf_format),
        _ => unreachable!("mutually exclusive export scopes checked above"),
    };
    match exported {
        Ok(bytes) => io::stdout().write_all(&bytes).unwrap(),
        Err(error) => {
            eprintln!("error exporting: {error}");
            std::process::exit(1);
        }
    }
}
