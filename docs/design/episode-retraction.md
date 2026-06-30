# Episode-Scoped Logical Retraction

> Created: 2026-06-29
> Status: IMPLEMENTED (aegis-hxb)
> Related: [vision.md](./vision.md), [../book/src/architecture/episodes.md](../book/src/architecture/episodes.md)

## One-Line

Retract everything a single episode's ingest contributed — and nothing else — by
closing the bitemporal `valid_to` of exactly the facts that episode's transaction
wrote, surfacing the store's existing internal retraction path over HTTP.

## The Motivating Problem

Before this, Quipu writes were assert-only: `/episode` and `/knot` add facts;
`/cord` / `/unravel` are read-only time-travel. The bitemporal store supported
retraction internally (`op=Retract`, `valid_to`) but nothing surfaced it. So
removing a bad or test-only episode meant hand-surgery on SQLite.

That is dangerous because episodes reuse **real, shared entity IRIs**. A test
episode that touched `quipu-server` and `kota` cannot be undone by deleting all
triples about those entities — that would destroy legitimate graph data.

## The Unit: the Episode's Transaction

Every episode ingest goes through **one** `Store::transact` call stamped
`source = "episode:{name}"` (`episode::ingest_episode` → `rdf::ingest_rdf`).
That transaction source is the complete, precise provenance handle:

- **Complete** — it covers the episode activity node, generated entities, the
  bare relationship triples (edges), and reified confidence statements. By
  contrast `prov:wasGeneratedBy` only links *entity nodes*, so it would miss
  edges and reifications.
- **Precise** — idempotent assertion (`transact` skips a duplicate active
  `(e, a, v)`) means each active fact has exactly **one** owning transaction.

So "retract episode X" = close every currently-active asserted fact whose owning
transaction carried `source = "episode:X"`.

### Why this is shared-IRI-safe

Retracting episode X closes only the facts X's transaction actually wrote. A fact
about a shared entity (`quipu-server`, `kota`) that was first asserted by a real
episode keeps that real episode as its owner, so it survives. Re-ingests of the
same episode create multiple transactions that all share the source tag, so every
one of the episode's currently-active contributions is caught.

## Mechanism: Logical, Not Physical

`Store::retract_episode` builds `Op::Retract` datums for the in-scope facts and
commits them through the normal `transact` path. This sets `valid_to` on the
original assertions and records retract rows — it never deletes anything. So:

- The facts drop out of **current** queries (`current_facts`, `/search`, SPARQL
  at `valid_now`).
- Time-travel (`/cord`, `/unravel`, `facts_as_of`, `entity_history`) still shows
  them, now closed.

Idempotent: a second retraction finds no active facts and is a no-op
(`tx_id == NOOP_TX`, `retracted == 0`). Unknown episodes are likewise no-ops.

## Surface

- Store: `Store::retract_episode(name, timestamp, actor) -> (tx_id, Vec<Fact>)`.
- Tool: `quipu_retract_episode` (`tool_retract_episode`).
- HTTP: `POST /episode/retract` — body `{ "episode": "<name>" }` (aliases
  `episode_id`, `name`; optional `timestamp`, `actor`).

## Authorization (hq-azs / hq-otm)

Retraction is a write, and a **more sensitive** one than assertion: it removes
facts from current views. `/episode/retract` is registered in
`http_auth::WRITE_ENDPOINTS`, so today it honours read-only mode and the bearer
token exactly like every other write — under the LAN-trusted default (no token)
it is open like the other writes.

**Requirement for when auth lands:** once per-principal scopes (hq-azs) and crew
identity (hq-otm) are in place, retraction should be gated to an *authorized
principal* — a distinct, higher-trust scope — not merely the same bearer token
that permits assertion. The current single-token model cannot express that
distinction; the gate must be tightened when the identity layer exists.

## First Use: Prune the Goldblum Deploy-Verification Episodes

The first production use of this endpoint is to clean up the bounded,
provenance-marked test episodes left on the live ontology by the aegis-7ui
deploy verification:

- `goldblum-deploy-verify-032`
- `goldblum-confidence-verify-032`
- `goldblum-final-verify-032`
- (plus any dearing co-verify / ian tx341 test episodes, if present)

> **Ownership:** this cleanup runs against the live Quipu store on **kota**
> (`/var/lib/quipu/quipu.db`). It is a **separate goldblum deploy step** — it
> requires the new `quipu-server` binary to be deployed there first. The
> implementing polecat does **not** touch the live store.

### Runbook (run by goldblum after the binary is deployed to kota)

```bash
# 1. Confirm a test episode's facts are currently live (expect rows).
curl -s http://quipu.svc/query -X POST -H 'Content-Type: application/json' \
  -d '{"query":"SELECT ?s ?p ?o WHERE { ?s ?p ?o . <http://aegis.gastown.local/ontology/episode_goldblum-deploy-verify-032> ?p2 ?o2 } LIMIT 5"}'

# 2. Retract each test episode (idempotent; safe to re-run).
for ep in goldblum-deploy-verify-032 goldblum-confidence-verify-032 goldblum-final-verify-032; do
  curl -s http://quipu.svc/episode/retract -X POST -H 'Content-Type: application/json' \
    -d "{\"episode\":\"$ep\",\"actor\":\"goldblum\"}"
  echo
done

# 3. Verify the test facts are gone from CURRENT queries (expect 0 rows / ASK false),
#    e.g. the deploy-test edge:
curl -s http://quipu.svc/query -X POST -H 'Content-Type: application/json' \
  -d '{"query":"ASK { <http://aegis.gastown.local/ontology/quipu-server> <http://aegis.gastown.local/ontology/running_version_on> ?v }"}'

# 4. Confirm real entities survive (expect the real facts intact).
curl -s http://quipu.svc/query -X POST -H 'Content-Type: application/json' \
  -d '{"query":"SELECT ?p ?o WHERE { <http://aegis.gastown.local/ontology/quipu-server> ?p ?o }"}'
```

History is preserved: the retracted test facts remain visible via `/cord`
time-travel, now closed — traceable, not erased.
