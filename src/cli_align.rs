//! CLI command: `quipu align` — make `src/align` reachable (aegis-5qmg3r).
//!
//! Five correct PRs built the alignment engine and none of them wired it to a
//! surface, so `src/align` summed to a feature only the test suite could
//! invoke: `align` appeared ZERO times in `main.rs` while `knot` appeared five
//! times. This module is the CLI third of the fix; the REST routes and the MCP
//! tools are the other two, and all three call the same functions.
//!
//! ## `apply` requires `--expected-version`, and that is not a convenience flag
//!
//! `align::apply` takes an `expected_version` for optimistic concurrency, and
//! `set_version(set)` is `sha256(set.to_tsv())` — the hash of the very set being
//! applied. So a surface that *computed* the version at apply time would hash
//! the set it is about to write, always match, and silently void the guarantee:
//! two operators deciding the same proposal concurrently would each see success
//! and one decision would vanish.
//!
//! The version therefore has to be CARRIED from the decision that produced it,
//! never recomputed here. `decide` prints it; `apply` requires it; there is no
//! default. A missing flag is an error rather than a helpful guess.
//!
//! ## `propose` is not the graph-to-graph entry point
//!
//! `align::propose` takes a pre-built enumeration. Two graph IRIs go through
//! `align::enumerate::propose_from_graphs`, which enumerates both sides itself.

use quipu::align::{
    apply::{self, set_version},
    decide::{self, Decision, DecisionRow},
    enumerate::propose_from_graphs,
    propose::LinkSpec,
    sssom::MappingSet,
};

use crate::cli::{chrono_now, flag_value};

pub fn cmd_align(args: &[String], db_path: &str) {
    match args.get(2).map(String::as_str) {
        Some("propose") => cmd_propose(args, db_path),
        Some("decide") => cmd_decide(args),
        Some("apply") => cmd_apply(args, db_path),
        _ => {
            eprintln!(
                "usage: quipu align propose <graph-a> <graph-b> [--out <set.tsv>] [--db <path>]\n\
                 \x20      quipu align decide <set.tsv> --decisions <rows.tsv> \
                 --reviewer <who> [--out <set.tsv>]\n\
                 \x20      quipu align apply <set.tsv> --graph-a <iri> --graph-b <iri> \
                 --expected-version <sha> [--actor <who>] [--db <path>]\n\n\
                 `apply` requires --expected-version, printed by `decide`. It is not\n\
                 defaulted: recomputing it here would hash the set being written, always\n\
                 match, and silently void the concurrency check."
            );
            std::process::exit(1);
        }
    }
}

/// Read-only: enumerates both graphs and scores candidate pairs.
fn cmd_propose(args: &[String], db_path: &str) {
    let (Some(graph_a), Some(graph_b)) = (args.get(3), args.get(4)) else {
        eprintln!("usage: quipu align propose <graph-a> <graph-b> [--out <set.tsv>]");
        std::process::exit(1);
    };
    let store = crate::cli_open::open_store(db_path);

    // REFUSE an unknown graph rather than proposing across it.
    //
    // `align::enumerate` does `store.lookup(graph_iri)` and returns an EMPTY
    // enumeration when the IRI is not a known term. So a typo, or a namespace
    // prefix passed where a graph IRI belongs, yields "0 candidate(s)" — which
    // is indistinguishable from two graphs that genuinely share nothing, and
    // reads as a clean answer. That is the failure this whole surface exists to
    // avoid handing an operator, so the CLI checks first and says which side is
    // missing.
    for (label, iri) in [("graph-a", graph_a.as_str()), ("graph-b", graph_b.as_str())] {
        match store.lookup(iri) {
            Ok(Some(_)) => {}
            Ok(None) => {
                eprintln!(
                    "align propose: {label} '{iri}' is not a known graph in this store.\n\
                     Refusing rather than reporting 0 candidates: an unknown graph enumerates\n\
                     empty, which looks exactly like two graphs with nothing in common."
                );
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("align propose: looking up {label}: {e}");
                std::process::exit(1);
            }
        }
    }

    let set_id = flag_value(args, "--set-id").unwrap_or("urn:quipu:align:cli");
    // propose_from_graphs, NOT propose: the latter takes a prepared enumeration.
    let proposal = match propose_from_graphs(
        &store,
        graph_a,
        graph_b,
        &LinkSpec::default(),
        &MappingSet::default(),
        set_id,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("align propose: {e}");
            std::process::exit(1);
        }
    };
    let tsv = match proposal.set.to_tsv() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("align propose: serialising set: {e}");
            std::process::exit(1);
        }
    };
    // Both counts in one sentence, from the engine's own summary, so neither
    // can be reported without the other.
    eprintln!("{}", proposal.summary());
    // The version is printed HERE, at the point it is computed, because `apply`
    // must be given this exact value rather than deriving one of its own.
    match set_version(&proposal.set) {
        Ok(v) => eprintln!("expected-version: {v}"),
        Err(e) => eprintln!("align propose: version: {e}"),
    }
    emit(args, &tsv);
}

/// Read-only: applies operator decisions to a proposed set.
fn cmd_decide(args: &[String]) {
    let Some(set_path) = args.get(3) else {
        eprintln!("usage: quipu align decide <set.tsv> --decisions <rows.tsv> --reviewer <who>");
        std::process::exit(1);
    };
    let (Some(rows_path), Some(reviewer)) =
        (flag_value(args, "--decisions"), flag_value(args, "--reviewer"))
    else {
        eprintln!("align decide: --decisions and --reviewer are required");
        std::process::exit(1);
    };
    let set = read_set(set_path);
    let decisions = read_decisions(rows_path);
    let report = match decide::decide(&set, &decisions, reviewer) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("align decide: {e}");
            std::process::exit(1);
        }
    };
    let tsv = match report.set.to_tsv() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("align decide: serialising set: {e}");
            std::process::exit(1);
        }
    };
    match set_version(&report.set) {
        Ok(v) => eprintln!("expected-version: {v}   <- pass this to `align apply`"),
        Err(e) => eprintln!("align decide: version: {e}"),
    }
    emit(args, &tsv);
}

/// THE ONLY WRITER. Takes `&mut Store`; the other two take `&Store`.
fn cmd_apply(args: &[String], db_path: &str) {
    let Some(set_path) = args.get(3) else {
        eprintln!(
            "usage: quipu align apply <set.tsv> --graph-a <iri> --graph-b <iri> \
             --expected-version <sha> [--actor <who>]"
        );
        std::process::exit(1);
    };
    // Required, never defaulted — see the module docs.
    let Some(expected) = flag_value(args, "--expected-version") else {
        eprintln!(
            "align apply: --expected-version is REQUIRED.\n\
             It is printed by `align propose` and `align decide`. Pass the value from the\n\
             decision you are applying: computing it here would hash the set being written,\n\
             always match, and silently discard a concurrent operator's decision."
        );
        std::process::exit(1);
    };
    let set = read_set(set_path);
    // The two SOURCE graphs, given explicitly — the same pair passed to
    // `propose`. `derived_graph_iri` then computes the target, so the alignment
    // graph is a function of its inputs rather than a name somebody typed
    // twice. An earlier draft of this inferred the pair from the mapping IRIs'
    // prefixes; that was invented, and the engine's own tests pass the pair in.
    let (Some(graph_a), Some(graph_b)) =
        (flag_value(args, "--graph-a"), flag_value(args, "--graph-b"))
    else {
        eprintln!("align apply: --graph-a and --graph-b are required (the pair given to `propose`)");
        std::process::exit(1);
    };
    let graph_iri = apply::derived_graph_iri(graph_a, graph_b);
    let mut store = crate::cli_open::open_store(db_path);
    // Create the derived alignment graph if absent — same reasoning as the MCP
    // tool, and done here too so the two surfaces cannot diverge. The IRI is a
    // hash of the source pair, so the operator was never given a name they
    // could `graph_create` in advance.
    if let Err(e) = store.graph_create(&graph_iri) {
        eprintln!("align apply: creating alignment graph {graph_iri}: {e}");
        std::process::exit(1);
    }
    let timestamp = chrono_now();
    match apply::apply(
        &mut store,
        &set,
        &graph_iri,
        expected,
        &timestamp,
        flag_value(args, "--actor"),
    ) {
        Ok(report) => println!("{report:?}"),
        Err(e) => {
            eprintln!("align apply: {e}");
            std::process::exit(1);
        }
    }
}

fn read_set(path: &str) -> MappingSet {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("align: reading {path}: {e}");
        std::process::exit(1);
    });
    MappingSet::from_tsv(&text).unwrap_or_else(|e| {
        eprintln!("align: parsing {path}: {e}");
        std::process::exit(1);
    })
}

/// Decisions are `<subject>\t<object>\t<accept|negate>` — one pair per line.
fn read_decisions(path: &str) -> Vec<DecisionRow> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("align: reading {path}: {e}");
        std::process::exit(1);
    });
    let mut rows = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        let [subject, object, verdict] = parts[..] else {
            eprintln!("align decide: {path}:{}: expected 3 tab-separated fields", n + 1);
            std::process::exit(1);
        };
        let decision = match verdict {
            "accept" => Decision::Accept,
            "negate" => Decision::Negate,
            other => {
                eprintln!("align decide: {path}:{}: unknown decision '{other}'", n + 1);
                std::process::exit(1);
            }
        };
        rows.push(DecisionRow {
            subject_id: subject.to_string(),
            object_id: object.to_string(),
            decision,
        });
    }
    rows
}

fn emit(args: &[String], tsv: &str) {
    if let Some(out) = flag_value(args, "--out") {
        if let Err(e) = std::fs::write(out, tsv) {
            eprintln!("align: writing {out}: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {out}");
    } else {
        print!("{tsv}");
    }
}
