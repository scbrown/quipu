//! `quipu explain` — walk a fact's derivation chain (quipu-923, gap G8).

/// `quipu explain <subject> <predicate> <object> [--depth N] [--db <path>]`
///
/// Prints the derivation tree `crate::explain::explain` resolves: a base
/// fact shows its transaction and source; a `reasoner:<rule-id>` fact shows
/// the rule and the premises it re-matches; an `owl:materialize` fact shows
/// every axiom family that currently re-derives it, premises recursed.
pub fn cmd_explain(args: &[String], db_path: &str) {
    // Gated like `quipu ontology`: explain resolves OWL axiom families, so it
    // lives behind the `owl` feature — and refuses LOUDLY on a build without
    // it rather than parsing to nothing (the silent-flag lesson of
    // `reason --reactive`).
    #[cfg(not(feature = "owl"))]
    {
        let _ = (args, db_path);
        eprintln!("error: `quipu explain` requires the 'owl' feature");
        eprintln!("  rebuild with: cargo build --features owl (release builds use full)");
        std::process::exit(1);
    }
    #[cfg(feature = "owl")]
    {
        let positional: Vec<&String> = args[2..]
            .iter()
            .take_while(|a| !a.starts_with("--"))
            .collect();
        let [subject, predicate, object] = positional.as_slice() else {
            eprintln!(
                "usage: quipu explain <subject-IRI> <predicate-IRI> <object-IRI-or-literal> \
                 [--depth N] [--db <path>]"
            );
            std::process::exit(1);
        };
        let depth = args
            .windows(2)
            .find(|w| w[0] == "--depth")
            .and_then(|w| w[1].parse().ok())
            .unwrap_or(quipu::explain::DEFAULT_EXPLAIN_DEPTH);

        let store = crate::cli_open::open_store(db_path);
        match quipu::explain::explain(&store, subject, predicate, object, depth) {
            Ok(tree) => println!(
                "{}",
                serde_json::to_string_pretty(&tree).expect("explain output serializes")
            ),
            Err(e) => {
                eprintln!("explain error: {e}");
                std::process::exit(1);
            }
        }
    }
}
