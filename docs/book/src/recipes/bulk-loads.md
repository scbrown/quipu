# Bulk loads and migrations: share → import → promote

Use this path whenever you have **a lot of data at once**:

- a first load;
- a backfill;
- moving a dataset from one store to another;
- migrating a board or a catalogue.

Do not use a loop of thousands of `quipu knot` calls, `/knot` requests or
SPARQL Update writes for this.

| situation | path |
|---|---|
| live, incremental facts arriving one at a time | `quipu knot`, `/knot`, `/episode` |
| a bulk or one-time load, or a migration | **share → import → promote** (this page) |

## Why

- **Validated before it touches your graph.** `quipu import` stages the whole
  dataset in its own staging graph and checks it against your shapes there.
  Non-conforming data is quarantined, and import never writes ROOT.
- **One transaction.** `quipu import promote` admits the staged data into ROOT in
  a single transaction. Thousands of small writes give you thousands of
  transactions, each paying the write path's per-write cost, and a half-finished
  loop leaves a half-loaded graph.
- **One rollback.** A promotion is one transaction, so undoing it is one step
  (see [Rollback](#rollback)).
- **Integrity.** The share carries canonical payload hashes, and import refuses a
  share whose bytes do not match its manifest.

## The three steps

### 1. Build a share from the data

Load the data into a scratch store, with the shapes you will validate against, and
write a share from it:

```bash
quipu shapes load items shapes.ttl --db scratch.db
quipu knot data.ttl --shapes shapes.ttl --db scratch.db
quipu share --output share1 --db scratch.db --destination internal
```

`share1/` now holds `export.nt` (the canonical facts), `shapes.ttl` and the
manifests. `--destination internal` is for a migration inside your own
infrastructure. Without it, `quipu share` runs the outward identifier scrub, which
needs an [identifier-policy catalogue](../sharing/README.md#prepare-an-outward-share).

### 2. Import: stage and validate

```bash
quipu import share1 --db target.db --actor you --destination internal
```

The JSON result is the review:

```json
{
  "outcome": "staged",
  "share_id": "sha256:1a3581b0…",
  "staging_graph": "urn:quipu:import:staging:1a3581b0…",
  "triples": { "accepted": 4000, "quarantined": 0 },
  "validation": { "conforms": true, "blocking": false },
  "promotion": { "eligible": true, "blockers": [] }
}
```

- ROOT is unchanged at this point.
- A share stamped `internal` must be imported with `--destination internal` too, or
  import refuses it.
- If the data breaks your shapes, the result says so, and nothing can be promoted:

```json
{ "outcome": "quarantined",
  "triples": { "accepted": 0, "quarantined": 5 },
  "promotion": { "eligible": false, "blockers": ["shacl_nonconforming"] } }
```

### 3. Promote: admit it into ROOT

```bash
quipu import promote sha256:1a3581b0… --db target.db --actor you
```

```json
{ "outcome": "promoted", "tx_id": 3, "triples": 4000, "suppressed_retractions": 0 }
```

Read back by **count**, not by the success message:

```bash
quipu read 'SELECT (COUNT(?s) AS ?n) WHERE { ?s a <http://example.org/Item> }' --db target.db
```

Promoting the same share again re-asserts the same facts and adds no duplicates.

## Rollback

Before promoting, note the store's current transaction. To undo a promotion, fork
ROOT as of the transaction just before it, check the diff, and promote the fork:

```bash
quipu fork 2 --name pre-import --db target.db         # 2 = the tx before the promotion
quipu fork diff main pre-import --db target.db         # expect -<the promoted triples>
quipu fork promote pre-import --db target.db           # one transaction retracts them
```

**Read the diff first.** It is the safety check: the fork also undoes any write
made after that transaction. History is kept, so `quipu read … --tx 2` still shows
the pre-promotion state.

## On a running server

The same steps exist over HTTP: `POST /import` stages a share and
`POST /import/promote` admits it. See the [REST API](../reference/rest-api.md#post-import).

## Measured

These are the numbers from the example above: 2,000 entities and 4,000 triples
moved from a scratch store into a target that already held its own data.

| step | result |
|---|---|
| `import` | staged, 4,000 accepted, 0 quarantined; ROOT unchanged (1 entity) |
| `import promote` | one transaction (`tx_id` 3), 4,000 triples, 0.12 s |
| read-back | 2,001 entities (1 existing + 2,000 promoted) |
| re-promote | same 4,002 current facts, no duplicates |
| fork rollback | `retracted 4000` in one transaction; back to 1 entity |

See also: [CLI: sharing, import and legacy packs](../reference/cli-sharing.md) and
[Sharing & Federation](../sharing/README.md).
