// Stamp the build with its git SHA so a RUNNING server can be asked what it is
// (aegis-odnr). /health returning {"status":"ok"} is identical on every build
// ever made, so "is the fix deployed?" was unanswerable from outside — which
// cost a P0 filed against the wrong root cause. A semantic version does not
// solve it either: shantytown's stayed 0.0.1 through every install and never
// once signalled drift. The SHA is the thing that changes when the code does.
//
// Degrades honestly: if git is unavailable (source tarball, vendored build)
// this records "unknown" rather than a plausible-looking wrong value. An
// unknown SHA must never be mistaken for a known one.
use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !o.stdout.is_empty());

    println!("cargo:rustc-env=QUIPU_GIT_SHA={sha}");
    println!("cargo:rustc-env=QUIPU_GIT_DIRTY={dirty}");
    // Re-run when HEAD moves, or the stamp goes stale and lies.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    stamp_features();
}

/// Stamp EVERY declared feature and whether it is enabled, for `/version`
/// (aegis-t1u2h).
///
/// `/version` used to hardcode two `cfg!` checks — shacl and onnx — so it
/// reported nothing about `owl` or `reactive-reasoner` whether they were present
/// or absent. That made a working deploy and a completely inert one look
/// IDENTICAL from outside, which is exactly the state aegis-06q1r shipped into:
/// the deploy script built `required-features`, owl was compiled out, every gate
/// passed, and the one instrument a person would reach for could not say so.
///
/// The list is derived from `Cargo.toml` rather than written here ON PURPOSE. A
/// hand-kept list is how the original drifted, and a third hand-written `cfg!`
/// line would drift the same way the moment feature four is added. Reading the
/// manifest means a new feature appears in `/version` for free, and the failure
/// mode of the parser is a MISSING key (visibly wrong) rather than a key
/// silently reported as false.
///
/// Mechanism: cargo sets `CARGO_FEATURE_<NAME>` for each ENABLED feature, with
/// the name uppercased and `-` mapped to `_`. That transform is lossy, so the
/// canonical spelling comes from the manifest and the env var is only consulted
/// as a yes/no — which is why `reactive-reasoner` keeps its hyphen here.
fn stamp_features() {
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    let mut in_features = false;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_features = t == "[features]";
            continue;
        }
        if !in_features || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = t.split_once('=') {
            let key = key.trim();
            // `default` is cargo bookkeeping, not a capability — reporting it
            // would say nothing about what the binary can DO.
            if !key.is_empty() && key != "default" {
                names.push(key.to_string());
            }
        }
    }
    names.sort();
    names.dedup();

    let pairs: Vec<String> = names
        .iter()
        .map(|n| {
            let var = format!("CARGO_FEATURE_{}", n.to_uppercase().replace('-', "_"));
            let on = u8::from(std::env::var_os(&var).is_some());
            format!("{n}={on}")
        })
        .collect();

    // The MARKER is load-bearing, not decoration. The deploy gate finds this stamp
    // by scanning `strings` over the binary, and a bare `name=0|1,...` pattern is
    // not distinctive enough to survive that: a release binary's string table
    // contains fragments like "i=0,r=1" that match it, and the gate then read
    // garbage as the feature set and refused a perfectly good build. Anchor on
    // something that cannot occur by accident, and version it so the format can
    // change without silently misreading an older binary.
    println!(
        "cargo:rustc-env=QUIPU_FEATURES=quipu-features/1;{}",
        pairs.join(",")
    );
    println!("cargo:rerun-if-changed=Cargo.toml");
}
