//! `quipu pack` / `quipu unpack` — the knowledge-pack CLI (quipu #81/#82).
//!
//! Split from `cli_commands.rs` for the file-size ratchet; dispatch in
//! `main.rs` is unchanged apart from the module path.

use crate::cli::{chrono_now, flag_value};

/// `quipu pack <graph-iri> --out <file>` / `quipu pack --verify <file>`
/// (quipu #81).
///
/// Top-level `pack`, deliberately not `quipu graph pack`: `quipu_graph` is an
/// MCP tool name and a `graph` subcommand would collide with it.
pub fn cmd_pack(args: &[String], db_path: &str) {
    if let Some(path) = flag_value(args, "--verify") {
        match quipu::pack::verify(path) {
            Ok((stored, recomputed, true)) => {
                println!("pack: OK\n  content_hash: {stored}");
                let _ = recomputed;
            }
            Ok((stored, recomputed, false)) => {
                eprintln!(
                    "pack: HASH MISMATCH\n  manifest:   {stored}\n  recomputed: {recomputed}\n\
                     The pack's contents do not match what it claims to be."
                );
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("pack verify error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let graph = args
        .get(2)
        .filter(|a| !a.starts_with("--"))
        .unwrap_or_else(|| {
            eprintln!(
                "usage: quipu pack <graph-iri> --out <file.qpack.db> [--name N] [--version V] \
             [--space N] [--shapes S]... [--queries Q]... [--with-vectors] [--format turtle]\n       \
             quipu pack --verify <file>"
            );
            std::process::exit(1);
        });
    let out = flag_value(args, "--out").unwrap_or_else(|| {
        eprintln!("quipu pack requires --out <file.qpack.db>");
        std::process::exit(1);
    });

    // Repeated flags collect, matching the `--predicate` idiom elsewhere.
    let multi = |name: &str| -> Vec<String> {
        args.windows(2)
            .filter(|w| w[0] == name)
            .map(|w| w[1].clone())
            .collect()
    };

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };
    // `--space N` ships the pack in term space N (quipu #74), so it attaches
    // to a consumer without id collisions.
    let space = flag_value(args, "--space").map(|s| {
        s.parse::<i64>().unwrap_or_else(|_| {
            eprintln!("--space must be an integer term-space number, got {s:?}");
            std::process::exit(1);
        })
    });
    let opts = quipu::pack::PackOptions {
        name: flag_value(args, "--name").map(String::from),
        version: flag_value(args, "--version").map(String::from),
        shapes: multi("--shapes"),
        queries: multi("--queries"),
        with_vectors: args.iter().any(|a| a == "--with-vectors"),
        space,
    };

    // `--format turtle` writes an interop BUNDLE (a directory of plain files)
    // rather than a store. Export-only: nothing unpacks it, because its purpose
    // is to be read by something that is not Quipu.
    let turtle = flag_value(args, "--format") == Some("turtle");
    let packed = if turtle {
        quipu::pack::pack_turtle(&store, graph, out, &opts, &chrono_now())
    } else {
        quipu::pack::pack(&store, graph, out, &opts, &chrono_now())
    };

    match packed {
        Ok(m) => {
            println!("packed {} -> {out}", m.source_graph);
            println!("  name:         {} {}", m.name, m.version);
            println!("  content_hash: {}", m.content_hash);
            println!("  counts:       {}", m.counts);
            println!("  term_space:   {}", m.term_space);
        }
        Err(e) => {
            eprintln!("pack error: {e}");
            std::process::exit(1);
        }
    }
}

/// `quipu unpack <pack> [--into <graph-iri>]` (quipu #82).
pub fn cmd_unpack(args: &[String], db_path: &str) {
    let Some(pack) = args.get(2).filter(|s| !s.starts_with("--")) else {
        eprintln!("usage: quipu unpack <file.qpack.db> [--into <graph-iri>] [--db <path>]");
        std::process::exit(1);
    };
    match quipu::pack::unpack(pack, db_path, flag_value(args, "--into"), &chrono_now()) {
        Ok(r) => println!(
            "unpacked {pack} into {}\n  facts: {}\n  shapes: {}\n  queries: {}",
            r.graph, r.facts, r.shapes, r.queries
        ),
        Err(e) => {
            eprintln!("unpack error: {e}");
            std::process::exit(1);
        }
    }
}
