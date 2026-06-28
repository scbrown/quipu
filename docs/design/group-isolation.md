# Design: Group Isolation / Multi-Tenant Partitioning

**Status:** Proposed — decision pending (hq-2u3). **Recommendation: do not build
yet.** Gate on a concrete multi-tenant requirement.

**Related:** hq-ct3 (analysis: `group_id` is a flat label, not a partition),
hq-fhc (episode idempotency), the `group_ids` parameter on `quipu_search_nodes`
/ `quipu_search_facts`.

## Problem

Today `group_id` looks like a tenancy boundary but is not one. Concretely:

- It is a single `aegis:groupId` literal stamped on the episode `prov:Activity`.
- Nodes created by an episode inherit the group only *transitively*, by walking
  `prov:wasGeneratedBy` back to the activity — there is no group on the node
  itself.
- Facts asserted via `/knot` (and any non-episode write path) carry **no group
  at all**.
- The `group_ids` filter on the search tools is **best-effort, post-hoc**: it
  narrows results that happen to trace back to a matching activity. It does not
  and cannot guarantee isolation, and silently returns ungrouped facts.

This is fine for *provenance* ("which episode produced this?"). It is **not**
safe for *isolation* ("tenant A must never see tenant B's data"). Treating the
current `group_ids` filter as an isolation boundary is the trap to avoid; the MCP
docs already hedge it as "best-effort … not an isolation boundary."

## Decision gate

**Build true isolation only when a real multi-tenant requirement exists** — i.e.
distinct principals whose data must be mutually invisible, with an access-control
story that says so. Until then, the cost (data-model change + migration +
query-path enforcement on every read) is not justified, and the current
provenance label is the right amount of machinery.

Signals that would flip this decision:

- A second tenant whose data must not leak into another tenant's search/SPARQL.
- A compliance or contractual boundary ("customer data segregation").
- An access-control layer (beyond the existing read-only / bearer-token server
  controls from hq-azs) that needs a data-scoping primitive to enforce.

If the only need is "tag where this came from" or "narrow my own results," the
existing label + best-effort filter already covers it. Say so and close.

## If isolation is wanted: design sketch

Four problems must be solved together; solving any subset yields a leaky
boundary that is worse than none (because it *looks* enforced).

### (a) Carrier — where the group lives in the data model

| Option | Sketch | Pros | Cons |
|--------|--------|------|------|
| **Per-fact group column** | Add a `group` column to the `facts` table (and `vectors`); every datum carries its group. | Uniform; cheap to filter (`WHERE group = ?`); works for `/knot` and episodes alike; indexable. | Schema migration; every write path must set it; cross-group references need a policy. |
| **Named graphs / quads** | Move from triples to quads; the 4th term is the graph = group. | Standards-aligned (SPARQL `GRAPH`); clean separation. | Large engine change (the SPARQL layer is triple-oriented today); pervasive. |
| **Group as a typed entity + membership edges** | Keep triples; model `aegis:inGroup` edges. | No schema change. | Same best-effort/post-hoc weakness we have now; not enforceable at the storage layer. |

**Lean:** the **per-fact `group` column** is the smallest change that yields
*enforceable* scoping with the current triple store. Named graphs are the
"right" long-term model but a much larger lift; revisit only if quads are wanted
for other reasons.

### (b) Propagation — every asserted fact gets a group

- `/knot`, `quipu_episode`, resolution writes, reasoner-derived facts, and
  backfill paths must all stamp the group.
- Source of the group: an explicit `group` parameter on the write tools, falling
  back to a server-/connection-bound default (e.g. derived from the
  authenticated principal once auth carries identity, building on hq-azs).
- Ungrouped writes must be a deliberate, named bucket (e.g. `__shared__`), never
  an accidental "visible to everyone."

### (c) Enforcement — scoping is mandatory, not best-effort

- Reads (SPARQL, vector/FTS, `quipu_ask`, context pipeline) take an **ambient
  group scope** and filter at the storage layer, not as an optional post-filter.
- The scope is set by the caller's identity/connection, not a query parameter a
  client can widen.
- Cross-group reads require an explicit, audited capability (e.g. an admin scope
  or an allowlist of shared groups).
- Decision needed: are cross-group *references* (a fact in group A pointing at an
  entity owned by group B) allowed, denied, or copy-on-reference?

### (d) Migration — existing ungrouped facts

- Backfill a `group` for historical facts: episode-generated facts inherit their
  activity's `aegis:groupId`; everything else lands in `__shared__`.
- Bitemporal store ⇒ the migration is itself a transaction; old states remain
  time-travelable and must keep resolving (no retroactive hiding that breaks
  `as_of` queries).
- Provide a dry-run report (counts per derived group, count landing in
  `__shared__`) before committing.

## Recommendation

1. **Do not build now.** Record this doc as the decision and close hq-2u3 as
   "design captured; gated on requirement."
2. Keep the docs honest: `group_id` is **provenance**, and `group_ids` filtering
   is **best-effort** (already noted in `mcp-tools.md`). Resist any caller
   treating it as isolation.
3. If/when a real tenant boundary appears, reopen with the **per-fact `group`
   column** as the starting design, and require (a)–(d) to ship together — a
   half-enforced boundary is a security footgun.

## Open questions (for keeper review)

- Is there any near-term multi-tenant requirement, or is Quipu single-trust-domain
  for the foreseeable future?
- Should authentication (hq-azs) eventually carry a principal identity that a
  group scope could bind to? That would make per-fact grouping enforceable
  without a separate access layer.
