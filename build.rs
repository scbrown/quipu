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
}
