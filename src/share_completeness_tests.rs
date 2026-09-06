use super::*;
use crate::Store;

/// Every table a real store creates must be classified.
///
/// THIS IS THE CHECK A ROUND-TRIP CANNOT DO. A round-trip proves that what you
/// carried came back; it is silent about what you never carried, because the
/// thing you never carried is absent from both sides and compares equal. So the
/// completeness of the DECLARED SET needs its own assertion, and it has to ask
/// the live schema rather than any source file: tables are created in both
/// `schema.rs` and `store/migrate.rs`, so neither file is the whole list, and a
/// list derived from one of them would be wrong in the reassuring direction.
#[test]
fn every_table_a_real_store_creates_is_classified() {
    let store = Store::open_in_memory().unwrap();
    let mut stmt = store
        .prepare(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    let live: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();

    // ANTI-VACUITY: an empty or tiny listing would pass the loop below while
    // proving nothing, and "the query returned no rows" is exactly how this
    // audit would silently stop auditing.
    assert!(
        live.len() >= 15,
        "expected a real schema, got {} table(s): {live:?}",
        live.len()
    );

    let undeclared: Vec<&String> = live
        .iter()
        .filter(|name| disposition(name).is_none())
        .collect();
    assert!(
        undeclared.is_empty(),
        "table(s) {undeclared:?} exist in a live store and are declared NOWHERE. \
         A reconstruction silently drops them. Classify each in `DECLARED` — and \
         if the answer is Excluded, write the reason beside it, because for that \
         group the list is a security boundary and not a convenience \
         (docs/design/standard-share-artifact.md, aegis-9f899e)."
    );
}

/// The reverse direction: nothing declared that no store creates.
///
/// A stale entry is not harmless. It makes the list look more complete than it
/// is, and a reader checking "is X handled?" gets a yes for a table that no
/// longer exists — while the table that replaced it may be undeclared.
#[test]
fn nothing_is_declared_that_a_real_store_does_not_create() {
    let store = Store::open_in_memory().unwrap();
    let mut stmt = store
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap();
    let live: std::collections::BTreeSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();

    let stale: Vec<&str> = DECLARED
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !live.contains(*name))
        .collect();
    assert!(
        stale.is_empty(),
        "declared table(s) {stale:?} are not created by a real store. Either the \
         name is wrong or the table is gone; a stale entry makes this list read \
         as more complete than it is."
    );
}

/// The security boundary, asserted by name rather than left to the list's order.
///
/// The two attestation tables are the reason the declared set exists at all
/// (see `DECLARED`). Stated as its own test so that flipping either one to a
/// carried disposition fails with the argument attached, rather than merely
/// changing a row in a table nobody re-reads.
#[test]
fn the_attestation_tables_are_never_carried() {
    for table in ["attestation_bindings", "attestation_nonces"] {
        assert_eq!(
            disposition(table),
            Some(Disposition::Excluded),
            "{table} must never be carried into a reconstruction. \
             `attestation_bindings` is the registry of which producer sessions \
             this store was told to trust, and restoring it grants trust the \
             consumer never granted — which is aegis-tadzdf's \"quipu never \
             self-registers\" defeated through a different door. \
             `attestation_nonces` is replay state and is wrong carried OR \
             silently dropped."
        );
        assert!(
            !carried().contains(&table),
            "{table} reached the carried set"
        );
    }
}

/// `carried()` is Content plus Log, and excludes the rest.
#[test]
fn carried_is_content_and_log_only() {
    let carried = carried();
    assert!(carried.contains(&"facts"), "content must be carried");
    assert!(carried.contains(&"events"), "the log must be carried");
    assert!(
        !carried.contains(&"vectors"),
        "vectors are regenerated, never serialized"
    );
    assert!(
        !carried.contains(&"consumers"),
        "a reader's cursor is not content"
    );
    // Every carried name is declared as one of exactly those two dispositions.
    for name in &carried {
        assert!(
            matches!(
                disposition(name),
                Some(Disposition::Content | Disposition::Log)
            ),
            "{name} is carried but is neither Content nor Log"
        );
    }
}
