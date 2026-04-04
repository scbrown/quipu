# EAVT Fact Log

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
    op        INTEGER NOT NULL,  -- 1 = assert, 0 = retract
    PRIMARY KEY (e, a, v, tx)
);
```

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

This preserves type fidelity across round-trips without external schema lookups.
