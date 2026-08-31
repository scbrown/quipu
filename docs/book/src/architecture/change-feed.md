# Change Feed

> **Implementation status (2026-08-31):** ✅ **Implemented** (quipu-2ae, from
> the Spanner investigation in `docs/design/spanner-capabilities.md` §4.4).
> `src/store/changes.rs` — `changes_after` with the three capture modes —
> served by `quipu changes` and `GET /changes`.

The append-only fact log has been a change stream all along; the change feed
gives it a **consumer contract**, modeled on Spanner's change streams but
adapted to a pull surface. It is fact-level and lossless where the
[event log](../concepts/governance.md) (`/events`) is semantic and
taxonomized: use `/events` to hear "an entity changed", use `/changes` to
mirror exactly which facts did.

## The contract

- **Records** are `(tx, sequence, op, graph, entity, attribute, value)`,
  derived directly from fact rows — never a second log that can drift from
  the first. `op` is `assert`, `retract`, or `tombstone`.
- **The cursor is a transaction id, and pages end on transaction
  boundaries.** Every cursor is therefore a consistent prefix of commit
  history: a reader never observes half a transaction.
- **Ordering**: per entity, records arrive in commit order (transaction,
  then write order within it). Across entities there is no ordering promise.
- **Watermark instead of heartbeat**: every page carries `watermark_tx` and
  `watermark_timestamp` — the newest committed transaction. An empty page
  with an advancing watermark means the store is idle (or your graph scope
  is quiet); a watermark that never moves means check the writer.
- **Cursors never expire.** Spanner retains change records for 1–30 days;
  quipu's fact log is permanent, so a consumer can resume from any
  transaction id it ever held, including 0.

## Value capture modes

| Mode | A record carries |
|------|------------------|
| `new_values` (default) | Asserts carry `value`; a retract identifies `(entity, attribute)` but withholds the ended value. |
| `old_and_new_values` | A retract also carries the value it ended, as `old_value`. |
| `new_row` | `old_and_new_values`, plus the entity's full state **as of that record's transaction** under `row` — so consumers skip the read-back that a bare notification would force. |

The `row` snapshot uses the same as-of-transaction predicate the fork
snapshot uses, so the two surfaces cannot disagree about what "live at tx N"
means.

## Reading it

```bash
quipu changes --db homelab.db                        # from genesis, new_values
quipu changes --from 42 --capture new_row --limit 50 # page after tx 42
quipu changes --graph "http://example.org/graph/tenant-a"
```

```text
GET /changes?since=42&capture=old_and_new_values&limit=100
GET /changes?graph=http://example.org/graph/tenant-a
```

Both return one page:

```json
{
  "records": [
    {
      "tx": 43, "sequence": 0, "timestamp": "2026-04-03", "actor": "ingest",
      "source": "episode:...", "op": "assert", "graph": "ROOT",
      "entity": "http://example.org/koror",
      "attribute": "http://example.org/cpuCores", "value": 8
    }
  ],
  "next_tx": 43,
  "watermark_tx": 43,
  "watermark_timestamp": "2026-04-03",
  "capture": "new_values"
}
```

Pass `next_tx` back as the cursor (`--from` / `since`). On an empty page it
stays put, so polling is a fixpoint, not a rewind. `Ref` values resolve to
`{"ref": "<iri>"}` so a consumer can tell an edge from a string that happens
to look like one; bytes report a length, not a body — the feed is a
notification surface, and blob bodies belong on the entity read path.

## First consumers

The intended first consumer is incremental index/embedding maintenance
(bobbin): re-embed exactly the entities whose facts changed rather than
rescanning, using per-entity ordering to apply changes safely. The event-push
delivery worker's semantic events remain the right surface for workflow
triggers; the change feed is for mirrors.
