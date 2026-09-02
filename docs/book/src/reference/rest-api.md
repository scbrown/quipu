# REST API

The `quipu-server` binary exposes all Quipu operations over HTTP (Axum).

## Starting the Server

```bash
quipu-server --db my.db --bind 0.0.0.0:3030
```

| Flag | Description |
|------|-------------|
| `--db <path>` | Store database path (default: `.bobbin/quipu/quipu.db`) |
| `--bind <addr>` | Bind address (default: `127.0.0.1:3030`) |

## `GET /.well-known/void`

Returns a live VoID and SPARQL 1.1 Service Description projection. The default
representation is Turtle; send `Accept: application/ld+json` for JSON-LD. The
document advertises the query endpoint, exact dataset and named-graph counts,
used vocabulary namespaces, executable result formats, and compiled entailment
features. Quipu's share `manifest.json` remains the integrity contract.

## Read Concurrency

Reads are served from a pool of read-only connections; writes keep the single
FIFO-fair writer connection. WAL already permits N concurrent readers alongside
one writer — before the pool, every read took the writer's mutex, so that
capability was present and unused.

```toml
[quipu.server]
read_pool_size = 4    # 0 disables the pool; every read then serialises
```

MEASURED on a 160k-fact store, same binary, pool the only variable (`server-CPU`
divided by wall time, so it counts cores actually used rather than inferring
them):

| | N=8 concurrent | N=16 |
|---|---|---|
| `read_pool_size = 0` | 1.09s, **1.00 cores** | 2.18s, **0.99 cores** |
| `read_pool_size = 8` | 0.43s, **6.40 cores** | 0.80s, **6.80 cores** |

`quipu_store_wait_seconds_total` — time spent acquiring, exported on `/metrics` —
falls to **0.000s** with the pool on. That is the number to watch: a rising
`wait` means readers are queueing again.

Wall-clock speedup is smaller than the core count because each query costs more
CPU when eight run at once (2.6x for a full scan, 1.4x for an index lookup —
shared-cache contention, not a lock). The pool removes the serialisation; it
cannot make a memory-bandwidth-bound scan free.

Two cases where the pool disables itself, both announced on stderr at startup:

- **an in-memory store** — each `:memory:` connection is its own empty database,
  so a pool there would not be slow, it would be wrong;
- **a configured vector delegate or local vector backend** — those are not
  shareable with a read-only connection, and several pooled handlers
  (`/search_nodes`, `/search_facts`, `/unified_search`, `/ask`) are vector-backed.
  Rather than answer the same question from two different indexes depending on
  which connection took it, the pool stands down.

`read_pool_size = 0` is the rollback, and it is runtime config — no redeploy.

## Authentication

**Reads are open; writes need a bearer token.** When the server is started with an
auth token configured, every *write* endpoint requires:

```text
Authorization: Bearer <token>
```

Reads — `/query`, `/search`, entity lookups, `/health`, `/version` — need no
credential and answer normally.

`POST /share` is also read-only. It returns the canonical Git-share manifest and
exact file contents in one JSON response, allowing a proxy on another host to
forward Quipu's own canonicalization, hashes, and share ID instead of reproducing
them. The body accepts `scope`, `shapes`, `no_shapes`, `parent_share`,
`turtle_view`, and an optional `max_bytes` that can lower (but not raise) the 8
MiB server cap.

**The authoritative list is `http_auth::WRITE_ENDPOINTS` in `src/http_auth.rs`, not
this page.** It is enforced: `write_endpoints_cover_every_route` fails the build if
any registered route is unclassified, so the code cannot drift from itself — but
this page can drift from the code, so treat it as a summary and the constant as the
answer.

Two entries surprise people, and both are deliberate:

| Endpoint | Why it is a WRITE |
|---|---|
| `/project` | Looks read-only — `stats`, `pagerank`, `ppr`, `components` only read — but `louvain` with `persist: true` **writes** `quipu:memberOfCommunity` and supersedes any prior derivation. The route is gated as a whole. |
| `/shapes` | Gated even to *list*. Loading a shape set persists it, and a listed-but-unloaded set validates nothing while still reporting success. |

A refusal is never silent. Both refusals return a JSON body naming
the cause, so `curl -s` cannot render an auth failure as an empty result:

```json
{"endpoint":"/project","reason":"missing_or_invalid_bearer_token","error":"unauthorized: ..."}
{"endpoint":"/knot","reason":"server_is_read_only","error":"read-only mode: ..."}
```

`reason` is the stable field to branch on; `error` is prose and may be reworded.

## Request Attribution

Callers can attach two optional headers to every endpoint:

| Header | Meaning | Missing/invalid value |
|--------|---------|-----------------------|
| `X-Quipu-Client` | Stable caller kind, such as `query-first` or `graph-extract` | Falls back to `User-Agent`, then `unattributed` |
| `X-Quipu-Task` | Work-item join key, normally a bead id such as `aegis-3aybc` | `unattributed`; there is deliberately no inferred fallback |

Both values appear in request start/completion logs. The `client`, `task`, and
route-template dimensions are also exported by
`quipu_http_client_requests_total` and
`quipu_http_client_request_seconds_total`. Client and task identities have
independent cardinality budgets; excess caller-controlled values fold into a
visible `other` bucket instead of growing the registry without bound. Use
`increase(...[window])` for counter comparisons so process restarts do not
invalidate the result.

## Endpoints

All POST endpoints accept `Content-Type: application/json`.

### `GET /health`

Health check.

```bash
curl localhost:3030/health
```

Response: `{"status": "ok"}`

### `GET /stats`

Store statistics.

```bash
curl localhost:3030/stats
```

Response: `{"facts": 1234, "entities": 56, "predicates": 12}`

### `POST /query`

Execute a SPARQL query.

```bash
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"}'
```

Optional fields: `valid_at` (ISO-8601), `tx` (integer), `graph` (a
named-graph IRI or dataset name that scopes the query's *default* graph
without writing a `FROM`/`GRAPH` clause — an unknown IRI yields an empty
default graph, never a silent ROOT fall-through), and `fork` (a fork name
registered by `quipu fork`; unknown or dropped forks are refused loudly;
mutually exclusive with `graph`).

`include_kinds` (array of `dataKind` tokens, e.g. `["archive"]`) widens the
default graph set with every registered graph declaring one of those kinds —
the explicit opt-in for composing cold/frozen graphs into a hot read. Silence
never widens: absent or empty means the scope is unchanged, a `FROM` clause in
the query text still overrides the request-level scope, and `fork` +
`include_kinds` is refused (one scope authority). The response's composed
`labels.kind` then honestly reports every kind that contributed.

With `"federated": true` the whole query text fans out through the federated
provider — the local store plus every `[[quipu.federation.remotes]]` — and the
response adds a per-member `providers` list (each carrying the remote's
operator-declared `label`) and a `complete` flag, with every row
`_provider`-tagged and, for declared remotes, `_trust`/`_freshness`-stamped.
The composed dataset `labels` fold the remotes in as members, and configured
`[quipu.labels]` floors refuse a federated query exactly as a local one — an
undeclared remote fails a configured freshness/trust floor (quipu-fd1). The
temporal/graph fields are refused on a federated query (they only shape the
local evaluator's context). See
[Federation](../architecture/federation.md).

### `POST /knot`

Assert facts from Turtle data.

```bash
curl -s localhost:3030/knot -X POST \
  -H "Content-Type: application/json" \
  -d '{"turtle": "@prefix ex: <http://example.org/> . ex:alice a ex:Person ."}'
```

Optional fields: `shapes` (SHACL Turtle), `timestamp`, `actor`, `source`,
`replace_snapshot` + `snapshot` (diffed replacement of a producer's prior
facts under a stable key), and `graph` (a named-graph IRI that must already
be registered committed-class via `POST /graph/create`; unknown IRIs error,
overlay-class targets are refused, omitted means ROOT).

Response: `{"tx_id": 1, "count": 2, "conforms": true}`

### `POST /cord`

List entities.

```bash
curl -s localhost:3030/cord -X POST \
  -H "Content-Type: application/json" \
  -d '{"type": "http://example.org/Person", "limit": 50}'
```

### `POST /unravel`

Time-travel query.

```bash
curl -s localhost:3030/unravel -X POST \
  -H "Content-Type: application/json" \
  -d '{"tx": 5}'
```

### `POST /episode`

Ingest an episode.

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "deploy-v2",
    "nodes": [{"name": "myapp", "type": "WebApplication"}],
    "edges": [{"source": "myapp", "target": "kota", "relation": "runs_on"}]
  }'
```

Set `"replace_snapshot": true` for producers whose payload is the complete
current state of an inventory. Facts previously asserted by the same episode
name but absent from the new payload are retracted atomically with the new
assertions. The default is `false`, preserving additive knowledge-ingestion
semantics. Reusing a stable episode name is required for replacement.

#### `outcome`: what the ingest DID — branch on this, never on `count`

`/episode` is idempotent. The activity IRI is derived from the episode name and
stamped with a content hash, so **re-posting identical content is a no-op and
retrying after a lost response is SAFE**.

The response says which of three things happened:

| `outcome` | Meaning | `count` | `tx_id` |
|---|---|---|---|
| `created` | The episode did not exist; its facts were written. | > 0 | > 0 |
| `updated` | It existed with DIFFERENT content; stale activity facts were retracted and the new content written. | > 0 | > 0 |
| `unchanged` | It already existed with identical content. Nothing was written and nothing needed to be. **This is success.** | 0 | 0 |

**Why this field exists.** Before it, the idempotent no-op returned `count: 0,
tx_id: 0` — byte-for-byte what a write that achieved nothing returns — while the
documented success check for callers of this API was "HTTP 200 with `count > 0`".
So a successful retry reported as a failure. The natural recovery from "my
episode did not land" is to re-post it under a different name or with re-worded
nodes, and *that* mints duplicate entities. The safe mechanism was steering
callers into the unsafe action.

So the success check is:

```bash
# right: the facts are in the store for all three outcomes
curl -s .../episode -X POST ... | jq -e '.outcome' >/dev/null

# WRONG: reports a successful idempotent retry as a failure
curl -s .../episode -X POST ... | jq -e '.count > 0'
```

`count > 0` remains a useful *"did this call write anything"* question. It was
never a *"did the write land"* question, and only looked like one because the
first post and the only post were usually the same post.

Two things it does **not** promise, both still on the caller:

- **`outcome` describes THIS episode name.** Re-posting the same knowledge under
  a *different* name is a new episode and will be `created` — idempotency is
  keyed on the name plus content hash, not on meaning.
- **A `200` still is not proof of retrievability.** A node filed under a type
  nobody queries is `created` and unreachable. Ask it back the way a reader
  would.

#### Edge `relation`: which vocabularies `/episode` can write

`/episode` used to force **every** relation into `aegis:` and then sanitize it, so
`"relation": "rdfs:subClassOf"` was stored as `aegis:rdfs_subClassOf` — a predicate
that resembles the intended one, matches nothing, and is inert — behind HTTP 200 with
a healthy `count`. It no longer does. The policy is now: **represent the caller's
predicate faithfully, or refuse and say which path to use.** Never silently rewrite it.

| `relation` | Emitted |
|---|---|
| `runs_on` | `aegis:runs_on` — the domain vocabulary, unchanged |
| `owl:sameAs`, `rdfs:seeAlso`, `rdf:*`, `skos:*`, `prov:*`, `quipu:*`, `xsd:*`, `sh:*` | verbatim, in that namespace |
| `<http://example.org/p>` | verbatim (full IRI in angle brackets) |
| `foo:bar` (undeclared prefix) | **400**, naming `/set` and the angle-bracket form |
| `runs on` (would not round-trip sanitization) | **400** — it would be silently renamed |

The declared prefix set is `KNOWN_PREFIXES` in `src/episode/mod.rs`, kept in lockstep
with the `@prefix` block `episode_to_turtle` emits.

#### Asserting an alias — entity dedup with `owl:sameAs`

`owl:sameAs` is this graph's alias convention, and `/episode`'s `resolution_hints`
exist to tell you at ingest time that you are about to split an entity. Acting on that
hint is a normal `/episode` edge:

```bash
curl -s localhost:3030/episode -X POST -H "Content-Type: application/json" \
  -d '{"name": "alias-fix", "source": "<bead-id>",
       "nodes": [{"name": "backup-freshness.timer"}, {"name": "backup-freshness-exporter"}],
       "edges": [{"source": "backup-freshness.timer",
                  "target": "backup-freshness-exporter",
                  "relation": "owl:sameAs"}]}'
```

Two rules that are not obvious from the 200:

- **Reuse the existing node names byte-for-byte.** Node identity here is the literal
  name string: quipu matches `canonical_name:exact` and merges, or it does not match
  and mints a second node. Re-wording a name on a follow-up post is how aliases get
  created rather than resolved. `/search` or `/resolve` first, and copy the name out.
- **`count > 0` proves the write landed, not that a reader can find it.** Follow every
  alias write with the query a reader would actually run:

  ```bash
  curl -s localhost:3030/query -X POST -H "Content-Type: application/json" \
    -d '{"query":"SELECT ?o WHERE { <http://aegis.gastown.local/ontology/backup-freshness.timer> <http://www.w3.org/2002/07/owl#sameAs> ?o }"}'
  ```

  A `0` here on a `200` write is the silent-rewrite shape: the fact is present and
  misnamed. Pair it with a control (query a predicate you know is populated) before
  believing an empty result.

Historical note, since the answer is not guessable from the data: the alias pairs
predating this fix were written through **`/knot`** (Turtle), which is the only write
path that accepts a caller-supplied `source`. Their transaction `source` strings are
free text — e.g. `"schema-gate ruling 2026-07-20"` — where `/episode` always stamps
`episode:<name>`, `/set` stamps `set`, and `/retract` stamps `retract`. Some are stamped `actor: null, source: null`: `/knot` called with neither,
which lands a structural identity fact with no audit trail. Pass `actor` and `source`.

### `POST /set`

Atomic single-call supersede: set `(entity, predicate)` to exactly `value`, retracting
every current object on that predicate and asserting the new one in ONE transaction.
Re-parenting (`reports_to` A → B) is one call with no window where the predicate is
empty and no way to end up multi-valued by forgetting the retract half.

```bash
curl -s localhost:3030/set -X POST -H "Content-Type: application/json" \
  -d '{"entity": "http://example.org/svc",
       "predicate": "http://example.org/reports_to",
       "value": {"iri": "http://example.org/new-boss"},
       "actor": "<who>"}'
```

Optional: `timestamp`, `actor`. Returns
`{"tx_id", "retracted": N, "asserted": 0|1, "entity", "predicate"}`; setting the
already-sole-current value is an idempotent no-op (`tx_id: 0, retracted: 0,
asserted: 0`).

- The **predicate is a full IRI**, from any vocabulary. This is the endpoint
  `/episode` names in its refusal when an edge relation uses an undeclared prefix.
- **SINGLE-VALUE semantics**: all current objects are replaced. For
  add-without-remove, assert via `/knot`.
- The **entity must already exist** — `/set` on a typo'd IRI must not mint an
  unlabelled orphan node. The predicate may be new.
- The `value` shape discipline is the same as `/retract`: a bare string is a
  *literal*; an edge must be `{"iri": "..."}`. A bare IRI-shaped string aimed at a
  Ref-holding predicate is a loud 400, not a mis-shaped write. `{"str": "..."}`
  states that a literal is intended and disarms that heuristic.

### `POST /validate`

Dry-run SHACL validation.

```bash
curl -s localhost:3030/validate -X POST \
  -H "Content-Type: application/json" \
  -d '{"shapes": "@prefix sh: ...", "data": "@prefix ex: ..."}'
```

### `POST /retract`

Retract facts for an entity.

```bash
curl -s localhost:3030/retract -X POST \
  -H "Content-Type: application/json" \
  -d '{"entity": "http://example.org/old-service"}'
```

Optional: `predicate` (only retract matching), `timestamp`, `actor`, and `value`
(retract only the one matching triple).

**The `value` shape matters.** An object that is an IRI reference — the target of
an edge such as `reports_to` or `rdf:type` — must be given as a tagged object, not
a bare string:

```bash
# retract exactly  <svc> reports_to <boss>
curl -s localhost:3030/retract -X POST -H "Content-Type: application/json" \
  -d '{"entity": "http://example.org/svc",
       "predicate": "http://example.org/reports_to",
       "value": {"iri": "http://example.org/boss"}}'
```

A **bare string** (`"value": "http://example.org/boss"`) is matched as a string
*literal*, which can never equal a stored IRI reference. Rather than silently
report `{"retracted": 0}` — indistinguishable from "the triple was already gone" —
the endpoint now **returns a 400 error** naming the `{"iri": ...}` form whenever a
bare string cannot match: either the predicate's stored objects are IRIs, or the
string itself parses as an IRI (has a `scheme://`). A correctly shaped `{"iri":
...}` (or a genuine string literal) for a triple that does not exist is still a
quiet, idempotent `{"retracted": 0}`.

### `POST /episode/retract`

Episode-scoped **logical** retraction. Retracts the facts an episode's ingest
contributed — its activity node, generated entities, the bare relationship
triples (edges), and any reified confidence statements — by closing their
`valid_to` via the bitemporal retract path. Facts are never physically deleted,
so time-travel queries (`/cord`, `/unravel`) still show them.

**Identity of surviving nodes is preserved by default.** Identity triples
(`rdfs:label`, `rdf:type`) are ordinary facts, so a naive scope retraction would
strip them from any node this episode *named* even when edges from other episodes
keep that node alive — leaving a "ghost": a node that answers predicate queries
but is invisible to every label scan and type query. The `on_orphan` parameter
decides that contract:

| `on_orphan` (alias `orphan_policy`) | Behaviour |
|-------------------------------------|-----------|
| `preserve` (**default**) | Keep `rdfs:label` / `rdf:type` alive for nodes that retain surviving references. So the default does **not** retract "every currently-active fact" — it spares the identity of nodes that would otherwise be orphaned. |
| `refuse` | If the retraction would orphan any node's identity, reject the whole operation (400) and change nothing. The safe mode when you do not want to strand entities. |
| `allow` | Legacy behaviour: retract every currently-active fact the episode wrote, orphaned identity included. |

Regardless of policy the response reports `identity_orphans` (a count) and names
the affected nodes, so a caller can tell a cleanup from a mutilation.

The retraction unit is the episode's ingest transaction(s), identified by their
`source = "episode:{name}"` tag. Because identical assertions are deduplicated to
a single owning transaction, retracting an episode only removes the facts *that
episode actually wrote* — entities and facts contributed by other episodes (even
about the same shared IRIs) survive untouched. This is the safe way to undo a
specific episode's contributions without SQL surgery on shared entities.

```bash
curl -s localhost:3030/episode/retract -X POST \
  -H "Content-Type: application/json" \
  -d '{"episode": "goldblum-deploy-verify-032"}'
```

Aliases for `episode`: `episode_id`, `name`. Optional: `timestamp`, `actor`,
`on_orphan` (`preserve` | `refuse` | `allow`, default `preserve` — see the table
above). **Idempotent** — retracting an already-retracted or unknown episode
returns `{"retracted": 0}` and changes nothing.

Response fields: `tx_id`, `retracted` (count), `episode`, `statements` (the
retracted facts), and the identity accounting — `on_orphan` (the policy applied),
`identity_preserved` (count) with `identity_preserved_statements`, and
`identity_orphans` (count) with `identity_orphan_entities` (`entity`,
`lost_label`, `lost_type`).

> **Auth (hq-azs / hq-otm).** Retraction is a write — and a *more* sensitive one
> than assertion, since it removes facts from current views. The endpoint is in
> `http_auth::WRITE_ENDPOINTS`, so it already honours read-only mode and the
> bearer token like every other write. When per-principal scopes (hq-azs) and
> crew identity (hq-otm) land, retraction should be gated to an authorized
> principal, not merely the same token that permits assertion.

### `POST /resolve`

Ask what entity resolution *would* say about a name, without writing anything.

Returns the same candidate list the ingest path computes, so "is this a duplicate
of something we already have?" can be answered **before** minting the entity.

```bash
curl -s localhost:3030/resolve -X POST \
  -H "Content-Type: application/json" \
  -d '{"name": "example-service", "properties": {"type": "DatabaseService"}}'

# {"candidates":[{"iri":"http://example.org/ontology/example-service",
#                "score":0.9,"matched_on":"canonical_name:jaro_winkler:0.90"}],
#  "count":1,"has_matches":true}
```

`name` is required. `properties` (object), `top_k` and `threshold` are optional;
`top_k` and `threshold` default to `[quipu.resolution]` config, so this route and
the ingest path agree by construction rather than by convention.

Both matchers run: Jaro-Winkler over `rdfs:label` (`matched_on:
canonical_name:jaro_winkler:<score>`) **and** vector similarity when an embedding
provider is configured (`matched_on: embedding:<score>`). The embedding half is
the reason a client-side name check is not a substitute.

Notes:

- **It does not write — and that is guaranteed by a test, not by the type.** The
  handler takes a `&Store`, but do not read that as a read-only capability:
  `Store` writes through `&self` methods via interior mutability, so a `&Store`
  handler *can* commit. Several routes registered the same way do write, which is
  why `/overlay/create` sits in `WRITE_ENDPOINTS` despite its signature. What
  actually holds this route read-only is the explicit assertion in
  `tool_resolve_entity_is_a_genuine_read_commits_nothing`.
- **It is not a write endpoint** (absent from `http_auth::WRITE_ENDPOINTS`), so it
  needs no bearer token and answers normally on a `read_only = true` server —
  where `POST /episode` returns 403.
- **It does not require `[quipu.resolution].enabled`.** Resolution being off
  disables the *ingest-time* hints; this route still answers.
- Not to be confused with `POST /reconcile` — the W3C Reconciliation API, which
  has its own substring scoring on a 0-100 scale and does not consult embeddings.
  (It is routed but undocumented here, as are `/spotlight` and `/fragments`.)

### `POST /shapes`

Manage persistent SHACL shapes.

```bash
# Load
curl -s localhost:3030/shapes -X POST \
  -H "Content-Type: application/json" \
  -d '{"action": "load", "name": "person", "turtle": "@prefix sh: ..."}'

# List
curl -s localhost:3030/shapes -X POST \
  -H "Content-Type: application/json" \
  -d '{"action": "list"}'

# Remove
curl -s localhost:3030/shapes -X POST \
  -H "Content-Type: application/json" \
  -d '{"action": "remove", "name": "person"}'
```

Rule Turtle (`a rule:Rule` subjects) may be stored alongside SHACL shapes. A
successful load or remove also **hot-reloads the reactive reasoner's ruleset**
— rules take effect on the next write, no restart (before 2026-08-27 the
ruleset was a startup snapshot).

### `POST /reason`

Run a Datalog ruleset to fixpoint and persist its derivations
(`source = reasoner:<rule-id>`). A write endpoint — derivations assert and
retract through the fact log — so it is bearer-gated like `/episode`.

Derivations land in the target graph's **companion inferred graph**
(`<graph>#inferred`; ROOT's is `urn:quipu:graph:root#inferred`, quipu-0b6).
Read them composed: `FROM <urn:quipu:graph:root> FROM
<urn:quipu:graph:root#inferred>` in a `/query` body. The suffix is reserved —
external writes to a companion graph are refused.

Body fields, all optional: `rules` (inline rule Turtle; absent, the stored
combined shapes are used), `prefix` (default IRI prefix for unqualified
predicate names), `graph` (a named-graph IRI — premises and derivations both
scope to it; absent means ROOT), `timestamp` (valid-from for derived facts).

```bash
# Evaluate the rules already loaded via /shapes, against ROOT
curl -s localhost:3030/reason -X POST \
  -H "Content-Type: application/json" -d '{}'

# Evaluate an inline ruleset against a named graph
curl -s localhost:3030/reason -X POST \
  -H "Content-Type: application/json" \
  -d '{"rules": "@prefix rule: ...", "graph": "http://example.org/graphs/staging"}'
# {"rules":2,"strata_run":1,"asserted":14,"retracted":0,
#  "per_rule":[{"rule":"R1","asserted":14},{"rule":"R2","asserted":0}]}
```

### `POST /explain`

Walk a fact's derivation chain from the provenance in the fact log. Read-only
and open (no bearer token). Body: `s`, `p`, `o` (IRIs; a non-IRI `o` is
treated as a string literal), optional `depth` (default 5).

A base fact answers with its transaction and source. A `reasoner:<rule-id>`
fact answers with the rule and the premise facts it currently re-matches; an
`owl:materialize` fact answers with every axiom family that currently
re-derives it — premises recursed, so the tree bottoms out in base facts.
Support is **re-matched, not stored**: a premise retracted since derivation
shows as absent support, which is itself diagnostic.

```bash
curl -s localhost:3030/explain -X POST \
  -H "Content-Type: application/json" \
  -d '{"s": "http://example.org/a", "p": "http://example.org/dependsOn",
       "o": "http://example.org/c"}'
# {"fact":{...},"found":true,"tx":42,"source":"owl:materialize",
#  "derivation":{"kind":"owl","families":[{"family":"transitive",...}]}}
```

### `POST /search`

Vector similarity search. Body: `embedding` (or `query`), optional `limit`,
`valid_at`, and best-effort scoping by `group_ids` / `entity_type`.

```bash
curl -s localhost:3030/search -X POST \
  -H "Content-Type: application/json" \
  -d '{"embedding": [0.1, 0.2, ...], "limit": 10}'
```

`group_ids` is a best-effort **provenance** filter, not an isolation boundary:
it narrows to entities whose facts trace (via `prov:wasGeneratedBy → episode →
groupId`) to a listed group, and it **drops** ungrouped `/knot` facts (they have
no episode to trace). `entity_type` restricts to an rdf:type IRI. See
[group-isolation](../../design/group-isolation.md).

> ⚠️ **The `type: A, B` in a result's `text` is NOT valid as `/episode` input.** A
> multi-typed entity renders as `... type: Feature, Tool, Concept`, and that string
> looks exactly like a `type` value you could paste back. It is not one — `/episode`
> takes `type` as a SINGLE class and refuses a comma-separated value (400). To give an
> entity several types, send **one node entry per type, repeating the same name**;
> the canonical-name resolver folds them into one entity:
>
> ```json
> {"nodes":[{"name":"governor","type":"Feature"},{"name":"governor","type":"Concept"}]}
> ```
>
> This bites careful readers specifically: searching first to reuse existing
> conventions is what hands you the string, so following the "search before you mint"
> rule is what leads into it.
>
> **Why the rendering is not simply changed:** that `text` is the EMBEDDING SOURCE
> (`src/embedding.rs`, `format!("type: {}", types.join(", "))`), not a display string.
> Altering the separator changes the text every stored vector was computed from, so it
> would need a full re-embed backfill to stay coherent — a much larger change than it
> looks. Documented here rather than "fixed" cheaply and inconsistently.

### `POST /hybrid_search`

Combined SPARQL filter + vector ranking.

```bash
curl -s localhost:3030/hybrid_search -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "sparql": "SELECT ?s WHERE { ?s a <http://example.org/Service> }",
    "embedding": [0.1, 0.2, ...],
    "limit": 5
  }'
```

### `POST /project`

Graph projection and algorithms.

```bash
curl -s localhost:3030/project -X POST \
  -H "Content-Type: application/json" \
  -d '{"algorithm": "in_degree", "limit": 10}'
```

### `GET|POST /report`

Live graph report: top hubs (god-nodes), surprising cross-community connections,
and auto-suggested questions (see `quipu_report` in the
[MCP tools reference](./mcp-tools.md)). Read-only. `GET` returns the report with
defaults; `POST` accepts an options body (`type`, `predicate`, `hubs`,
`surprises`, `questions`).

```bash
curl -s localhost:3030/report
curl -s localhost:3030/report -X POST \
  -H "Content-Type: application/json" \
  -d '{"hubs": 5, "surprises": 5, "questions": 6}'
```

### `GET /graphs`

List registered named graphs with class, source, storage lifecycle, and label
cache (freshness / durability / trust / policy / kind). Query params: `kind`
(a `dataKind` token) and `lifecycle` (`frozen`). Also the consumer
**capability probe** for the graph-kinds surface: a 404 means the store
predates it — treat that as "cannot tell", never as "no graphs".

A graph whose latest RML materialization is on record additionally serves a
`materialization` object (quipu-212): the mapping IRI, mapping-closure hash,
external-truth subject, **verified source hash**, transaction, and timestamp
of the last executor commit — the comparands a freshness verdict needs
(camayoc's `rml_executor.py freshness`/`remap` read them from here). Parsed
from transaction provenance, so it cannot drift from what actually
committed; omitted rather than faked on graphs with no RML history.

```bash
curl -s 'localhost:3030/graphs?kind=operational'
```

### `POST /graph/label`

Declare a graph's labels — any subset of the five axes. Required: `graph`,
`timestamp`. Optional: `freshness`, `durability`, `kind` (a `dataKind` token,
strictly parsed), `trust` (`{"iri", "chain", "rank"}`), `policy` (array of
obligation tokens), `valid_to` (expiring declaration), `actor`. Each axis is
parsed strictly: an unrecognised value is an error, never a dropped axis.
Returns `{"tx_id": N}`. Write endpoint; honors bearer auth.

```bash
curl -s localhost:3030/graph/label -X POST -H "Content-Type: application/json" \
  -d '{"graph": "urn:app:runs/2026-08", "kind": "operational",
       "freshness": "fresh", "timestamp": "2026-08-24T00:00:00Z"}'
```

### `POST /graph/freeze` and `POST /graph/thaw`

The deep-freeze surface — same inputs and outputs as the `quipu_graph_freeze`
/ `quipu_graph_thaw` MCP tools. Freeze relocates a graph's full history into
a read-only archive pack (kept addressable and composable at query time);
thaw restores it. Both are write endpoints and honor bearer auth.

```bash
curl -s localhost:3030/graph/freeze -X POST -H "Content-Type: application/json" \
  -d '{"graph": "urn:app:shuttle/runs/2026-07", "timestamp": "2026-08-24T00:00:00Z"}'
```

### `POST /graph`

Render-ready node-link projection — the single payload the web UI draws from.

```bash
curl -s localhost:3030/graph -X POST \
  -H "Content-Type: application/json" \
  -d '{"limit": 250}'
```

Body: optional `limit` (nodes, ranked by degree; default 250, max 2000),
`type` (restrict to one `rdf:type` IRI), `include_episodes` (default `false`).

```json
{
  "nodes": [{"iri": "…", "label": "kota", "type": "…/ProxmoxNode", "deg": 8}],
  "edges": [[0, 10, "managed_by"]],
  "types": [{"iri": "…", "label": "SystemdService", "count": 15}],
  "truncated": {"shown": 250, "of": 1180},
  "stats": {"nodes": 250, "edges": 612}
}
```

`edges` address nodes by **index** into `nodes`, not by IRI — an IRI averages
~45 bytes and would otherwise repeat at both ends of every edge. `prov:Activity`
episodes and `rdf`/`rdfs`/`prov` scaffolding predicates are excluded by default
so the domain graph is not buried in provenance. `truncated` always states what
was dropped rather than silently capping.

### `POST /context`

Knowledge context pipeline.

```bash
curl -s localhost:3030/context -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "traefik", "max_entities": 10}'
```

The `summary` carries an `embeddings` block reporting whether semantic
retrieval was possible at all, so an empty `entities` list is not ambiguous:

```json
"embeddings": { "configured": true, "embedded_entities": 2579 }
```

`configured: false` means no embedding provider is attached;
`embedded_entities: 0` with `configured: true` means the store was never
embedded (`quipu knot` does not embed — run a backfill). See
[Embeddings and Semantic Search](../concepts/embeddings.md).

### `POST /unified_search`

Unified knowledge search (text + optional vector); results tagged
`source="knowledge"` with normalized 0–1 scores. Body: `query`, optional
`embedding`, `limit`, `expand_links`, `max_facts_per_entity`.

### `POST /ask`

Run a curated, parameterized named query by name (see `quipu_ask` in the
[MCP tools reference](./mcp-tools.md)). Body: `name` (omit or `"list"` to list
the catalog), optional `params` map. Parameters are validated and escaped by
type. Response: `query`, resolved `sparql`, `columns`, `rows`, `count`.

```bash
curl -s localhost:3030/ask -X POST \
  -d '{"name":"service_deps","params":{"entity":"http://example.org/traefik"}}'
```

### `POST /search_nodes`

Search entities by natural-language query (text matching). Body: `query`,
optional `group_ids`, `max_results`, `entity_type_filter`.

### `POST /search_facts`

Search relationships/edges by natural-language query. Body: `query`, optional
`group_ids`, `max_results`.

### `POST /search/nodes`

Graphiti-compatible node search (mirrors Graphiti's `search_nodes` shape).

### `POST /episodes/complete`

Graphiti-compatible flat episode ingestion. Body: `name`, optional
`episode_body`, `group_id`, `source_description`, `timestamp`.

### `POST /impact`

Impact analysis: walk downstream from an entity, optionally counterfactual.
Body: `entity`, optional `remove`, `hops`, `predicates`, `timestamp`.

### `POST /path/cone`

Golden paths: the provenance cone of a trajectory — which steps did its
falsifier-gated verified result depend on? Read-only. Body: `trajectory` (the
Trajectory IRI, required), optional `via` (array of derivation predicate IRIs
walked in addition to `verifiedBy`, which is always followed), `hops` (walk
depth, default 8), `base_ns` (vocabulary namespace override; defaults to the
store's configured `base_ns`).

```bash
curl -s localhost:3030/path/cone -X POST \
  -H 'Content-Type: application/json' \
  -d '{"trajectory": "http://example.org/traj/42", "hops": 6}'
```

Returns the cone report: the trajectory, the hop bound, the verifications it
was checked against, and one entry per step carrying `iri`, `order`, `verdict`
(`InCone` / `OutOfCone` / `CannotEvaluate`) and the human-readable `reason`.
Refuses a trajectory with no steps or no falsifier-gated verification.

### `POST /path/backtest`

Golden paths: replay a pruned candidate (the exemplar trajectory minus omitted
steps) over recorded history — which past trajectories sharing a work-item
topic would have conformed under `gp-grammar/1`, and how did their work items
close? Read-only. Body: `exemplar` (the exemplar Trajectory IRI, required),
optional `omit` (array of step IRIs the candidate omits), `base_ns`.

```bash
curl -s localhost:3030/path/backtest -X POST \
  -H 'Content-Type: application/json' \
  -d '{"exemplar": "http://example.org/traj/42",
       "omit": ["http://example.org/step/3"]}'
```

Returns the backtest report: the exemplar, the grammar, the matched topics, one
row per replayed trajectory, and the conformer/deviator completion counts with
an explicit `cannot_evaluate` tally — 0 matches and "nothing measurable" are
never reported as the same thing.

### `POST /propose`

Submit a schema-evolution proposal. Body: `kind`, `target`, `diff`, `proposer`,
optional `rationale`, `trigger_ref`, `timestamp`.

### `POST /proposals`

List schema-evolution proposals. Body: optional `status`
(`pending`/`accepted`/`rejected`).

### `POST /proposal/accept`

Accept a pending proposal. Body: `id`, optional `decided_by`, `note`,
`timestamp`.

### `POST /proposal/reject`

Reject a pending proposal. Body: `id`, `note`, optional `decided_by`,
`timestamp`.

### `POST /entity_history`

Return the full fact history (across transactions) for an entity. The body field is
**`iri`**, not `entity` — `{"entity": ...}` returns
`{"error": "missing 'iri' parameter"}`.

```bash
curl -s localhost:3030/entity_history -X POST -H "Content-Type: application/json" \
  -d '{"iri": "http://example.org/svc"}'
```

Returns `{"iri", "count", "history": [{"op", "predicate", "value", "tx",
"valid_from", "valid_to"}, ...]}`. The `tx` is the handle for `/transactions` below —
together they answer "which write path asserted this fact, and who owned it".

### `GET /transactions`

List transactions in the store, oldest first.

| Param | Effect |
|---|---|
| *(none)* | the whole log |
| `since=<tx>` | only transactions **newer** than `<tx>` — the poller's cursor, so a watermarked poll is O(new) rather than O(log) |
| `limit=<n>` | clamped to `1..=10_000`; **applies from the start of the log**, not the end |

There is no `offset`. Passing `limit` alone therefore returns the *oldest* N — on a
38k-transaction store, `?limit=40000` hands back transactions 1–10000 and nothing
recent. To look up a specific transaction, use `?since=<tx-1>&limit=1`.

Each entry is `{id, timestamp, actor, source}`. `source` identifies the write path:
`episode:<name>` (`/episode`), `set` (`/set`), `retract` (`/retract`,
`/episode/retract`), or caller-supplied free text (`/knot`). `actor` and `source` are
both optional on `/knot`, and a call that omits them lands facts with **no audit
trail at all** — `{"actor": null, "source": null}`. Pass them.

### `POST /embed_backfill`

Backfill embeddings for entities that lack them. Returns
`{"status": "error", ...}` when no embedding provider is configured; the
`--embed-backfill` startup flag instead exits non-zero rather than serving
without the capability it was asked for.

### `GET /preview/{iri}`

Return a preview rendering of an entity by IRI.

## Service Metadata

### `GET /version`

What build is actually running: `{"version", "git_sha", "git_dirty",
"features"}`. The git SHA is the field that matters for "is the fix
deployed?" — a semantic version does not move when a fix lands. `features`
maps every declared Cargo feature to whether this binary compiled it in.

### `GET /metrics`

Prometheus scrape endpoint (`text/plain; version=0.0.4`). Request counters
come from the middleware; graph-size gauges cost one cheap SQL aggregate —
deliberately not `/stats`' full scan.

Caller attribution uses the normalized `X-Quipu-Client` header (falling back
to `User-Agent`, then `unattributed`) and is capped at 32 identities; overflow
folds into `other` rather than creating unbounded Prometheus cardinality:

- `quipu_http_client_requests_total{client,endpoint}` — request count;
- `quipu_http_client_request_seconds_total{client,endpoint}` — wall time;
- `quipu_store_wait_seconds_total{client,endpoint}` — time waiting to acquire a
  store connection;
- `quipu_store_held_seconds_total{client,endpoint}` — store capacity consumed.

The server also writes one-line JSON request events to stderr for journald/Loki.
`request_start` makes a request that never completes visible. `request_complete`
adds `status`, `duration_ms`, and the actual `auth_outcome`; `/query` responses
also add `query_shape` and `result_size`. Logs contain normalized attribution
and bounded metadata, never the Authorization header or response body. Slow or
failed query text retains its existing separate diagnostic line.

### UI assets (not documented individually)

`GET /` and `GET /ui` serve the built-in web UI; `GET /quipu-components.js`,
`GET /graph-canvas.js`, `GET /datalinks.js` and
`GET /vendor/three.module.min.js` serve its static assets, vendored so the UI
renders on an air-gapped deploy. They are part of the UI, not the API surface.

## Export

### `GET|HEAD|PUT|POST|DELETE /rdf-graph-store`

SPARQL 1.1 Graph Store HTTP Protocol using indirect graph identification.
Select exactly one target with `?graph=<absolute-IRI>` or `?default`. `GET`
returns the graph in the RDF syntax requested by `Accept`; `HEAD` returns the
same status and headers without a body. `PUT` replaces the graph, `POST` merges
the payload, and `DELETE` removes its contents (and a named graph's registry
entry). Writes use the same bearer-token and read-only enforcement as Quipu's
native write APIs.

```bash
curl localhost:3030/rdf-graph-store?graph=http%3A%2F%2Fexample.org%2Fg \
  -X PUT -H 'Content-Type: text/turtle' --data-binary '@graph.ttl'
curl localhost:3030/rdf-graph-store?graph=http%3A%2F%2Fexample.org%2Fg \
  -H 'Accept: application/n-triples'
```

Supported request and response syntaxes are Turtle (`text/turtle`) and
N-Triples (`application/n-triples`). Unsupported request media types return
415; unsupported response types return 406; unknown named graphs return 404.

### `POST /export`

Export deterministic RDF from ROOT, one named graph, one episode provenance
group, or a SPARQL CONSTRUCT/DESCRIBE result. `graph`, `group_id`, and
`construct` are mutually exclusive. The handler uses the read pool, so
serializing a large export does not hold Quipu's writer lock.

```bash
curl -s localhost:3030/export -X POST \
  -H "Content-Type: application/json" \
  -d '{"graph": "http://example.org/graphs/derived", "format": "turtle"}'
```

| Field | Required | Description |
|---|---|---|
| `graph` | No | Named-graph IRI (omit for ROOT; unknown IRI → 400) |
| `group_id` | No | ROOT entities attributed through `prov:wasGeneratedBy` to episodes in this group, plus those episode resources |
| `construct` | No | SPARQL CONSTRUCT or DESCRIBE query to export |
| `format` | No | `turtle` (default) or `ntriples` |

Returns the RDF document itself with the matching content-type, not JSON.
N-Triples output is lexically sorted and duplicate-free. Blank-node dataset
canonicalization belongs to the share-bundle layer; raw export preserves blank
node labels.

## Share Import and Composition

### `POST /import`

Verify a v1 share manifest and its exact `export.nt` and `shapes.ttl` payloads,
then stage the resolved triples in a per-share named graph. Exact canonical-name
matches are rewritten to local IRIs; fuzzy matches are returned as review
candidates and never merged automatically. Local loaded shapes remain the
authority: bundled shapes are evidence only. Off-vocabulary or non-conforming
data is retained in a quarantine graph and is not eligible for promotion.

The request fields are `manifest`, `export_ntriples`, `shapes_turtle`, `source`,
and optional `actor`. The response reports `staged`, `quarantined`, or
`unchanged`, the stable import and graph IRIs, accepted/quarantined counts,
resolution candidates, the SHACL report, and promotion blockers. This is an
authenticated write endpoint.

### `POST /import/promote`

Explicitly copy an eligible staging graph into ROOT. The body is
`{"share_id":"sha256:...","actor":"optional"}`. Quarantined shares have no
eligible staging graph and are refused. Importing never promotes implicitly.

## Registries

These mirror their MCP tools (see the [MCP reference](./mcp-tools.md)) —
action-style managers where an unknown `action` errors rather than falling
through to `list`.

### `POST /ontology`

Manage OWL ontologies: `{"action": "load"|"list"|"remove", "name",
"turtle", "timestamp"}` (mirrors `quipu_load_ontology`). Registered even
without the `owl` feature — a build without it answers with an explicit
error naming the missing feature rather than a 404, so "not compiled in"
and "no such route" stay distinguishable.

### `POST /subscriptions`

Event-push subscription registry: `{"action": "create"|"list"|"delete", ...}`
— register an HTTP endpoint to receive graph-change events pushed by the
server's delivery worker (mirrors `quipu_subscriptions`).

### `POST /datasets`

Named dataset registry: `{"action": ..., "name", "members", ...}` — declare a
named set of graphs queryable as one unit via `FROM <dataset>` or the `graph`
query param (mirrors `quipu_datasets`).

### `POST /queries`

Stored named-query registry: `{"action": "load"|"list"|"get"|"remove",
"name", "template", "params", ...}` — competency questions callable through
`/ask` alongside the compiled-in catalog; definitions are validated at load
and versioned (mirrors `quipu_queries`).

## Governance

The REST half of the governance gate; each mirrors its MCP tool, where the
semantics are documented in full.

### `POST /policy/check`

Committed-tier evaluation of a governance Policy against a target: returns a
Verdict — `outcome` ∈ `satisfied | unsatisfied | unknown` bound to a
reproducible `evidence_hash` — signed when the store has a signing identity.

```bash
curl -s localhost:3030/policy/check -X POST \
  -H "Content-Type: application/json" \
  -d '{"policy": "http://example.org/policy/has-owner", "target": "http://example.org/svc"}'
```

| Field | Required | Description |
|---|---|---|
| `policy` | One of policy/claim | Policy IRI whose `aegis:claim` to evaluate |
| `claim` | One of policy/claim | Inline SPARQL ASK |
| `target` | Yes | Target IRI bound to `$target` |
| `predicate_id` | No | Recorded predicate id for inline claims |
| `evidence_probe` | No | ASK for "does the evidence exist?" → `unknown` |
| `valid_at` | No | ISO-8601 point-in-time |

### `POST /verifier/authorized`

`{"verifier", "predicate"}` → `{"authorized": bool}`: may this verifier
attest this predicate, per the Phase-0 verifier registry?

### `POST /verdict/verify`

Verify a signed Verdict against the Phase-0 root of trust:
`{"predicate_id", "target_ref", "outcome", "evidence_hash", "tier"?,
"verifier", "signature"}` → `{"signature_valid", "verifier_registered",
"verifier_authorized", "trusted"}` — `trusted` is the conjunction to gate on.

## Overlays

Scratch layers over the committed graph (bind-once to a parent branch):
hypotheses go in the overlay, the committed layer stays untouched. Mirror
the `quipu_overlay_*` MCP tools.

### `POST /overlay/create`

`{"overlay": "<iri>", "parent_branch": "<iri>"?}` → `{"g", "parent_branch"}`.
Omitted `parent_branch` binds to ROOT.

### `POST /overlay/write`

`{"overlay", "op": "assert"|"retract"|"tombstone", "subject", "predicate",
"object", "timestamp"?}` → `{"tx_id"}`. `tombstone` masks the parent's fact
in the composed view.

### `POST /overlay/compose`

`{"overlay": "<iri>"}` → `{"triples": [{subject, predicate, object}],
"count"}`: the resolved view over `[overlay > parent-branch-root]`,
asserted-and-not-tombstoned, nearest wins.

## Provenance Analytics

### `POST /cooccurrence`

`{"work_item": "<iri>", "valid_at"?, "tx"?}` → the other work-items sharing
at least one touched code entity, via `Bead ←implements− GitCommit
−modifies→ entity`, ordered by overlap strength (mirrors
`quipu_cooccurrence`).

## Events

The durable graph-change event log (at-least-once delivery; consumers dedup
by offset).

### `GET /events`

Pull a batch of events in offset order.

| Param | Effect |
|---|---|
| `since=<offset>` | start after this offset |
| `consumer=<id>` | omit `since` to resume from this consumer's committed offset |
| `limit=<n>` | batch size, clamped to `1..=10_000` (default 100) |
| `types=<a,b>` | filter by event type |
| `group=<g>` | filter by provenance group |

Returns `{events, next_offset, lag, committed_offset?}`; pass `next_offset`
back as `since` (or commit it) to page forward — polling is a fixpoint, not a
rewind.

### `POST /events/commit`

`{"consumer_id", "offset"}` — durably record a consumer's cursor. Any offset
≥ 0 is accepted, including a **lower** one: that is the explicit replay knob.

### Refusal events (`write.refused`)

A refused write never enters the graph, so the event log is where the attempt
is recorded (camayoc-0d3 — the incident-rate denominator: how many writes were
attempted and refused, by which gate). Every write-gate refusal — SHACL on the
episode and `/knot` paths, and the policy, authority, OWL and placement gates
in `transact` — appends a `write.refused` event *after* the refused write's
savepoint has rolled back, so the event survives the rollback that the refusal
caused. A failure to record never masks the refusal error itself.

Payload: `{gate, graph, actor, source, reason, refused_datums}` where `gate`
is one of `shacl | policy | authority | owl | placement`, `graph` is the
destination graph IRI, `reason` is the gate's own terse text (shape/policy id,
constraint name; truncated), and `refused_datums` counts what was refused.

Deliberately **not** recorded: the refused datum bodies. Refused payloads can
be junk or sensitive — the event stores identifying metadata only.

Refusals inside `speculate` (counterfactual writes) are **not** recorded: the
whole speculation rolls back by design, so a hypothetical write's refusal is
not a real one.

Query via `GET /events?types=write.refused`, or count by gate with the CLI:
`quipu events refusals`.

## Linked-Data Surface

Standards-flavoured read endpoints for semantic-web tooling.

### `GET /changes`

Returns fact-level change records after the optional `since` transaction.
`capture` selects `new_values`, `old_and_new_values`, or `new_row`; `graph`
optionally scopes the feed to one graph IRI. The response includes `next_tx`
and a watermark so consumers can distinguish an idle feed from a stalled one.

### `GET /entity/{iri}`

Content-negotiated entity page: `Accept: application/ld+json` → JSON-LD,
`text/turtle` → Turtle, anything else → the web UI's HTML page for the
entity. `GET /entity/{iri}/json`, `GET /entity/{iri}/ttl` and
`GET /entity/{iri}/html` pin the format in the path instead of the header.
For a full IRI containing path separators or a fragment, use the equivalent
query form: `GET /entity?iri=https%3A%2F%2Fexample.org%2Fresource%231`.

### `POST /spotlight`

DBpedia-Spotlight-style annotation: `{"text", "confidence"?}` → mentions of
known entities found in the text, with offsets and IRIs. The labeled-entity
list it scans against is generation-cached, so a burst pays the expensive
fetch once.

### `GET /fragments`

Triple Pattern Fragments: `?subject=&predicate=&object=&page=&pageSize=`
selectors, each optional — a paged triple-pattern read for TPF clients.

### `POST /reconcile`

OpenRefine Reconciliation API: a body without `queries` returns the service
manifest; `{"queries": {...}}` runs the batch and returns candidates per
query, scored the way `/resolve` scores.
