//! Producer keys for the write paths that had no caller-supplied source.
//!
//! ## Why this module exists (aegis-byn4fn)
//!
//! Every fact in this store is retractable only through the source its
//! transaction carries: `plan_source_retraction` matches `WHERE t.source = ?1`.
//! So a transaction's source is not metadata — it is the *handle* by which its
//! facts can ever be corrected or removed, and it is the only record of who
//! wrote them.
//!
//! Two defects, both measured on the live log at 97,957 transactions:
//!
//! | path | source it wrote | count | problem |
//! |---|---|---|---|
//! | `/knot` with no `source` field | **NULL** | 1,008 | `NULL = ?1` is never true in SQL, so these facts can never be retracted by anything, ever |
//! | `/set` | `"set"` | 26,008 | ONE constant key for every correction the fleet has ever made |
//! | `/retract` | `"retract"` | 36,409 | same |
//!
//! The constant keys are the larger hazard and were the surprising half. They
//! make a correction attributable to nobody, and they make the retraction handle
//! so coarse that naming it means "every `/set` ever run". That was harmless
//! only while nothing could name it — until `/retract/source` (aegis-rz75m6)
//! made an arbitrary source addressable, at which point `source: "set"` became a
//! five-figure blast radius guarded by nothing but the `expect` handshake.
//!
//! ## What this does NOT change
//!
//! The existing 62,417 transactions keep the keys they were written with; this
//! is additive. Nothing about the caller's request changes, so no agent has to
//! do anything differently — the tap is server-side, which is the point: a rule
//! that asks every writer to remember a field is the rule this store already
//! failed to enforce 1,008 times.

/// Producer key for a write that supplied no source of its own.
///
/// `<endpoint>:<actor>` — the same shape as the keys already in use
/// (`episode:<name>`, `snapshot:<key>`, `repair:<ticket>`), so a reader of the
/// transaction log sees one convention rather than four. An actorless write
/// gets `<endpoint>:anonymous` rather than a bare endpoint name: the whole
/// defect being fixed is a key that collapses every caller into one bucket, and
/// silently re-creating that bucket for the actorless case would leave the hole
/// open in exactly the position nobody looks at.
///
/// The result is deliberately NOT in the `snapshot:` namespace. A snapshot key
/// authorises absence-retraction of a producer's whole inventory; these writes
/// are single statements and must not be mistaken for one.
pub fn derive(endpoint: &str, actor: Option<&str>) -> String {
    let who = actor.map(str::trim).filter(|a| !a.is_empty());
    format!("{endpoint}:{}", who.unwrap_or("anonymous"))
}

/// The producer key a write should carry: the caller's, or [`derive`]'s.
///
/// One helper rather than the same three lines at each call site — the whole
/// defect was that these paths each decided this separately and two of them
/// decided wrong.
pub fn resolve(endpoint: &str, actor: Option<&str>, source: Option<&str>) -> String {
    match source.map(str::trim).filter(|s| !s.is_empty()) {
        Some(named) => named.to_string(),
        None => derive(endpoint, actor),
    }
}

#[cfg(test)]
mod tests {
    use super::{derive, resolve};

    #[test]
    fn names_the_endpoint_and_the_actor() {
        assert_eq!(derive("set", Some("malcolm")), "set:malcolm");
        assert_eq!(derive("retract", Some("kelly")), "retract:kelly");
        assert_eq!(derive("knot", Some("wu")), "knot:wu");
    }

    #[test]
    fn an_actorless_write_is_not_folded_into_a_shared_bucket() {
        // The bare endpoint name is what the constant keys 'set'/'retract' were,
        // and reproducing it here would leave the defect standing for exactly
        // the writes nobody is watching.
        for actor in [None, Some(""), Some("   ")] {
            let tag = derive("set", actor);
            assert_eq!(tag, "set:anonymous");
            assert_ne!(tag, "set", "must not collapse back to the constant key");
        }
    }

    #[test]
    fn a_caller_supplied_key_wins_and_blank_ones_do_not() {
        assert_eq!(
            resolve("knot", Some("wu"), Some("snapshot:code:quipu")),
            "snapshot:code:quipu"
        );
        for blank in [None, Some(""), Some("  ")] {
            assert_eq!(resolve("knot", Some("wu"), blank), "knot:wu");
        }
    }

    #[test]
    fn never_forges_a_snapshot_or_episode_key() {
        // A producer key in those namespaces carries authority this write does
        // not have: `snapshot:` authorises absence-retraction of an inventory.
        for actor in [Some("snapshot:code:quipu"), Some("episode:x"), None] {
            let tag = derive("knot", actor);
            assert!(tag.starts_with("knot:"), "{tag}");
            assert!(!tag.starts_with("snapshot:"), "{tag}");
            assert!(!tag.starts_with("episode:"), "{tag}");
        }
    }
}
