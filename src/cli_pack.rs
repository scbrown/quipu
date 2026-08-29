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

    let store = crate::cli_open::open_store(db_path);
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
            "unpacked {pack} into {}\n  facts: {}\n  shapes: {}\n  queries: {}\n  vectors: {}",
            r.graph, r.facts, r.shapes, r.queries, r.vectors
        ),
        Err(e) => {
            eprintln!("unpack error: {e}");
            std::process::exit(1);
        }
    }
}

/// `quipu share --output <dir>` — write a deterministic, git-native share.
pub fn cmd_share(args: &[String], db_path: &str) {
    let output = flag_value(args, "--output").unwrap_or_else(|| {
        eprintln!(
            "usage: quipu share --output <dir> [--graph <iri> | --group-id <id> | \
             --construct <query>] [--shapes <name>]... [--no-shapes] \
             [--parent-share <sha256:id>] [--turtle]"
        );
        std::process::exit(1);
    });
    let graph = flag_value(args, "--graph");
    let group = flag_value(args, "--group-id");
    let construct = flag_value(args, "--construct");
    if [graph.is_some(), group.is_some(), construct.is_some()]
        .into_iter()
        .filter(|selected| *selected)
        .count()
        > 1
    {
        eprintln!("share accepts only one of --graph, --group-id, or --construct");
        std::process::exit(1);
    }
    let scope = match (graph, group, construct) {
        (Some(iri), None, None) => quipu::share::ShareScope::Graph(iri.into()),
        (None, Some(id), None) => quipu::share::ShareScope::Group(id.into()),
        (None, None, Some(query)) => quipu::share::ShareScope::Construct(query.into()),
        (None, None, None) => quipu::share::ShareScope::Root,
        _ => unreachable!("mutually exclusive share scopes checked above"),
    };
    let shapes = args
        .windows(2)
        .filter(|w| w[0] == "--shapes")
        .map(|w| w[1].clone())
        .collect();
    let no_shapes = args.iter().any(|arg| arg == "--no-shapes");
    if no_shapes && args.iter().any(|arg| arg == "--shapes") {
        eprintln!("share accepts either --shapes or --no-shapes, not both");
        std::process::exit(1);
    }
    let opts = quipu::share::ShareOptions {
        scope,
        shapes,
        no_shapes,
        parent_share: flag_value(args, "--parent-share").map(String::from),
        turtle_view: args.iter().any(|arg| arg == "--turtle"),
    };
    let store = crate::cli_open::open_store(db_path);
    match quipu::share::share(&store, output, &opts) {
        Ok(manifest) => {
            println!("shared {}", manifest.share_id);
            println!("  graph_hash: {}", manifest.graph_hash);
            println!("  tx_anchor:  {}", manifest.tx_anchor);
            println!("  output:     {output}");
        }
        Err(error) => {
            eprintln!("share error: {error}");
            std::process::exit(1);
        }
    }
}

/// `quipu status <share-dir>` — report divergence from the share's parent.
pub fn cmd_status(args: &[String], db_path: &str) {
    let dir = args
        .get(2)
        .filter(|s| !s.starts_with("--"))
        .unwrap_or_else(|| {
            eprintln!("usage: quipu status <share-dir> [--db <path>]");
            std::process::exit(1);
        });
    let store = crate::cli_open::open_store(db_path);
    match quipu::share_merge::status(&store, std::path::Path::new(dir)) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            eprintln!("status error: {error}");
            std::process::exit(1);
        }
    }
}

/// `quipu merge <share-dir>` — shape-aware three-way reconnect into ROOT.
pub fn cmd_merge(args: &[String], db_path: &str) {
    let dir = args
        .get(2)
        .filter(|s| !s.starts_with("--"))
        .unwrap_or_else(|| {
            eprintln!("usage: quipu merge <share-dir> [--actor <id>] [--db <path>]");
            std::process::exit(1);
        });
    let mut store = crate::cli_open::open_store(db_path);
    match quipu::share_merge::merge(
        &mut store,
        std::path::Path::new(dir),
        &chrono_now(),
        flag_value(args, "--actor"),
    ) {
        Ok(result) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            if result.outcome == "conflicts" {
                std::process::exit(2);
            }
        }
        Err(error) => {
            eprintln!("merge error: {error}");
            std::process::exit(1);
        }
    }
}

/// `quipu import <share-dir>` stages a verified share; promotion is separate.
pub fn cmd_import(args: &[String], db_path: &str) {
    let mut store = crate::cli_open::open_store(db_path);
    let timestamp = chrono_now();
    if args.get(2).map(String::as_str) == Some("promote") {
        let share_id = args
            .get(3)
            .filter(|s| !s.starts_with("--"))
            .unwrap_or_else(|| {
                eprintln!("usage: quipu import promote <share-id> [--actor <id>] [--db <path>]");
                std::process::exit(1);
            });
        let request = quipu::share_import::PromoteImportRequest {
            share_id: share_id.clone(),
            actor: flag_value(args, "--actor").map(String::from),
        };
        match quipu::share_import::promote_import(&mut store, &request, &timestamp) {
            Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
            Err(error) => {
                eprintln!("import promotion error: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let dir = args
        .get(2)
        .filter(|s| !s.starts_with("--"))
        .unwrap_or_else(|| {
            eprintln!(
                "usage: quipu import <share-dir> [--source <uri>] [--actor <id>] [--db <path>]"
            );
            std::process::exit(1);
        });
    let read = |name: &str| {
        std::fs::read_to_string(std::path::Path::new(dir).join(name)).unwrap_or_else(|e| {
            eprintln!("import error reading {dir}/{name}: {e}");
            std::process::exit(1);
        })
    };
    let manifest = serde_json::from_str(&read("manifest.json")).unwrap_or_else(|e| {
        eprintln!("import manifest error: {e}");
        std::process::exit(1);
    });
    let request = quipu::share_import::ShareImportRequest {
        manifest,
        export_ntriples: read("export.nt"),
        shapes_turtle: read("shapes.ttl"),
        source: flag_value(args, "--source").unwrap_or(dir).to_string(),
        actor: flag_value(args, "--actor").map(String::from),
    };
    match quipu::share_import::import_share(&mut store, &request, &timestamp) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            eprintln!("import error: {error}");
            std::process::exit(1);
        }
    }
}
