use super::*;

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(std::string::ToString::to_string).collect()
}

#[test]
fn test_flag_value_found_and_missing() {
    let a = args(&["quipu", "knot", "f.ttl", "--shapes", "s.ttl", "--db", "x"]);
    assert_eq!(flag_value(&a, "--shapes"), Some("s.ttl"));
    assert_eq!(flag_value(&a, "--db"), Some("x"));
    assert_eq!(flag_value(&a, "--timestamp"), None);
}

#[test]
fn test_flag_value_trailing_flag_has_no_value() {
    // A flag in the final position has no following value.
    let a = args(&["quipu", "retract", "iri", "--predicate"]);
    assert_eq!(flag_value(&a, "--predicate"), None);
}

#[test]
fn test_resolve_timestamp_defaults_to_now() {
    // Absent --timestamp falls back to a generated ISO instant.
    let a = args(&["quipu", "knot", "f.ttl"]);
    assert!(looks_like_iso8601(&resolve_timestamp(&a)));
}

#[test]
fn test_resolve_timestamp_passes_through_supplied() {
    let a = args(&[
        "quipu",
        "knot",
        "f.ttl",
        "--timestamp",
        "2026-07-13T12:00:00Z",
    ]);
    assert_eq!(resolve_timestamp(&a), "2026-07-13T12:00:00Z");
}

#[test]
fn test_looks_like_iso8601() {
    assert!(looks_like_iso8601("2026-07-13"));
    assert!(looks_like_iso8601("2026-07-13T12:00:00Z"));
    assert!(!looks_like_iso8601("2026/07/13")); // wrong separators
    assert!(!looks_like_iso8601("13-07-2026")); // digits where dashes expected
    assert!(!looks_like_iso8601("2026-07")); // too short
    assert!(!looks_like_iso8601("not-a-date"));
}
