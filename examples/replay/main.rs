//! `replay` — ARM B of the shape-aware merge paper.
//!
//! Replays the two RECORDED multi-agent divergence classes from the aegis
//! production graph as merge scenarios, against the **shipped** operator
//! (`quipu::share_merge`) driven through **real share bundles** — not against
//! the benchmark's reference implementation. That distinction is the whole
//! point of the arm: agreement between a benchmark and its own reimplementation
//! of the thing under test is not evidence about the thing under test.
//!
//! ```bash
//! cargo run --example replay --features shacl
//! cargo run --example replay --features shacl -- --selftest
//! ```
//!
//! The corpus is `benchmark/replay/corpus/corpus.json`, built and pseudonymised
//! by `scripts/build-replay-corpus.py`. What this harness can and cannot
//! establish is stated in `benchmark/replay/BUILD_REPORT.md`; read it before
//! quoting a number.

use std::collections::BTreeSet;
use std::path::Path;

use quipu::rdf::ingest_rdf;
use quipu::share::{ShareOptions, share};
use quipu::share_merge::{DecisionRecord, merge, status};
use quipu::store::Store;

use oxrdfio::RdfFormat;
use serde::Deserialize;

mod case_study;

const COMMENT: &str = "http://www.w3.org/2000/01/rdf-schema#comment";
const SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";
const LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The two predicates the classes turn on, with the real cardinalities the
/// aegis shapes declare: `rdfs:comment` is functional (aegis-ontology.shapes.ttl
/// and code-entities.ttl both bound it `sh:maxCount 1`), `owl:sameAs` is not.
/// The negative control for the whole arm: identical except that
/// `rdfs:comment` carries no bound. If the 939 decisions below survive this,
/// they were never coming from the shapes and the result is an artefact.
const SHAPES_UNCONSTRAINED: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
[] sh:path <http://www.w3.org/2000/01/rdf-schema#comment> .
[] sh:path <http://www.w3.org/2000/01/rdf-schema#label> .
[] sh:path <http://www.w3.org/2002/07/owl#sameAs> .
[] sh:path <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> .
"#;

const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
[] sh:path <http://www.w3.org/2000/01/rdf-schema#comment> ; sh:maxCount 1 .
[] sh:path <http://www.w3.org/2000/01/rdf-schema#label>   ; sh:maxCount 1 .
[] sh:path <http://www.w3.org/2002/07/owl#sameAs> .
[] sh:path <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> .
"#;

#[derive(Deserialize)]
struct Corpus {
    alias_pairs: Vec<AliasPair>,
    comment_doublings: Vec<CommentDoubling>,
    counts: serde_json::Value,
}

#[derive(Deserialize)]
struct AliasPair {
    left: String,
    right: String,
    class: String,
}

#[derive(Deserialize)]
struct CommentDoubling {
    subject: String,
    values: Vec<String>,
}

fn lit(s: &str, p: &str, o: &str) -> String {
    format!(
        "<{s}> <{p}> \"{}\" .\n",
        o.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn iri(s: &str, p: &str, o: &str) -> String {
    format!("<{s}> <{p}> <{o}> .\n")
}

/// One replay scenario: three graphs plus the historical decision count.
struct Scenario {
    name: &'static str,
    what: &'static str,
    base: String,
    ours: String,
    theirs: String,
    /// Decisions the historical manual repair actually took.
    historical: usize,
    /// What a reader should conclude if the operator raises none.
    verdict_if_silent: &'static str,
}

/// Run one scenario against the SHIPPED operator, through real share bundles.
fn run(
    scenario: &Scenario,
    tmp: &Path,
    shapes: &str,
) -> (Vec<DecisionRecord>, String, usize, usize) {
    let dir = tmp.join(scenario.name);
    std::fs::create_dir_all(&dir).expect("scenario dir");

    // "theirs" is produced as a share whose parent is the base share, which is
    // how a real peer's bundle reaches us.
    let mut source = Store::open_in_memory().expect("source store");
    source
        .load_shapes("replay", shapes, "2026-08-29")
        .expect("shapes");
    ingest_rdf(
        &mut source,
        scenario.base.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:00:00Z",
        None,
        Some("base"),
    )
    .expect("ingest base");

    let base_dir = dir.join("base");
    let base_manifest = share(
        &source,
        base_dir.to_str().unwrap(),
        &ShareOptions {
            shapes: vec!["replay".into()],
            ..Default::default()
        },
    )
    .expect("base share");

    ingest_rdf(
        &mut source,
        scenario.theirs.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:01:00Z",
        None,
        Some("theirs"),
    )
    .expect("ingest theirs");
    let incoming_dir = dir.join("incoming");
    share(
        &source,
        incoming_dir.to_str().unwrap(),
        &ShareOptions {
            shapes: vec!["replay".into()],
            parent_share: Some(base_manifest.share_id.clone()),
            ..Default::default()
        },
    )
    .expect("incoming share");

    // Our side diverged independently from the same base.
    let mut local = Store::open_in_memory().expect("local store");
    local
        .load_shapes("replay", shapes, "2026-08-29")
        .expect("shapes");
    ingest_rdf(
        &mut local,
        scenario.base.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:00:00Z",
        None,
        Some("base"),
    )
    .expect("ingest base");
    ingest_rdf(
        &mut local,
        scenario.ours.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:02:00Z",
        None,
        Some("ours"),
    )
    .expect("ingest ours");

    let st = status(&local, &incoming_dir).expect("status");
    let result = merge(
        &mut local,
        &incoming_dir,
        "2026-08-29T00:03:00Z",
        Some("replay"),
    )
    .expect("merge");

    // Entities surviving in the merged store — how the alias class is caught
    // or missed is only visible here, never in the conflict list.
    let (bytes, _) =
        quipu::rdf::export_rdf_subset(&local, RdfFormat::NTriples, None).expect("export");
    let text = String::from_utf8(bytes).expect("utf8");
    let subjects: BTreeSet<&str> = text
        .lines()
        .filter_map(|l| l.split_once("> <").map(|(s, _)| s.trim_start_matches('<')))
        .collect();

    (
        result.conflicts.clone(),
        result.outcome.clone(),
        subjects.len(),
        st.theirs_added,
    )
}

fn alias_scenario(pairs: &[AliasPair], class: &str) -> (Scenario, usize) {
    let (mut base, mut ours, mut theirs) = (String::new(), String::new(), String::new());
    // One entity can appear in more than one recorded pair — an alias CHAIN
    // (`repo_quipu` was knotted both to `Quipu` and to `quipu-repo-github`).
    // A chained pair would make this arm measure an incidental collision on
    // the shared endpoint rather than alias detection, so take a disjoint
    // subset and report the remainder separately: it is a real property of
    // the corpus, not something to quietly drop.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut used = 0usize;
    let mut chained = 0usize;
    for p in pairs.iter().filter(|p| p.class == class) {
        if seen.contains(p.left.as_str()) || seen.contains(p.right.as_str()) {
            chained += 1;
            continue;
        }
        seen.insert(p.left.as_str());
        seen.insert(p.right.as_str());
        used += 1;
        // Base holds the entity under its first name, with facts.
        base.push_str(&iri(&p.left, TYPE, "https://example.org/kg/Entity"));
        base.push_str(&lit(&p.left, LABEL, "the entity"));
        // Our side adds a fact to the entity it knows.
        ours.push_str(&lit(&p.left, COMMENT, "our description"));
        // Their side re-words the NAME and so mints a second node for the
        // same underlying thing — the recorded alias-mint incident, exactly.
        theirs.push_str(&iri(&p.right, TYPE, "https://example.org/kg/Entity"));
        theirs.push_str(&lit(&p.right, LABEL, "the entity"));
        theirs.push_str(&lit(&p.right, COMMENT, "their description"));
    }
    (
        Scenario {
            name: if class == "id-form" {
                "alias-id-form"
            } else {
                "alias-semantic"
            },
            what: if class == "id-form" {
                "two id spellings for one commit (mechanically normalisable)"
            } else {
                "two phrasings for one concept (irreducibly human)"
            },
            base,
            ours,
            theirs,
            historical: used,
            verdict_if_silent: "admits both nodes, asks nothing — FALSE NEGATIVE",
        },
        chained,
    )
}

fn comment_scenario(rows: &[CommentDoubling]) -> Scenario {
    let (mut base, mut ours, mut theirs) = (String::new(), String::new(), String::new());
    for r in rows {
        base.push_str(&iri(&r.subject, TYPE, "https://example.org/kg/Entity"));
        base.push_str(&lit(&r.subject, COMMENT, &r.values[0]));
        // Each side edits the description independently — the shape that
        // production's append path turned into a silent second comment.
        ours.push_str(&lit(&r.subject, COMMENT, "our revised description"));
        theirs.push_str(&lit(&r.subject, COMMENT, "their revised description"));
    }
    Scenario {
        name: "comment-double",
        what: "both sides edit a maxCount-1 description",
        base,
        ours,
        theirs,
        historical: 0,
        verdict_if_silent: "operator would double the comment as production did",
    }
}

/// ARM C ingredient: a repair made on one side must survive a concurrent edit
/// on the other. If a merge can drop a knot, the repair path is not durable.
fn repair_scenario(pairs: &[AliasPair]) -> Scenario {
    let (mut base, mut ours, mut theirs) = (String::new(), String::new(), String::new());
    for p in pairs.iter().take(20) {
        base.push_str(&iri(&p.left, TYPE, "https://example.org/kg/Entity"));
        base.push_str(&iri(&p.right, TYPE, "https://example.org/kg/Entity"));
        // Our side performs the sameAs repair.
        ours.push_str(&iri(&p.left, SAME_AS, &p.right));
        // Their side, unaware, keeps editing one of the two nodes.
        theirs.push_str(&lit(&p.right, LABEL, "still editing the alias"));
    }
    Scenario {
        name: "sameas-repair",
        what: "a sameAs repair meets a concurrent edit to the aliased node",
        base,
        ours,
        theirs,
        historical: 20,
        verdict_if_silent: "repair survives the merge — the wanted outcome",
    }
}

fn main() {
    let selftest = std::env::args().any(|a| a == "--selftest");
    let negative = std::env::args().any(|a| a == "--negative-control");
    let case = std::env::args().any(|a| a == "--case-study");
    let shapes = if negative {
        SHAPES_UNCONSTRAINED
    } else {
        SHAPES
    };
    let corpus_path = "benchmark/replay/corpus/corpus.json";
    let raw = std::fs::read_to_string(corpus_path).unwrap_or_else(|e| {
        eprintln!("cannot read {corpus_path}: {e}\nrun scripts/build-replay-corpus.py first");
        std::process::exit(2);
    });
    let corpus: Corpus = serde_json::from_str(&raw).expect("corpus parse");

    let tmp = tempfile::tempdir().expect("tempdir");

    if case {
        println!("ARM C — share / diverge / reconnect, end to end on a real bundle\n");
        let failed = case_study::run(&corpus, tmp.path());
        if failed > 0 {
            eprintln!("\ncase study FAILED: {failed} check(s)");
            std::process::exit(1);
        }
        println!("\ncase study passed: every hop verified against the shipped operator");
        return;
    }

    let (id_form, chained_id) = alias_scenario(&corpus.alias_pairs, "id-form");
    let (semantic, chained_sem) = alias_scenario(&corpus.alias_pairs, "semantic");
    let scenarios = vec![
        id_form,
        semantic,
        comment_scenario(&corpus.comment_doublings),
        repair_scenario(&corpus.alias_pairs),
    ];

    println!("ARM B — recorded divergence replayed against the SHIPPED operator");
    println!("corpus: {}", serde_json::to_string(&corpus.counts).unwrap());
    println!();
    println!(
        "alias pairs excluded as chained (an endpoint shared with another pair): \
         {chained_id} id-form, {chained_sem} semantic"
    );
    println!();
    println!(
        "{:<16} {:>9} {:>10} {:>9}  what it means",
        "scenario", "decisions", "historical", "outcome"
    );
    println!("{}", "-".repeat(104));

    let mut failures = 0;
    for s in &scenarios {
        let (conflicts, outcome, subjects, theirs_added) = run(s, tmp.path(), shapes);
        let meaning = if conflicts.is_empty() {
            s.verdict_if_silent
        } else {
            "operator raised it as a decision"
        };
        println!(
            "{:<16} {:>9} {:>10} {:>9}  {}",
            s.name,
            conflicts.len(),
            s.historical,
            outcome,
            meaning
        );
        println!("{:<16} {}", "", s.what);
        let _ = (subjects, theirs_added);

        if negative {
            // Under the unconstrained shapes the functional class MUST go
            // silent. If it does not, the decisions are not shape-derived.
            if s.name == "comment-double" && !conflicts.is_empty() {
                eprintln!(
                    "  NEGATIVE CONTROL FAILED: {} decisions with no bound on the predicate",
                    conflicts.len()
                );
                failures += 1;
            }
        } else if selftest {
            // The instrument must be shown to fire in BOTH directions, or a
            // column of zeros is indistinguishable from a detector that is off.
            match s.name {
                "comment-double" => {
                    if conflicts.len() != s.base.lines().count() / 2 {
                        eprintln!("  SELFTEST: expected one decision per doubled subject");
                        failures += 1;
                    }
                    if outcome != "conflicts" {
                        eprintln!("  SELFTEST: a conflicting merge must not report merged");
                        failures += 1;
                    }
                }
                "alias-id-form" | "alias-semantic" => {
                    if !conflicts.is_empty() {
                        eprintln!(
                            "  SELFTEST: alias mint is not triple-visible; decisions must be 0"
                        );
                        failures += 1;
                    }
                }
                "sameas-repair" if !conflicts.is_empty() => {
                    eprintln!("  SELFTEST: an unconstrained predicate must union, not conflict");
                    failures += 1;
                }
                _ => {}
            }
        }
    }

    if negative {
        if failures > 0 {
            eprintln!("\nnegative control FAILED: the decisions are not shape-derived");
            std::process::exit(1);
        }
        println!(
            "\nnegative control passed: removing the maxCount bound silences the \
             functional class, so the decisions come from the shapes and not from \
             the harness"
        );
        return;
    }
    if selftest {
        if failures > 0 {
            eprintln!("\nselftest FAILED: {failures} check(s)");
            std::process::exit(1);
        }
        println!(
            "\nselftest passed: detector fires on the functional class and stays \
                  silent on the alias class, both observed"
        );
    }
}
