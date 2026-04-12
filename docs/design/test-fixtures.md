# Test Fixtures: Seed Data for UI Development and Demos

Status: accepted
Author: owen
Bead: qp-4ky

## Context

Every UI phase and polecat needs consistent test data. Demo recordings need
visually interesting graphs. Without shared fixtures, every polecat reinvents
test data. This design creates a `test-fixtures/` directory with a seed binary
that generates a realistic, curated test database on demand.

## Deliverables

1. `test-fixtures/test-shapes.ttl` -- SHACL shapes subset from `shapes/aegis-ontology.shapes.ttl`
2. `test-fixtures/test-episodes.json` -- 7 episodes (incident, handoff, directive, observation, deployment, discovery, recovery)
3. `test-fixtures/test-embed.html` -- HTML page loading all 5 `<quipu-*>` web components
4. `src/bin/seed_fixtures.rs` -- binary that generates `test-fixtures/test-store.db`
5. Justfile recipes: `just seed`, `just serve-fixtures`

## Architecture

A dedicated Rust binary (`src/bin/seed_fixtures.rs`) uses the quipu library
API directly. This is the right approach because:

- It needs `Store::open(path)` for a file-backed DB (not in-memory)
- It needs `ingest_rdf()` with different timestamps per transaction
- It needs `store.transact()` + `store.retract_entity()` for retraction pairs
- It needs `ingest_episode()` for structured episode data
- It needs `store.load_shapes()` to embed SHACL shapes
- Other devs can regenerate fixtures with `just seed`

### Execution sequence

```text
fn main():
  1. Delete test-fixtures/test-store.db if exists
  2. Store::open("test-fixtures/test-store.db")
  3. Ingest infrastructure subgraph (Turtle, 2026-01-15)
  4. Ingest agent platform subgraph (Turtle, 2026-02-01)
  5. Ingest knowledge subgraph (Turtle, 2026-02-15)
  6. Temporal mutations:
     a. koror status online -> down (2026-03-27)
     b. koror status down -> online (2026-03-29)
     c. postgres status -> restarted (2026-03-27)
     d. traefik status -> degraded -> healthy
  7. Ingest episodes from test-fixtures/test-episodes.json
  8. Load shapes from test-fixtures/test-shapes.ttl
  9. Store layout seed metadata (Value::Int(42))
  10. Verification assertions + summary output
```

## Data Plan (~55 entities)

### Infrastructure subgraph (~30 entities)

Timestamp: 2026-01-15T10:00:00Z, actor: "seed", source: "fixture:infra"

| Type | Entities | Count |
|------|----------|-------|
| ProxmoxNode | koror (compute), kota (storage), niue (backup) | 3 |
| LXCContainer | ct-100..ct-103 on koror, ct-200..ct-203 on kota, ct-300..ct-303 on niue | 12 |
| SystemdService | traefik, grafana, prometheus, quipu-server, forgejo, postgresql, dolt, falkordb, plex, pihole, restic, minio, node-exporter, alertmanager, caddy | 15 |

koror is the dense hub with 8+ edges:
ct-100..ct-103 `runs_on` koror, node-exporter `runs_on` koror,
prometheus `monitors` koror, grafana `depends_on` prometheus,
traefik `routes_to` containers on koror, koror `managed_by` stiwi.

### Agent platform subgraph (~15 entities)

Timestamp: 2026-02-01T10:00:00Z, actor: "seed", source: "fixture:agents"

| Type | Entities | Count |
|------|----------|-------|
| Rig | aegis, quipu, bobbin | 3 |
| CrewMember | malcolm, goldblum, ellie, owen, alan, muldoon | 6 |
| CLI | gt, bd, bobbin, quipu, marshal | 5 |
| Overseer | stiwi | 1 |

### Knowledge subgraph (~10 entities)

Timestamp: 2026-02-15T10:00:00Z, actor: "seed", source: "fixture:knowledge"

| Type | Entities | Count |
|------|----------|-------|
| Directive | hla-001..hla-003, ops-001, ops-002 | 5 |
| DesignDoc | design-quipu-ui, design-reasoner, design-federation | 3 |
| Probe | probe-koror-downtime, probe-dolt-performance | 2 |

### Temporal mutations

- koror: online (01-15) -> down (03-27T09:00) -> online (03-29T14:00)
- postgresql: running (01-15) -> crashed (03-27T09:15) -> running (03-27T11:00)
- traefik: healthy (01-15) -> degraded (03-27T09:05) -> healthy (03-29T14:30)

Uses `retract_entity(id, Some(status_attr), timestamp, actor)` then
`transact()` with new status value at same timestamp.

### Episodes (7 total)

From `test-fixtures/test-episodes.json`, timestamps spanning 2026-01 to 2026-04:

| Episode | Source Type | Group |
|---------|------------|-------|
| infra-discovery-2026-01 | crew/mayor | infrastructure |
| koror-incident-2026-03-27 | monitoring/alertmanager | incidents |
| koror-recovery-2026-03-29 | crew/goldblum | incidents |
| directive-broadcast-2026-02 | hla/stiwi | directives |
| agent-observation-dolt | crew/muldoon | observations |
| handoff-owen-to-ellie | crew/owen | handoffs |
| new-service-deployed | crew/goldblum | deployments |

## Key APIs

```rust
Store::open(path)                                          // file-backed store
ingest_rdf(store, reader, RdfFormat::Turtle, None, ts, actor, source)
store.intern(iri) / store.lookup(iri)                      // IRI <-> term ID
store.transact(datums, timestamp, actor, source)           // low-level write
store.retract_entity(entity_id, Some(attr_id), ts, actor)  // temporal retract
ingest_episode(store, &episode, timestamp, base_ns)        // episode ingestion
store.load_shapes(name, turtle, timestamp)                 // embed shapes
```

## Files

| File | Action |
|------|--------|
| `Cargo.toml` | Add `[[bin]]` for seed-fixtures |
| `justfile` | Add seed + serve-fixtures recipes |
| `.gitignore` | Add test-fixtures/test-store.db |
| `src/bin/seed_fixtures.rs` | New -- seed binary |
| `test-fixtures/test-shapes.ttl` | New -- SHACL subset |
| `test-fixtures/test-episodes.json` | New -- episode data |
| `test-fixtures/test-embed.html` | New -- embed test page |

## Verification

The seed binary prints a summary and asserts:

- 50+ entities created
- koror has 8+ edges (dense hub)
- koror status history has 5 entries (3 asserts + 2 retracts)
- 7+ episode transactions
- Shapes loaded successfully
- `quipu-server --db test-fixtures/test-store.db` starts and serves all endpoints
