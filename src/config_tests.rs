use std::path::Path;

use super::*;
// `ConfigFile` moved to `config_load` in the size-ratchet split; the loader
// round-trip tests below still deserialize it directly.
use crate::config_load::ConfigFile;

/// Top-level `pub` fields of `QuipuConfig` that this repo deliberately does NOT
/// consume — documented capabilities that are unwired here. Each
/// entry is a promise: it is loud at runtime via `unwired_warnings()` and its
/// docs say "unimplemented". When one is wired, remove it here AND from
/// `unwired_warnings`, and the guard below will hold you to consuming it.
/// quipu #47 emptied this: `federation` was the wholly-dead sub-config the
/// guard was written for, and `provider::federated_from_config` now consumes
/// it. An EMPTY allowlist is the healthy state — every documented knob is
/// wired. Re-adding an entry means accepting a settable-but-inert switch, so
/// it needs a `unwired_warnings()` branch and an "unimplemented" doc note in
/// the same change.
const UNWIRED_TOP_LEVEL: &[&str] = &[];

/// Concatenated source of every `src/**/*.rs` EXCEPT config.rs, so the guard
/// can ask "is this field read anywhere but its own definition?".
fn src_without_config() -> String {
    fn walk(dir: &std::path::Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs")
                && p.file_name().is_some_and(|n| n != "config.rs")
                && let Ok(s) = std::fs::read_to_string(&p)
            {
                out.push_str(&s);
                out.push('\n');
            }
        }
    }
    let mut out = String::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut out,
    );
    out
}

fn quipu_config_top_level_fields() -> Vec<String> {
    // Pull `pub NAME:` out of the `pub struct QuipuConfig { ... }` block only.
    let src = include_str!("config.rs");
    let start = src
        .find("pub struct QuipuConfig {")
        .expect("QuipuConfig struct");
    let body = &src[start..];
    let end = body.find("\n}").expect("end of QuipuConfig");
    body[..end]
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .filter_map(|l| l.split(':').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .collect()
}

/// `src` with `//`-comments stripped, so PROSE cannot satisfy a usage check.
///
/// Third false-negative class found for this guard (quipu #47), and the
/// worst of the three: a mention in a COMMENT counted as a use. Writing
/// `[[quipu.federation.remotes]]` in a doc string marked `federation` as
/// consumed — so a comment explaining that a field is DEAD CONFIG would
/// itself certify it as live.
///
/// Truncating at `//` can also cut a line at a URL inside a string literal.
/// That direction is safe: it can only cause a spurious FAILURE (the guard
/// complains about a field that is used), never a spurious pass.
fn strip_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `.field` is read in `src` as a WHOLE field access.
///
/// A bare `src.contains(".labels")` also matches `.labels_config`, so a
/// field could be reported as wired by an identifier that merely starts
/// with its name. Measured 2026-08-06 (quipu #68): with the sole consumer
/// of `config.labels` deleted, the guard still PASSED, because
/// `store.labels_config()` contains `.labels`. The guard's comment
/// anticipates false matches for generic leaf names and assumes top-level
/// names are distinctive — that assumption breaks the moment a field shares
/// a prefix with any other identifier in the tree, which is easy to do
/// accidentally and impossible to notice, since the failure is a PASS.
///
/// So require the next character not to continue an identifier.
fn field_is_read(src: &str, field: &str) -> bool {
    let needle = format!(".{field}");
    src.match_indices(&needle).any(|(i, _)| {
        src[i + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

#[test]
fn the_wiring_guard_does_not_match_a_longer_identifier() {
    // The guard guarding the guard. Without this, a prefix collision makes
    // `config_knobs_are_wired_or_listed_unwired` silently vacuous for the
    // colliding field — and it fails in the direction that reassures.
    assert!(
        !field_is_read("store.labels_config_mut().foo();", "labels"),
        "`.labels_config` must NOT count as a read of `.labels`"
    );
    assert!(field_is_read("cfg.labels.min_freshness", "labels"));
    assert!(field_is_read("clone_from(&config.labels);", "labels"));
    assert!(field_is_read("let x = c.labels;", "labels"));
}

#[test]
fn config_knobs_are_wired_or_listed_unwired() {
    // THE CLASS GUARD. A documented, settable config field that
    // nothing reads is the failure mode this fleet keeps paying for — accepted
    // and inert. This asserts every top-level QuipuConfig field is either
    // consumed somewhere outside config.rs, or is explicitly on UNWIRED_TOP_LEVEL
    // (which forces a runtime warning + an "unimplemented" doc note). Scoped to
    // top-level fields because their names are distinctive; a generic leaf name
    // like `name`/`url` cannot be grepped without false matches. federation was
    // the wholly-dead sub-config this was written for.
    let src = strip_comments(&src_without_config());
    let mut dead = Vec::new();
    for field in quipu_config_top_level_fields() {
        let consumed = field_is_read(&src, &field);
        let listed = UNWIRED_TOP_LEVEL.contains(&field.as_str());
        if !consumed && !listed {
            dead.push(field);
        }
    }
    assert!(
        dead.is_empty(),
        "these QuipuConfig fields are settable+documented but read by NOTHING outside \
         config.rs: {dead:?}. Wire each one, or add it to UNWIRED_TOP_LEVEL, give it a \
         warning in unwired_warnings(), and mark it unimplemented in the docs — a knob \
         that is accepted and does nothing is the silently-inert-config bug."
    );
    // And the allowlist must not rot: every entry must still be a real field
    // AND still genuinely unconsumed, or it should have been removed.
    for u in UNWIRED_TOP_LEVEL {
        assert!(
            quipu_config_top_level_fields().iter().any(|f| f == u),
            "UNWIRED_TOP_LEVEL lists {u:?} which is not a QuipuConfig field — remove it"
        );
        assert!(
            !field_is_read(&src, u),
            "UNWIRED_TOP_LEVEL still lists {u:?} but it is now consumed outside config.rs — \
             it is wired; remove it from the allowlist and unwired_warnings()"
        );
    }
}

#[test]
fn unwired_knobs_warn_loudly_when_set() {
    // The set-but-inert knobs must produce a warning, so they are never
    // silent. Wiring one means updating this — federation did exactly that
    // in quipu #47, and the assertion below flipped rather than vanished.
    let mut cfg = QuipuConfig::default();
    assert!(cfg.unwired_warnings().is_empty(), "defaults must not warn");

    cfg.vector.backend = VectorBackend::Lancedb;
    assert!(
        cfg.unwired_warnings()
            .iter()
            .any(|w| w.contains("vector.backend")),
        "vector.backend = lancedb must warn — the quipu binaries do not honour it"
    );

    // quipu #47 WIRED federation, so this flipped from "must warn" to "must
    // NOT warn". Kept rather than deleted: a warning that outlives its
    // subject is worse than no warning — it trains readers to ignore the
    // channel, and the next genuinely-inert knob is the one they miss.
    cfg.federation
        .remotes
        .push(RemoteEndpoint::new("prod", "http://x:1"));
    assert!(
        !cfg.unwired_warnings()
            .iter()
            .any(|w| w.contains("federation")),
        "federation is implemented (quipu #47) — it must NOT warn as unwired"
    );
}

#[test]
fn search_clamp_limit() {
    let cfg = SearchConfig::default(); // default_limit=10, max_limit=1000
    // Absent → default.
    assert_eq!(cfg.clamp_limit(None), 10);
    // In range → unchanged.
    assert_eq!(cfg.clamp_limit(Some(50)), 50);
    // Over the ceiling → clamped (the 1_000_000 attack).
    assert_eq!(cfg.clamp_limit(Some(1_000_000)), 1000);
    // Zero → never returns an empty page.
    assert_eq!(cfg.clamp_limit(Some(0)), 1);
}

#[test]
fn search_oversample_uses_unified_factor() {
    let cfg = SearchConfig::default(); // factor = DEFAULT_OVERSAMPLE_FACTOR (10)
    assert_eq!(cfg.oversample(10), 10 * DEFAULT_OVERSAMPLE_FACTOR);
    // Saturating + never below the input.
    assert_eq!(cfg.oversample(usize::MAX), usize::MAX);
}

#[test]
fn search_defaults() {
    let cfg = SearchConfig::default();
    assert_eq!(cfg.default_limit, 10);
    assert_eq!(cfg.max_limit, 1000);
    assert_eq!(cfg.oversample_factor, DEFAULT_OVERSAMPLE_FACTOR);
    assert_eq!(cfg.max_sparql_rows, 10_000);
}

#[test]
fn defaults() {
    let cfg = QuipuConfig::default();
    assert_eq!(cfg.store_path, PathBuf::from(".bobbin/quipu/quipu.db"));
    assert_eq!(cfg.server.bind, "127.0.0.1:3030");
    assert!(!cfg.server.enabled);
    assert!(cfg.federation.remotes.is_empty());
    assert!(!cfg.embedding.auto_embed);
    assert_eq!(cfg.embedding.embed_batch_size, 32);
    assert_eq!(cfg.vector.backend, VectorBackend::Sqlite);
    assert_eq!(
        cfg.vector.lancedb_path,
        PathBuf::from(".bobbin/quipu/quipu-vectors")
    );
}

#[test]
fn parse_toml() {
    let toml_str = r#"
[quipu]
store_path = "/data/quipu.db"

[quipu.server]
enabled = true
bind = "0.0.0.0:8080"

[[quipu.federation.remotes]]
name = "prod"
url = "http://quipu.example:3030"

[quipu.embedding]
auto_embed = true
embed_batch_size = 64

[quipu.search]
query_timeout_ms = 5000
"#;
    let file: ConfigFile = toml::from_str(toml_str).unwrap();
    let cfg = file.quipu;
    assert_eq!(cfg.search.query_timeout_ms, 5000);
    assert_eq!(cfg.store_path, PathBuf::from("/data/quipu.db"));
    assert!(cfg.server.enabled);
    assert_eq!(cfg.server.bind, "0.0.0.0:8080");
    assert_eq!(cfg.federation.remotes.len(), 1);
    assert_eq!(cfg.federation.remotes[0].name, "prod");
    assert_eq!(cfg.federation.remotes[0].url, "http://quipu.example:3030");
    assert!(cfg.embedding.auto_embed);
    assert_eq!(cfg.embedding.embed_batch_size, 64);
}

#[test]
fn parse_vector_config() {
    let toml_str = r#"
[quipu]
store_path = "/data/quipu.db"

[quipu.vector]
backend = "lancedb"
lancedb_path = "/data/vectors"
"#;
    let file: ConfigFile = toml::from_str(toml_str).unwrap();
    let cfg = file.quipu;
    assert_eq!(cfg.vector.backend, VectorBackend::Lancedb);
    assert_eq!(cfg.vector.lancedb_path, PathBuf::from("/data/vectors"));
}

#[test]
fn parse_vector_config_lance_alias() {
    let toml_str = r#"
[quipu.vector]
backend = "lance"
"#;
    let file: ConfigFile = toml::from_str(toml_str).unwrap();
    assert_eq!(file.quipu.vector.backend, VectorBackend::Lancedb);
}

#[test]
fn cli_overrides() {
    let cfg = QuipuConfig::default()
        .with_db_override(Some("/custom/path.db"))
        .with_bind_override(Some("0.0.0.0:9090"));
    assert_eq!(cfg.store_path, PathBuf::from("/custom/path.db"));
    assert_eq!(cfg.server.bind, "0.0.0.0:9090");
}

#[test]
fn partial_toml() {
    let toml_str = r#"
[quipu]
store_path = "/data/quipu.db"
"#;
    let file: ConfigFile = toml::from_str(toml_str).unwrap();
    let cfg = file.quipu;
    assert_eq!(cfg.store_path, PathBuf::from("/data/quipu.db"));
    // Server and federation should have defaults.
    assert_eq!(cfg.server.bind, "127.0.0.1:3030");
    assert!(cfg.federation.remotes.is_empty());
}

#[test]
fn empty_file_gives_defaults() {
    let toml_str = "";
    let file: ConfigFile = toml::from_str(toml_str).unwrap();
    let cfg = file.quipu;
    assert_eq!(cfg.store_path, PathBuf::from(".bobbin/quipu/quipu.db"));
}

#[test]
fn load_nonexistent_dir() {
    let cfg = QuipuConfig::load(Path::new("/nonexistent/dir"));
    assert_eq!(cfg.store_path, PathBuf::from(".bobbin/quipu/quipu.db"));
}

#[test]
fn parse_attachments_table() {
    // quipu-at2: the declaration shape, and that an absent table stays empty
    // rather than defaulting to something mounted.
    let toml_str = r#"
[quipu]
store_path = "/data/quipu.db"

[[quipu.attachments]]
alias = "reference"
path = "/data/reference.qpack.db"

[[quipu.attachments]]
alias = "tenant_a"
path = "packs/tenant-a.db"
"#;
    let file: ConfigFile = toml::from_str(toml_str).unwrap();
    let cfg = file.quipu;
    assert_eq!(cfg.attachments.len(), 2);
    assert_eq!(cfg.attachments[0].alias, "reference");
    assert_eq!(
        cfg.attachments[0].path,
        PathBuf::from("/data/reference.qpack.db")
    );
    assert_eq!(cfg.attachments[1].alias, "tenant_a");
    assert_eq!(cfg.attachments[1].path, PathBuf::from("packs/tenant-a.db"));

    let empty: ConfigFile = toml::from_str("[quipu]\n").unwrap();
    assert!(
        empty.quipu.attachments.is_empty(),
        "silence must mount nothing — the pre-existing behaviour exactly"
    );
}
