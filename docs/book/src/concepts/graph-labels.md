# Graph Labels & the Trust Lattice

> **Implementation status:** 🟨 **Substantially built.** The axes
> (`Freshness`, `Trust`, `PolicyClass`, `Durability`, `DataKind`), the
> meet/join algebra in `src/lattice.rs` / `src/lattice_kind.rs`, label storage (`set_graph_label` / `label_of`, the
> reserved meta-graph, cache columns), dataset labels on the query path, label
> floors, expiring declarations, and derivation methods are all implemented.
> Statement-level labels (quipu #73) remain design-only. See
> `docs/design/graph-labels.md` for the full design.

Named graphs partition the store, but on their own they cannot say anything
*about* a graph: how current its contents are, how far they should be trusted,
or what policy governs them. Every consumer of Quipu was hand-rolling that
missing layer differently. Graph labels make it a store primitive: labels on
graphs, drawn from ordered value sets, composing under one invariant.

## The one invariant: composition never widens

When graphs are combined into a dataset, the composed label may never claim
more than any member does. The operator flips direction by axis, but the
invariant holds both ways:

- **Freshness, trust, and durability compose by meet** — a union of graphs is
  only as fresh, as trusted, and as durable as its weakest member.
- **Policy obligations and kinds compose by join** — a union of graphs carries
  the *union* of their restrictions, so one `no-export` graph taints the set,
  and the *union* of their declared kinds, so a dataset touching an `archive`
  graph says so.

## The axes

| Axis | Values | Compose |
|---|---|---|
| `quipu:freshness` | `fresh` > `recomputing` > `stale` | meet |
| `quipu:trust` | IRIs ranked by a declared chain | meet, within one chain |
| `quipu:policyClass` | set of obligation tokens (`pii`, `no-export`, …) | join (union) |
| `quipu:durability` | `backed` > `reproducible` > `soleRecord` | meet |
| `quipu:dataKind` | one token per graph (`knowledge`, `operational`, `identity`, `archive`, …) | join (union) |

Four things to know about these values:

- **Trust is not a hardcoded enum.** Trust values are IRIs and the ordering is
  data: `smac:canonical quipu:trustRank 40 ; quipu:inChain smac:ruleTierChain`.
  Ranks are only comparable *within* a declared chain — comparing across chains
  is refused, never silently compared as integers.
- **Durability answers an owner-facing question** the belief axes do not:
  which facts would be lost if this store were lost? `backed` is persisted
  elsewhere, `reproducible` is re-derivable from a source that still exists,
  `soleRecord` means loss is permanent. A derived fact is only as durable as
  its least durable input, so it composes by meet.
- **Nothing is ever synthesized.** A producer declares labels; Quipu never
  observes staleness or infers `backed` because a backup ran once. Related but
  distinct: `quipu:derivedBy` records *how* to re-derive a fact (system, query,
  parameters). It is a per-fact value, not a lattice axis — two methods do not
  meet into a third. `derivedBy` says how to recover a fact; `durability` says
  whether you must.
- **Kind is categorical, not ordered.** `quipu:dataKind` declares *what sort*
  of data a graph holds — the token space is lexically open
  (`[a-z][a-z0-9-]*`), parsed strictly, and never ranked, so it composes by
  union rather than by weakest member. It also drives fetch-time scope
  widening (`include_kinds`) and the deep-freeze lifecycle — see
  [Graph Kinds & Deep Freeze](graph-kinds.md).

## Undeclared is not a lattice value

Every pre-existing graph is unlabelled, so the default matters. Defaulting to
top would fail-open trust; defaulting to bottom would drag every existing query
to the floor. Instead a composed label is a pair: the fold over the *declared*
labels, plus a `coverage` (`full`, `partial`, `none`) saying how much of the
dataset declared anything. An unlabelled graph reads as *undeclared*, never as
a fabricated `fresh`. Declarations can also expire (`set_graph_label_until`);
past their `valid_to` they simply become undeclared again.

## Labels on the query path

The composed label is a property of the query's dataset, computed once per
query — not per row. A dataset containing a stale graph is labelled stale even
if no returned row came from it; conservative cannot overstate. `/query` and
`quipu_query` responses carry a top-level `labels` key beside `truncated`:

```json
{
  "rows": ["..."],
  "truncated": false,
  "labels": {
    "freshness": { "value": "stale", "coverage": "full" },
    "durability": { "value": "soleRecord", "coverage": "partial" },
    "trust": { "value": "smac:canonical", "chain": "smac:ruleTierChain", "coverage": "full" },
    "policy": { "value": ["no-export"], "coverage": "full" },
    "kind": { "value": ["knowledge", "archive"], "coverage": "full" }
  }
}
```

The key is always present: `null` means nothing was declared, and a fold
refusal (member graphs trusted in different chains) is reported as
`{"error": …}` while the query still returns its rows. Old clients ignore the
extra key. Per-row label columns exist only where the graph is already bound
per row — under `GRAPH ?g`, opt-in.

## Label floors

An opt-in enforcement floor in configuration:

```toml
[quipu.labels]
min_freshness = "fresh"
min_trust_rank = 30
min_trust_chain = "https://quipu.dev/ontology/defaultTrustChain"
deny_policy_tokens = ["no-export"]
deny_data_kinds = ["archive"]
```

When a dataset's label falls below the floor, the query is **refused**, and
the refusal names the graph that dragged the label down. Undeclared fails a
configured floor — fail-safe at enforcement, honest at reporting. The one
deliberate exception is `deny_data_kinds`, which is a **blocklist**, not a
minimum: an *undeclared* kind passes, because kind is categorical and failing
every unlabelled graph the moment the key is set would be a different (and
unasked-for) migration. All keys are unset by default, and **unset means zero behaviour change**: no query is ever
refused by a store that has not configured a floor, and the unconfigured path
does no label work at all.

**⚠ Label floors are NOT access control.** A floor refuses a *query*; it does
not hide rows, and nothing stops a caller who names a graph directly from
reading it. `aegis:authorityOver` gates writes only; a read-side authority
check does not exist and is not built here. Presenting trust labels as a
confidentiality boundary would repeat the `group_id` mistake this stack
already documents.

One governance consequence worth knowing: labels live as ordinary facts in the
reserved meta-graph `urn:quipu:graph:meta`, so relabelling a graph requires
authority over the *meta-graph*, not over the graph being labelled — otherwise
a tenant could relabel itself `attested`.

## Built vs designed

Built: the five axes and the fold, graph-label storage with the RDF meta-graph
as source of truth and cache columns checked by `quipu doctor labels`, dataset
labels on the query path, label floors, expiring declarations, durability and
derivation. Design-only: statement-level labels (#73) — the same vocabulary
attached to individual statements, with downward-only override.

## Related

- [Named Graphs](named-graphs.md) — the substrate labels attach to
- [Graph Kinds & Deep Freeze](graph-kinds.md) — the `dataKind` axis in use
- [Configuration](../getting-started/configuration.md) — the `labels.*` floor keys
- [REST API](../reference/rest-api.md) — the `/query` response shape
- Design: `docs/design/graph-labels.md`
