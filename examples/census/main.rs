//! Census — the paper's single lifecycle benchmark.
//!
//! One scripted, seeded, multi-writer lifecycle over a governed store; one
//! run emits a ground-truth manifest and one metrics file per research
//! question. No LLM anywhere in the loop: the writers are deterministic
//! drivers, so the whole run is its own oracle. Scenario and defect
//! catalogue: `docs/design/paper-principles.md` §4; plan:
//! `docs/design/paper.md`.
//!
//! Phases 1–4 execute (beads quipu-zg0, quipu-y41): founding, recording
//! with the defect probes, correction with escalation, and composition.
//! Phases 5–6 stay `planned` in the manifest for quipu-krv / quipu-tj0 /
//! quipu-4mi.
//!
//! ```bash
//! just bench census                       # gated arm, seed 42
//! just bench census --arm control        # same script, gates off
//! ```

mod catalogue;
mod manifest;
mod phase2;
mod phase3;
mod phase4;
mod phases;
mod rng;
mod score;

use manifest::{Arm, Manifest, RunInfo};
use phases::Ctx;

fn usage() -> ! {
    eprintln!("usage: census [--seed <u64>] [--arm gated|control] [--out <dir>]");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut seed: u64 = 42;
    let mut arm = Arm::Gated;
    let mut out = String::from("benchmark/census/out");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                i += 1;
                seed = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--arm" => {
                i += 1;
                arm = match args.get(i).map(String::as_str) {
                    Some("gated") => Arm::Gated,
                    Some("control") => Arm::Control,
                    _ => usage(),
                };
            }
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or_else(|| usage());
            }
            _ => usage(),
        }
        i += 1;
    }

    std::fs::create_dir_all(&out).expect("create output directory");
    let db_path = format!("{out}/census-{}.db", arm.as_str());
    // A fresh store per run: determinism starts at byte zero.
    let _ = std::fs::remove_file(&db_path);
    let store = quipu::Store::open(&db_path).expect("open census store");

    let mut ctx = Ctx::new(store, rng::SplitMix64::new(seed), arm, db_path);
    phases::run_all(&mut ctx, &out);

    let manifest = Manifest {
        run: RunInfo {
            seed,
            arm: arm.as_str().to_string(),
            harness: "phases 1-4 executed (quipu-zg0, quipu-y41); 5-6 planned".to_string(),
        },
        entries: std::mem::take(&mut ctx.entries),
    };
    score::write_metrics(&ctx, &manifest, &out);
    let path = format!("{out}/manifest-{}.json", arm.as_str());
    std::fs::write(&path, manifest.to_json()).expect("write manifest");

    let executed = manifest
        .entries
        .iter()
        .filter(|e| e.status == "executed")
        .count();
    let planned = manifest.entries.len() - executed;
    println!(
        "census[{}] seed={seed}: {executed} probes executed, {planned} planned -> {path}",
        arm.as_str()
    );
}
