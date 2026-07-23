# EAVT Fact Log

> **Implementation status (2026-07-23, kelly):** ✅ **Implemented** (the schema + value-tag tables were corrected in this commit). Core is real and shipped: `facts`/`terms`/`transactions`
> with the `idx_eavt`/`idx_aevt`/`idx_vaet`/`idx_tx` indexes, bitemporal
> `valid_from`/`valid_to`, current-state `op=1 AND valid_to IS NULL`, and the term
> dictionary (`src/schema.rs`, `src/store/mod.rs`). **Drift, three items:** the real
> `facts` table also has a **`g` (graph) column + `idx_geav`** (named-graph support),
> absent from the doc's `CREATE TABLE`; the `op` discriminant also has **2 = Tombstone**
> (doc shows only 1/0); and the value-encoding table omits **tag 6 = Lang** and **tag 7
> = Typed** (`src/types.rs`) — the schema + value-tag tables below are now corrected to match.

The core of Quipu is an immutable, bitemporal fact log stored in SQLite.
Every fact is an append-only entry that is never deleted, only superseded.

## Schema

```sql
CREATE TABLE facts (
    e         INTEGER NOT NULL,  -- entity (dictionary-encoded IRI)
    a         INTEGER NOT NULL,  -- attribute (dictionary-encoded IRI)
    v         BLOB    NOT NULL,  -- value (tagged encoding)
    tx        INTEGER NOT NULL,  -- transaction ID
    valid_from TEXT   NOT NULL,  -- when fact became true
    valid_to   TEXT,             -- when fact stopped being true (NULL = current)
    op        INTEGER NOT NULL,  -- 1 = assert, 0 = retract, 2 = tombstone (overlay absence)
    g         INTEGER NOT NULL DEFAULT 0,  -- named graph (0 = default/root graph)
    PRIMARY KEY (e, a, v, tx)
);
```

Alongside the `idx_eavt` / `idx_aevt` / `idx_vaet` covering indexes, a
graph-scoped `idx_geav ON facts(g, e, a, v, valid_from)` supports named-graph
reads (the `g` column; graph 0 is the default/root graph — writes target it
unless `transact_to_graph` names another). `op = 2` (tombstone) marks a specific
`(e, a, v)` absent in an overlay's composed view, distinct from a retract.

### Term Dictionary

IRIs are stored once in the `terms` table and referenced by integer ID
everywhere else. This keeps the fact table compact and makes integer
comparisons fast.

```sql
CREATE TABLE terms (
    id  INTEGER PRIMARY KEY,
    iri TEXT NOT NULL UNIQUE
);
```

### Transactions

Every write is wrapped in a transaction with metadata:

```sql
CREATE TABLE transactions (
    id        INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL,
    actor     TEXT,     -- who made the change
    source    TEXT      -- provenance (episode, file, etc.)
);
```

## Index Permutations

Four indexes support the standard Datomic-style access patterns:

| Index | Use Case |
|-------|----------|
| EAVT  | "What are all facts about entity X?" |
| AEVT  | "What entities have attribute Y?" |
| VAET  | "What entities reference value Z?" (reverse lookup) |
| TX    | "What changed in transaction T?" |

## Bitemporal Model

Every fact has two time axes:

- **Transaction time** (`tx`): when the fact was recorded in the system
- **Valid time** (`valid_from`, `valid_to`): when the fact was true in the world

This enables:

- **Current state**: `WHERE op = 1 AND valid_to IS NULL`
- **Time-travel**: `WHERE tx <= ? AND valid_from <= ? AND (valid_to IS NULL OR valid_to > ?)`
- **Contradiction detection**: overlapping valid-time intervals on the same entity+attribute

## Value Encoding

Values are stored as tagged BLOBs with a single-byte type discriminant:

| Tag | Type | Encoding |
|-----|------|----------|
| 0   | Ref  | i64 term ID (little-endian) |
| 1   | Str  | UTF-8 bytes |
| 2   | Int  | i64 (little-endian) |
| 3   | Float | f64 (little-endian) |
| 4   | Bool | single byte (0/1) |
| 5   | Bytes | raw bytes |
| 6   | Lang | language tag + lexical form (`Value::Lang { lexical, lang }`) |
| 7   | Typed | datatype IRI + lexical form (`Value::Typed { lexical, datatype }`) |

This preserves type fidelity across round-trips without external schema lookups.
