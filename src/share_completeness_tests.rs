use super::*;
use crate::Store;

/// Every table name a `CREATE TABLE` in the SOURCE creates, test sources aside.
///
/// Finds a table wherever it is declared — `schema.rs`, `store/migrate.rs`, or a
/// feature module — and, unlike `sqlite_master` on a fresh store, finds one that
/// is created LAZILY on a code path no fixture walks.
fn tables_created_in_source() -> std::collections::BTreeSet<String> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = std::collections::BTreeSet::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // Test sources re-declare tables in fixtures (and declare throwaways
            // like `t`), which are not store schema.
            if path.to_string_lossy().contains("test") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source");
            for line in text.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || trimmed.starts_with("--") {
                    continue;
                }
                let lower = line.to_ascii_lowercase();
                let Some(at) = lower.find("create table") else {
                    continue;
                };
                let rest = line[at + "create table".len()..].trim_start();
                let rest = rest
                    .strip_prefix("IF NOT EXISTS")
                    .or_else(|| rest.strip_prefix("if not exists"))
                    .unwrap_or(rest)
                    .trim_start();
                let table: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !table.is_empty() {
                    found.insert(table);
                }
            }
        }
    }
    found
}

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

/// The reverse direction: nothing declared that nothing creates.
///
/// A stale entry is not harmless. It makes the list look more complete than it
/// is, and a reader checking "is X handled?" gets a yes for a table that no
/// longer exists — while the table that replaced it may be undeclared.
///
/// ⚠️ "Creates" means a FRESH STORE **or** anywhere in the source, and the union
/// is load-bearing rather than belt-and-braces. Checking a fresh store alone —
/// which this test did until 2026-09-11 — makes a LAZILY created table look
/// stale, because a fresh store genuinely does not have one. Paired with the
/// forward audit, which cannot SEE a lazy table, that did not merely leave a
/// gap: the forward test never asked for `pack_loads` to be declared and this
/// one REFUSED it when it was, so the correct state was unreachable and the
/// only stable configuration was the wrong one (aegis-9f899e, measured — the
/// declaration failed exactly here).
#[test]
fn nothing_is_declared_that_nothing_creates() {
    let store = Store::open_in_memory().unwrap();
    let mut stmt = store
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap();
    let live: std::collections::BTreeSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    let in_source = tables_created_in_source();

    let stale: Vec<&str> = DECLARED
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !live.contains(*name) && !in_source.contains(*name))
        .collect();
    assert!(
        stale.is_empty(),
        "declared table(s) {stale:?} are created neither by a fresh store nor \
         anywhere in the source. Either the name is wrong or the table is gone; \
         a stale entry makes this list read as more complete than it is."
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

/// The live-schema audit above cannot see a table that is created LAZILY, and
/// this is the test that can.
///
/// `every_table_a_real_store_creates_is_classified` opens a store and asks
/// `sqlite_master`. That is the right instrument for anything schema init or a
/// migration creates — and it is structurally blind to a table created later,
/// on a code path the fixture never walks, because such a table is absent from
/// the fixture and from `DECLARED` at the same time and so compares equal.
///
/// `pack_loads` is exactly that table. `pack_load.rs` creates it in the
/// DESTINATION store when a pack is loaded, so a fresh store has 24 tables and
/// no `pack_loads` while any store that has ever loaded a pack has 25 — and it
/// went undeclared, silently, under a passing audit whose stated purpose was to
/// catch precisely this (aegis-9f899e, measured 2026-09-11).
///
/// So this scans the SOURCE for every `CREATE TABLE`, which finds a table
/// wherever it is created and regardless of whether any fixture creates it. The
/// two tests are complements and neither is redundant: the source scan cannot
/// see a table a dependency creates, and the live audit cannot see a lazy one.
#[test]
fn every_table_created_anywhere_in_the_source_is_classified() {
    // Lives in the pack ARTIFACT, not in a store — `pack.rs` writes it into the
    // .qpack.db file itself. A store round-trip must never carry it, so it is
    // correctly absent from DECLARED rather than missing from it.
    const NOT_A_STORE_TABLE: &[&str] = &["pack_manifest"];

    let found = tables_created_in_source();

    // ANTI-VACUITY: a scan that found nothing — a moved directory, a changed
    // extension — would pass the loop below while auditing nothing at all.
    assert!(
        found.len() >= 20,
        "expected to find the schema in the source, found {} table(s): {found:?}",
        found.len()
    );

    let undeclared: Vec<&String> = found
        .iter()
        .filter(|name| !NOT_A_STORE_TABLE.contains(&name.as_str()))
        .filter(|name| disposition(name).is_none())
        .collect();
    assert!(
        undeclared.is_empty(),
        "table(s) {undeclared:?} are created somewhere in the source and are \
         declared NOWHERE. A reconstruction silently drops them. If a table is \
         created lazily it will NOT appear in a fresh store, so the live-schema \
         audit cannot catch it — that is why this test exists (aegis-9f899e). \
         Classify each in `DECLARED`, with the reason beside it when Excluded."
    );
}
