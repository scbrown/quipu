# Episode-Scoped Logical Retraction

> **Implementation status (2026-07-23, kelly):** ✅ **Implemented** — verified by
> mechanism. `Store::retract_episode_with_policy` + `OrphanPolicy::{Preserve,Refuse,Allow}`
> (`src/store/ops.rs`, both-outcomes tested in `src/store/tests.rs`); the
> `/episode/retract` and single-statement `/retract` routes (`src/server.rs`,
> `src/http_auth.rs` write-endpoints); the `tool_retract_episode` MCP tool
> (`src/mcp/tools.rs`); the `identity_orphans` report + default `preserve` policy are
> all live. Exercised in production during this sweep.
>
> Created: 2026-06-29
> Status: IMPLEMENTED (aegis-hxb); identity-preservation contract added 2026-07-19 (aegis-arup)
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

## Identity Is Not Collateral: Ghost Nodes (aegis-arup)

Shared-IRI safety above is about *whose facts* survive. It says nothing about
*what kind* of fact — and that gap had teeth.

Episode scope is **not attribute-aware**: `rdfs:label` and `rdf:type` are
ordinary facts. So if episode A declared a node's identity and episode B later
added an inbound edge to it, retracting A closed the label and the type while B's
edge stayed live. The result is a **GHOST**: a node that

- still exists, is still reachable by IRI, still answers `?s <pred> ?o`, still
  counts in `/stats` — and
- is invisible to a `rdfs:label` regex scan and to `SELECT ?s WHERE { ?s a T }`,
  which is exactly the discovery path the read path and the agent-facing skills
  tell every caller to use.

Present and unfindable at the same time: every count right, every list wrong.
Measured on the live graph by maldoon 2026-07-15 — his own node came out of a
retraction holding `rdfs:comment`, `prov:wasGeneratedBy` and `applies_to`, with
no name and no type. It also made his verification query lie to him: a
label-joined check reported "0 edges" when the edges were fine, and nearly had
him report destroyed data that was never destroyed.

The deeper problem was the silence. `{"retracted": N}` was returned identically
whether you re-posted the identity afterwards or walked away — **the API could
not tell a cleanup from a mutilation**, and neither could the caller. "Remember
to re-post" is prose, not a control.

### The contract

`on_orphan` decides what happens when a retraction would strip the identity of a
node that OTHER writes still reference:

| `on_orphan` | Behaviour |
|---|---|
| `preserve` (**default**) | The node's `rdfs:label` / `rdf:type` stay **active**. A node that survives the retraction keeps its name and type. |
| `refuse` | The whole retraction is rejected (HTTP 400, nothing written), naming the nodes at risk. |
| `allow` | Legacy strict scope — identity goes too, ghosts are created. Still **reported**. |

Two boundary conditions keep `preserve` from over-reaching:

- A node **nothing else references** is not at risk — it leaves the graph whole,
  identity included. Preserving its label would leave a stub, which is its own
  debris.
- If **another episode independently asserts** a label or type, ours is not the
  node's only identity, so retracting ours orphans nothing and it goes.

Idempotency is unchanged: a second retraction re-selects the preserved identity
facts and preserves them again — `tx_id == NOOP_TX`, `retracted == 0`.

Whatever the policy, the response **always** reports `identity_orphans` and
`identity_orphan_entities`. Silence was the bug; the count is the control.

### The blunt tool was the only tool

Part of why this bit is that episode granularity was the finest handle
available. Removing two stray edges meant retracting the whole episode — 33
statements for a 2-statement target, a 16x blast radius — and the re-post that
follows is where identity gets lost. `quipu_retract` / `POST /retract` now takes
an optional `value` alongside `predicate`, so entity + predicate + value closes
exactly one `(e, a, v)` statement. Reach for that first; retract the episode only
when you really do mean the episode.

### Detecting existing ghosts

For a whole-graph sweep, note that on this engine `FILTER NOT EXISTS` silently
returns zero rows (aegis-cclm) — the obvious detector passes its own founding
specimen. The idiom that actually works is `OPTIONAL` + `!bound`:

```sparql
SELECT DISTINCT ?s WHERE {
  ?s ?p ?o .
  FILTER(strstarts(str(?s), "http://aegis.gastown.local/ontology/"))
  OPTIONAL { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?l } FILTER(!bound(?l))
}
```

Exclude `stmt_*` reification nodes and the `aegis` root — those are unlabelled
by design.

## Surface

- Store: `Store::retract_episode(name, timestamp, actor) -> (tx_id, Vec<Fact>)`
  (applies the default `preserve` policy), and
  `Store::retract_episode_with_policy(name, timestamp, actor, policy) ->
  RetractEpisodeOutcome` for an explicit policy plus the identity report.
- Tool: `quipu_retract_episode` (`tool_retract_episode`).
- HTTP: `POST /episode/retract` — body `{ "episode": "<name>" }` (aliases
  `episode_id`, `name`; optional `timestamp`, `actor`, `on_orphan`).

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
curl -s http://quipu.example/query -X POST -H 'Content-Type: application/json' \
  -d '{"query":"SELECT ?s ?p ?o WHERE { ?s ?p ?o . <http://aegis.gastown.local/ontology/episode_goldblum-deploy-verify-032> ?p2 ?o2 } LIMIT 5"}'

# 2. Retract each test episode (idempotent; safe to re-run).
for ep in goldblum-deploy-verify-032 goldblum-confidence-verify-032 goldblum-final-verify-032; do
  curl -s http://quipu.example/episode/retract -X POST -H 'Content-Type: application/json' \
    -d "{\"episode\":\"$ep\",\"actor\":\"goldblum\"}"
  echo
done

# 3. Verify the test facts are gone from CURRENT queries (expect 0 rows / ASK false),
#    e.g. the deploy-test edge:
curl -s http://quipu.example/query -X POST -H 'Content-Type: application/json' \
  -d '{"query":"ASK { <http://aegis.gastown.local/ontology/quipu-server> <http://aegis.gastown.local/ontology/running_version_on> ?v }"}'

# 4. Confirm real entities survive (expect the real facts intact).
curl -s http://quipu.example/query -X POST -H 'Content-Type: application/json' \
  -d '{"query":"SELECT ?p ?o WHERE { <http://aegis.gastown.local/ontology/quipu-server> ?p ?o }"}'

# 5. IDENTITY POST-CHECK (aegis-arup). "Facts gone, shared entities intact" reads
#    GREEN over a ghost — step 4 cannot catch this, because a ghost still answers
#    ?p ?o. Check each retraction's own report, and then that the survivors are
#    still FINDABLE by name and by type:
#      - every response above must show "identity_orphans": 0, or (under the
#        default preserve policy) a matching "identity_preserved" count;
#      - and the label scan must still return them:
curl -s http://quipu.example/query -X POST -H 'Content-Type: application/json' \
  -d '{"query":"ASK { <http://aegis.gastown.local/ontology/quipu-server> <http://www.w3.org/2000/01/rdf-schema#label> ?l }"}'
```

History is preserved: the retracted test facts remain visible via `/cord`
time-travel, now closed — traceable, not erased.
