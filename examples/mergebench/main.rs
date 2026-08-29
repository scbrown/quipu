//! `mergebench` — the divergence benchmark behind the shape-aware merge paper.
//!
//! One seeded base graph, two independently edited copies, seven merge
//! strategies scored against one oracle. No LLM anywhere: the writers are
//! deterministic drivers, so the run is its own ground truth.
//!
//! ```bash
//! just bench merge                              # seed 42, defaults
//! just bench merge --overlap 0.8 --entities 400
//! just bench merge --sweep                      # wall time vs graph size
//! ```
//!
//! What this harness can and cannot establish is stated in
//! `benchmark/mergebench/BUILD_REPORT.md`; read it before quoting a number.

mod generate;
mod model;
mod rng;
mod score;
mod selftest;
mod shapes;
mod strategies;

use generate::Params;

fn usage() -> ! {
    eprintln!(
        "usage: mergebench [--seed <u64>] [--entities <n>] [--edits <n>] \
         [--overlap <0.0..1.0>] [--sweep] [--selftest] [--out <dir>]"
    );
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut params = Params { entities: 200, edits_per_side: 200, overlap: 0.5, seed: 42 };
    let mut out = String::from("benchmark/mergebench/out");
    let mut sweep = false;
    let mut selftest = false;
    let mut i = 1;
    while i < args.len() {
        let next = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_else(|| usage())
        };
        match args[i].as_str() {
            "--seed" => params.seed = next(&mut i).parse().unwrap_or_else(|_| usage()),
            "--entities" => params.entities = next(&mut i).parse().unwrap_or_else(|_| usage()),
            "--edits" => params.edits_per_side = next(&mut i).parse().unwrap_or_else(|_| usage()),
            "--overlap" => params.overlap = next(&mut i).parse().unwrap_or_else(|_| usage()),
            "--out" => out = next(&mut i),
            "--sweep" => sweep = true,
            "--selftest" => selftest = true,
            _ => usage(),
        }
        i += 1;
    }
    if params.entities == 0 || !(0.0..=1.0).contains(&params.overlap) {
        usage();
    }

    if selftest {
        selftest::run(params);
        return;
    }

    std::fs::create_dir_all(&out).expect("create output directory");
    if !strategies::git_available() {
        eprintln!(
            "warning: `git` not found — the two line-merge arms will be reported \
             as unavailable rather than as zero"
        );
    }

    if sweep {
        run_sweep(params, &out);
        return;
    }
    run_one(params, &out);
}

fn run_one(params: Params, out: &str) {
    let scenario = generate::scenario(params);
    let arms = score::score(&scenario);

    let mut by_class: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for class in scenario.truth.values() {
        *by_class.entry(class.as_str()).or_default() += 1;
    }

    let body = serde_json::json!({
        "params": scenario.params,
        "graphs": {
            "base_triples": scenario.base.len(),
            "ours_triples": scenario.ours.len(),
            "theirs_triples": scenario.theirs.len(),
            "edits_applied": scenario.edits,
        },
        "oracle": {
            "conflicts": scenario.truth.len(),
            "by_class": by_class,
        },
        "arms": arms,
    });
    let path = format!("{out}/metrics-seed{}.json", params.seed);
    std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&body).unwrap()))
        .expect("write metrics");
    // The same numbers as a markdown table. Prose that quotes a figure cites
    // THIS file rather than retyping it — a hand-carried number is a number
    // with no command behind it.
    let md = format!("{out}/RESULTS-seed{}.md", params.seed);
    std::fs::write(&md, results_markdown(&scenario, &arms)).expect("write results table");

    println!(
        "mergebench seed={} entities={} edits={} overlap={}",
        params.seed, params.entities, params.edits_per_side * 2, params.overlap
    );
    println!("oracle: {} conflicts {by_class:?}\n", scenario.truth.len());
    println!(
        "{:<14} {:>6} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6} {:>6} {:>8}",
        "arm", "flag", "TP", "FP", "FN", "prec", "recall", "shacl", "bad", "merge_us"
    );
    for a in &arms {
        if !a.available {
            println!("{:<14} {:>6}", a.arm, "n/a");
            continue;
        }
        let f = |v: Option<f64>| v.map_or_else(|| "-".to_string(), |x| format!("{x:.3}"));
        println!(
            "{:<14} {:>6} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6} {:>6} {:>8}",
            a.arm,
            a.human_decisions,
            a.true_positives,
            a.false_positives,
            a.false_negatives,
            f(a.precision),
            f(a.recall),
            a.shacl_violations,
            a.unparseable_lines,
            a.merge_us,
        );
    }
    println!("\n-> {path}\n-> {md}");
}

/// Render the scored arms as a markdown table, with the command that produced
/// them in the header so the table is self-describing wherever it is pasted.
fn results_markdown(scenario: &generate::Scenario, arms: &[score::ArmMetrics]) -> String {
    let p = scenario.params;
    let mut s = format!(
        "# mergebench results\n\n\
         Generated by:\n\n```bash\njust bench merge --seed {} --entities {} \
         --edits {} --overlap {}\n```\n\n\
         Base graph {} triples; {} edits applied across both sides. Oracle: {} \
         conflicts.\n\n",
        p.seed,
        p.entities,
        p.edits_per_side,
        p.overlap,
        scenario.base.len(),
        scenario.edits,
        scenario.truth.len(),
    );
    s.push_str(
        "| arm | decisions | /1k edits | TP | FP | FN | precision | recall | \
         SHACL violations | unparseable lines | triples lost | triples spurious | merge us |\n\
         |---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|\n",
    );
    let f = |v: Option<f64>| v.map_or_else(|| "-".to_string(), |x| format!("{x:.3}"));
    for a in arms {
        if !a.available {
            s.push_str(&format!(
                "| `{}` | *unavailable in this environment* |||||||||||\n",
                a.arm
            ));
            continue;
        }
        s.push_str(&format!(
            "| `{}` | {} | {:.1} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            a.arm,
            a.human_decisions,
            a.decisions_per_1k_edits,
            a.true_positives,
            a.false_positives,
            a.false_negatives,
            f(a.precision),
            f(a.recall),
            a.shacl_violations,
            a.unparseable_lines,
            a.triples_lost,
            a.triples_spurious,
            a.merge_us,
        ));
    }
    s.push_str("\n## Recall by conflict class\n\nWhich conflicts an arm can SEE, \
                rather than how many it raises.\n\n| arm |");
    let classes: Vec<&String> = arms
        .first()
        .map(|a| a.recall_by_class.keys().collect())
        .unwrap_or_default();
    for c in &classes {
        s.push_str(&format!(" {c} |"));
    }
    s.push_str("\n|---|");
    for _ in &classes {
        s.push_str("--:|");
    }
    s.push('\n');
    for a in arms {
        if !a.available {
            continue;
        }
        s.push_str(&format!("| `{}` |", a.arm));
        for c in &classes {
            let r = &a.recall_by_class[*c];
            s.push_str(&format!(" {}/{} |", r.detected, r.declared));
        }
        s.push('\n');
    }
    s.push_str(
        "\nEvery arm scores 0 on `alias-mint`, including the shape-aware one. \
         Two names for one entity is not visible to any triple-level operator; \
         it is reported rather than excluded. See `BUILD_REPORT.md` §4.\n",
    );
    s
}

/// Wall time versus graph size, at a fixed edit-to-entity ratio.
fn run_sweep(base: Params, out: &str) {
    let mut rows = Vec::new();
    println!("{:>9} {:>9}  arm timings (us)", "entities", "triples");
    for entities in [50usize, 100, 200, 400, 800, 1600] {
        let params = Params { entities, edits_per_side: entities, ..base };
        let scenario = generate::scenario(params);
        let arms = score::score(&scenario);
        println!(
            "{entities:>9} {:>9}  {}",
            scenario.base.len(),
            arms.iter()
                .filter(|a| a.available)
                .map(|a| format!("{}={}", a.arm, a.merge_us))
                .collect::<Vec<_>>()
                .join(" ")
        );
        rows.push(serde_json::json!({
            "params": params,
            "base_triples": scenario.base.len(),
            "oracle_conflicts": scenario.truth.len(),
            "arms": arms,
        }));
    }
    let path = format!("{out}/sweep-seed{}.json", base.seed);
    std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&rows).unwrap()))
        .expect("write sweep");
    println!("\n-> {path}");
}
