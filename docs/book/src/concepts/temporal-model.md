# The Temporal Model

Every fact in Quipu has two time dimensions:

1. **Transaction time** — when the fact was recorded in the database
2. **Valid time** — when the fact was true in the real world

This is called a **bitemporal model**, and it means you can always answer:

- "What did we know at time T?" (transaction time)
- "What was true at time T?" (valid time)
- "What did we know at time T1 about what was true at T2?" (both)

## Why Bitemporality Matters

Say you record on April 1 that koror has 4 CPU cores:

```bash
quipu knot - --db homelab.db --timestamp 2026-04-01 <<'EOF'
@prefix hw: <http://example.org/homelab/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
hw:koror hw:cpuCores "4"^^xsd:integer .
EOF
```

On April 3 you upgrade to 8 cores and record the change:

```bash
quipu retract "http://example.org/homelab/koror" \
  --predicate "http://example.org/homelab/cpuCores" \
  --db homelab.db --timestamp 2026-04-03

quipu knot - --db homelab.db --timestamp 2026-04-03 <<'EOF'
@prefix hw: <http://example.org/homelab/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
hw:koror hw:cpuCores "8"^^xsd:integer .
EOF
```

Now you can time-travel:

```bash
# What's the current state?
quipu read "SELECT ?cores WHERE {
  <http://example.org/homelab/koror> <http://example.org/homelab/cpuCores> ?cores
}" --db homelab.db
# → 8

# What was true on April 2?
quipu read "SELECT ?cores WHERE {
  <http://example.org/homelab/koror> <http://example.org/homelab/cpuCores> ?cores
}" --db homelab.db --valid-at 2026-04-02
# → 4
```

## The EAVT Fact Log

Under the hood, facts are stored as immutable rows:

| E (entity) | A (attribute) | V (value) | T (tx) | valid_from | valid_to | op |
|------------|---------------|-----------|--------|------------|----------|-----|
| `hw:koror` | `hw:cpuCores` | 4 | 1 | 2026-04-01 | 2026-04-03 | Assert |
| `hw:koror` | `hw:cpuCores` | 4 | 2 | 2026-04-03 | 2026-04-03 | Retract |
| `hw:koror` | `hw:cpuCores` | 8 | 3 | 2026-04-03 | *null* | Assert |

Nothing is deleted. Retractions close the `valid_to` window on old facts
and add a new retraction record. The full history is always available.

## Transaction Time vs Valid Time

| Dimension | What it tracks | Set by | Queryable via |
|-----------|---------------|--------|---------------|
| **Transaction time** | When the database learned about the fact | System (auto-incremented tx ID) | `--tx` flag, `as_of_tx` parameter |
| **Valid time** | When the fact was true in reality | You (`--timestamp` flag) | `--valid-at` flag, `valid_at` parameter |

Transaction time is monotonic and system-controlled. Valid time is
user-supplied and can refer to the past or future.

## Querying Through Time

### Current state (default)

```sparql
SELECT ?host ?cores WHERE {
  ?host <http://example.org/homelab/cpuCores> ?cores .
}
```

Returns only currently-asserted facts (op=Assert, valid_to is null).

### Valid-time travel

```bash
quipu read "SELECT ?host ?cores WHERE {
  ?host <http://example.org/homelab/cpuCores> ?cores
}" --db homelab.db --valid-at 2026-04-02
```

Returns facts that were valid at the specified point in time.

### Transaction-time travel

```bash
quipu read "SELECT ?host ?cores WHERE {
  ?host <http://example.org/homelab/cpuCores> ?cores
}" --db homelab.db --tx 1
```

Returns only facts recorded up to transaction 1 — what the database knew
at that point, regardless of valid-time windows.

### REST API

```bash
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT ?host ?cores WHERE { ?host <http://example.org/homelab/cpuCores> ?cores }",
    "valid_at": "2026-04-02"
  }'
```

## Contradiction Detection

If two facts for the same entity+attribute have overlapping valid-time windows,
Quipu flags a contradiction. This prevents conflicting states from silently
coexisting:

```rust
let issues = store.detect_contradictions()?;
// Returns pairs of facts with overlapping intervals
```

## Design Principles

- **Append-only**: Facts are never mutated or deleted
- **Full audit trail**: Every change is a transaction with metadata
- **Time-travel by default**: Any query can add a temporal context
- **Contradiction-aware**: Overlapping valid-time windows are surfaced
