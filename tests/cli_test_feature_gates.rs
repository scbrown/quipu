//! A test that spawns a binary must be gated on that binary's features.
//!
//! `CARGO_BIN_EXE_<name>` expands to a PATH, at compile time, whether or not
//! the binary was built. Every `[[bin]]` in this crate carries
//! `required-features`, so under a feature set that omits them the binary does
//! not exist and the test dies with:
//!
//! ```text
//! Os { code: 2, kind: NotFound, message: "No such file or directory" }
//! ```
//!
//! That is not an assertion failure a reader can act on, it is a panic in the
//! harness, and it turned main red once already (aegis-z29mwm): CI's
//! `Test (default)` runs `--no-default-features`, so `[[bin]] quipu`
//! (`required-features = ["shacl"]`) is not built.
//!
//! # Why a check and not care
//!
//! It does not reproduce for the author. `CARGO_BIN_EXE_*` resolves to a path,
//! so a developer whose target directory still holds a binary from an earlier
//! `--features shacl` run gets that LEFTOVER binary and the test passes
//! locally under `--no-default-features`. The bug is invisible to exactly the
//! person who introduced it, and it was introduced twice in one session by an
//! author who had already diagnosed the first instance. The convention was
//! present in the repository the whole time.
//!
//! This test needs no feature of its own: it reads files.

use std::collections::BTreeMap;

/// `name -> required-features`, parsed from the `[[bin]]` sections.
///
/// Deliberately a small hand parser rather than a toml dependency for a test:
/// the shape it reads is three fixed keys, and a parse that silently found
/// nothing would make this check vacuous, so it asserts it found some.
fn binaries_with_required_features() -> BTreeMap<String, Vec<String>> {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("Cargo.toml is readable");
    let mut found = BTreeMap::new();
    let mut name: Option<String> = None;
    let mut in_bin = false;

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_bin = trimmed == "[[bin]]";
            name = None;
            continue;
        }
        if !in_bin {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name") {
            name = value
                .trim_start_matches(['=', ' '])
                .trim()
                .trim_matches('"')
                .to_string()
                .into();
        }
        if let Some(value) = trimmed.strip_prefix("required-features") {
            let features: Vec<String> = value
                .trim_start_matches(['=', ' '])
                .trim()
                .trim_matches(['[', ']'])
                .split(',')
                .map(|f| f.trim().trim_matches('"').to_string())
                .filter(|f| !f.is_empty())
                .collect();
            if let Some(bin) = name.clone() {
                found.insert(bin, features);
            }
        }
    }
    found
}

#[test]
fn every_test_that_spawns_a_binary_is_gated_on_that_binary_s_features() {
    let binaries = binaries_with_required_features();
    assert!(
        binaries.len() >= 3,
        "parsed {} [[bin]] entries with required-features; the manifest parser has \
         stopped working and this check would pass vacuously",
        binaries.len()
    );

    let tests_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let mut checked = 0usize;
    let mut problems: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(tests_dir).expect("tests/ is readable") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("test source is readable");
        let file = path.file_name().unwrap().to_string_lossy().to_string();

        for (bin, features) in &binaries {
            if !source.contains(&format!("CARGO_BIN_EXE_{bin}")) {
                continue;
            }
            checked += 1;
            // The gate may be `#![cfg(feature = "x")]` or
            // `#![cfg(all(feature = "x", feature = "y"))]`; both are in use, so
            // require only that every needed feature is named in some cfg.
            for feature in features {
                let needle = format!("feature = \"{feature}\"");
                if !source.contains(&needle) {
                    problems.push(format!(
                        "  {file} spawns CARGO_BIN_EXE_{bin}, which is \
                         required-features = {features:?}, but never names {needle}.\n    \
                         Add `#![cfg(feature = \"{feature}\")]` (or an `all(..)` covering it) \
                         at the top of the file, as tests/pack_cli.rs does."
                    ));
                }
            }
        }
    }

    assert!(
        checked > 0,
        "no test file referenced any CARGO_BIN_EXE_<bin>; this check found nothing to \
         check and would pass however broken the convention got"
    );
    assert!(
        problems.is_empty(),
        "a test spawns a binary it may not have built:\n{}",
        problems.join("\n")
    );
}
