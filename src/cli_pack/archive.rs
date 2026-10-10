//! Archive pack, unpack and full-store restore commands.

use crate::cli::{chrono_now, flag_value};

/// `quipu pack [graph-iri] --out <file>` / `quipu pack --verify <file>`
/// (quipu #81).
///
/// Top-level `pack`, deliberately not `quipu graph pack`: `quipu_graph` is an
/// MCP tool name and a `graph` subcommand would collide with it.
/// The `--format` values `pack` accepts. A value outside this set is REFUSED,
/// never defaulted: see the refusal in [`cmd_pack`] for why a silent fallthrough
/// publishes the wrong artifact (aegis-jpmgm8).
const PACK_FORMATS: &[&str] = &["turtle", "text"];

pub fn cmd_pack(args: &[String], db_path: &str) {
    if let Some(path) = flag_value(args, "--verify") {
        // Dispatch on the FORMAT, because the two artifacts hash differently
        // and `pack::verify` cannot recompute a full pack's hash at all: it
        // hashes canonical CURRENT-FACTS content for `manifest.source_graph`,
        // and a full pack's is the sentinel `urn:quipu:whole-store`. Measured
        // on an intact pack, this printed `unknown graph:
        // urn:quipu:whole-store` — so the lossless BACKUP was the one artifact
        // whose integrity could not be checked, which is the question a backup
        // exists to answer (aegis-9f899e).
        let verified = match quipu::pack::read_manifest(path).map(|m| m.pack_format) {
            Ok(f) if f == quipu::pack_restore::FORMAT_FULL => quipu::pack_full::verify_full(path),
            _ => quipu::pack::verify(path),
        };
        match verified {
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
        .map_or(quipu::schema::ROOT_GRAPH_IRI, String::as_str);
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

    let mut store = crate::cli_open::open_store(db_path);
    // Record the CLI-configured recipe without loading an embedding provider.
    store
        .embedding_config_mut()
        .clone_from(&crate::cli_open::config().embedding);
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
        repository: flag_value(args, "--repo").map(String::from),
        repository_sha: flag_value(args, "--repo-sha").map(String::from),
        model_id: flag_value(args, "--model-id").map(String::from),
        model_version: flag_value(args, "--model-version").map(String::from),
        allow_missing_embedding_recipe: args
            .iter()
            .any(|a| a == "--allow-missing-embedding-recipe"),
        destination: match flag_value(args, "--destination") {
            Some("internal") => quipu::share::ShareDestination::Internal,
            _ => quipu::share::ShareDestination::Outward,
        },
    };

    // `--format turtle` writes an interop BUNDLE (a directory of plain files)
    // rather than a store. Export-only: nothing unpacks it, because its purpose
    // is to be read by something that is not Quipu.
    // REFUSE an unrecognized --format rather than falling through to the
    // default (aegis-jpmgm8). Every branch below tests equality against a value
    // it knows, so without this a typo — or the documented `--format text` typed
    // against a CLI older than #244 — silently produces a DIFFERENT ARTIFACT at
    // the requested path and reports success with a valid content hash.
    //
    // Measured on the CLI installed on this host at 27f6d452: `--format text`
    // wrote a 274 KB binary SQLite whole-store pack where a text DIRECTORY was
    // asked for. The substitution hands back the artifact with the LARGER
    // disclosure surface — a binary full pack carries `events` and `vectors`,
    // which the text pack deliberately does not — under the filename chosen for
    // the small one.
    //
    // `pack_restore::reader_for` already refuses an unknown pack_format on the
    // READ path for the same reason. Guessing on the WRITE path is the wrong way
    // round: a bad read is caught, a bad write is published.
    if let Some(format) = flag_value(args, "--format").filter(|f| !PACK_FORMATS.contains(f)) {
        eprintln!(
            "pack error: unknown --format {format:?}. Accepted: {}. \
             Refusing rather than writing a different artifact than the one \
             asked for (aegis-jpmgm8).",
            PACK_FORMATS
                .iter()
                .map(|f| format!("{f:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::process::exit(2);
    }
    let turtle = flag_value(args, "--format") == Some("turtle");
    // `--format text` is the LOSSLESS text whole-store pack (aegis-9f899e):
    // git-friendly AND reconstructing, which neither the share (text, lossy)
    // nor `--full` (lossless, a sqlite blob) is on its own.
    let text = flag_value(args, "--format") == Some("text");
    // `--full` is a DIFFERENT ARTIFACT, not a mode of this one: a lossless
    // whole-store copy for internal backup, which takes no graph and refuses an
    // outward destination. Dispatched here rather than folded into `pack` so the
    // two contracts stay separable (aegis-9f899e).
    let full = args.iter().any(|a| a == "--full");
    let packed = if text && !full {
        Err(quipu::error::Error::InvalidValue(
            "pack --format text is a whole-store artifact and needs --full. For a \
             text artifact of the CURRENT FACTS, which is a different contract, \
             use `quipu share --output <dir>`."
                .into(),
        ))
    } else if full && text {
        quipu::pack_full_text::pack_full_text(&store, out, &opts, &chrono_now())
    } else if full && turtle {
        Err(quipu::error::Error::InvalidValue(
            "pack --full --format turtle: a full pack is a whole-store artifact,              not an interop bundle. Use one or the other."
                .into(),
        ))
    } else if full {
        quipu::pack_full::pack_full(&store, out, &opts, &chrono_now())
    } else if turtle {
        quipu::pack::pack_turtle(&store, graph, out, &opts, &chrono_now())
    } else {
        quipu::pack::pack(&store, graph, out, &opts, &chrono_now())
    };

    match packed {
        Ok(m) => {
            if m.pack_format == quipu::pack_full_text::FORMAT_FULL_TEXT
                && let Some(warning) = quipu::pack_full_text::pack_recipe_warning(&m.counts)
            {
                eprintln!("WARNING: {warning}");
            }
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
    let opts = quipu::pack::LoadOptions {
        into: flag_value(args, "--into"),
        expect_repository: flag_value(args, "--expect-repo"),
        head_sha: flag_value(args, "--head-sha"),
    };
    match quipu::pack::unpack_verified(pack, db_path, &opts, &chrono_now()) {
        Ok(r) => println!(
            "{} {pack} into {}\n  facts: {}\n  shapes: {}\n  queries: {}\n  vectors: {}\n  repository_sha: {}\n  head_sha: {}",
            r.outcome,
            r.graph,
            r.facts,
            r.shapes,
            r.queries,
            r.vectors,
            r.repository_sha.as_deref().unwrap_or("-"),
            r.head_sha.as_deref().unwrap_or("-")
        ),
        Err(e) => {
            eprintln!("unpack error: {e}");
            std::process::exit(1);
        }
    }
}

/// `quipu restore <full-pack> [--force]` — REPLACE this store with a full pack.
///
/// The sibling of [`cmd_unpack`] and deliberately a different verb: `unpack`
/// merges, `restore` replaces, and which one happens is declared by the
/// operator rather than inferred from how empty the destination looks
/// (aegis-9f899e, settled with wu).
pub fn cmd_restore(args: &[String], db_path: &str) {
    let Some(pack) = args.get(2).filter(|s| !s.starts_with("--")) else {
        eprintln!(
            "usage: quipu restore <file.qpack> [--force] [--db <path>]\n       \
             REPLACES the store at --db with the pack's whole contents. To MERGE a \
             published pack into an existing store, use `quipu unpack` instead."
        );
        std::process::exit(1);
    };
    let force = args.iter().any(|a| a == "--force");
    match quipu::pack_restore::restore(pack, db_path, force) {
        Ok(r) => {
            println!(
                "restored {pack} -> {}\n  content_hash: {}\n  tables:       {}\n  replaced:     {} live fact(s)",
                r.destination, r.content_hash, r.tables, r.replaced_facts
            );
            // Printed only when there IS something to rebuild. A restore that
            // carried everything says nothing here rather than printing a
            // reassurance, and an incomplete one cannot be mistaken for
            // complete by an operator reading the success line (aegis-9f899e).
            if let Some(notice) = r.regenerate {
                println!("  REGENERATE:   {notice}");
                println!(
                    "                facts, history and provenance are complete; \
                     derived data is NOT, until this is rebuilt."
                );
            }
        }
        Err(e) => {
            eprintln!("restore error: {e}");
            std::process::exit(1);
        }
    }
}
