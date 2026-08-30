# Federation

Federation and Git-native shares are complementary boundaries. The
`[[quipu.federation.remotes]]` configuration drives live read fan-out through
`federated_from_config`; it does not silently publish local facts or bypass the
share scrub gate. Durable exchange uses a canonical share followed by explicit
import, quarantine, and promotion. This keeps remote availability and local
publication policy independent: adding a read peer cannot turn it into an
outbound replication target.

> **Implementation status (2026-08-25):** ✅ **Built.** In `src/provider/`
> (with tests): the `GraphProvider` trait, `ProviderStatus`, `LocalProvider`,
> `FederatedProvider` with outcome-reporting `query_all`, and — behind the
> `remote` feature — `RemoteProvider` plus `federated_from_config()`
> (re-exported from `lib.rs`). `quipu-server` health-checks every configured
> remote at startup, and `POST /query` with `"federated": true` fans the query
> out through the federated provider per request (quipu-tkh). Since quipu-fd1,
> remotes carry an operator-**declared** trust/freshness label
> (`src/provider/label.rs`) and configured `[quipu.labels]` floors refuse a
> federated result exactly as a local one. See
> `docs/design/federation-remote-provider.md`.

Quipu defines federated queries across multiple graph providers through
the `GraphProvider` trait, so that a host embedding quipu can query a local store
and its own remote providers in a single operation.

## The GraphProvider Trait

```rust
pub trait GraphProvider {
    fn name(&self) -> &str;
    fn query(&self, sparql: &str) -> Result<QueryResult>;
    fn entities(&self, type_filter: Option<&str>, limit: usize) -> Result<JsonValue>;
    fn health(&self) -> ProviderStatus;
}
```

Any data source that implements this trait can participate in federated
queries.

## Built-in Providers

### LocalProvider

Wraps a local Quipu `Store`:

```rust
use quipu::provider::LocalProvider;

let provider = LocalProvider::new(&store, "local");
let result = provider.query("SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5").unwrap();
```

### FederatedProvider

Aggregates multiple providers and merges their results:

```rust
use quipu::provider::{FederatedProvider, LocalProvider};

let mut federation = FederatedProvider::new();
federation.add(Box::new(LocalProvider::new(&store, "local")));
// Add remote providers as they become available

// Query all providers; the outcome reports who answered.
let fq = federation.query_all("SELECT ?s ?p ?o WHERE { ?s ?p ?o }");
assert!(fq.complete, "every member contributed: {:?}", fq.providers);

// Health check all
let statuses = federation.health_all();
for s in &statuses {
    println!("{}: healthy={}, facts={:?}", s.name, s.healthy, s.fact_count);
}
```

`query_all` never aborts because one member is down — a dead peer must not
deny the whole result — but it never *hides* it either: the returned
`FederatedQuery` carries the merged rows plus a `ProviderOutcome` per member
(row count, or the failure reason), and `complete` is the one-field answer to
"can I trust this result set as exhaustive?". A member that errors, answers a
non-SELECT shape, or disagrees on the variable list is a reported failure,
never a silent merge.

### RemoteProvider

Behind the `remote` feature (a default of the shipped binaries): another
`quipu-server`, reached over its REST API — `POST /query`, `POST /cord`, and
`GET /stats` as the health probe.

## Configuration

```toml
[[quipu.federation.remotes]]
name        = "prod"
url         = "http://quipu.example:3030"
auth_token  = "…"     # optional; sent as `Authorization: Bearer …`
timeout_ms  = 5000    # optional; default 5000

# The label this remote's rows carry, DECLARED by you, the local operator
# (quipu-fd1). Never read from the remote itself — a remote asserting its own
# trustworthiness would defeat the trust boundary. All optional; trust needs
# all three fields (a rank means nothing outside its chain) and a partial
# declaration is refused at startup and on every federated query.
trust       = "urn:trust:partner"
trust_chain = "https://quipu.dev/ontology/defaultTrustChain"
trust_rank  = 30
freshness   = "fresh"  # fresh | recomputing | stale
```

`quipu-server` builds the federated provider from these at startup and
health-checks every remote (reported on stderr, with each remote's declared
label — or `undeclared`), so a dead peer, a wrong token, or a missing label
is visible without waiting for a federated query to be issued.

## Trust labels at the federation edge

A remote's rows enter your composed result set, so they must enter your label
lattice — and the label is **declared by the local operator**, never inferred
and never read from the remote (the SARC trust boundary, surfaced at the
federation edge — see `docs/design/multi-db-composition.md` §5).

- **Rows are stamped.** Beside `_provider`, rows from a declared remote carry
  `_trust` (the trust IRI; rank and chain ride the per-member `providers`
  entry) and `_freshness`. Rows from an undeclared member simply lack the
  binding — undeclared is absent, never fabricated.
- **`ProviderStatus` carries the label.** Health reports (startup stderr, and
  the `label` field on each `providers` entry) show what each member's rows
  are declared as; `null`/`undeclared` means exactly that.
- **The composed label folds remotes in as members.** The federated response's
  `labels` key is the local dataset fold with each remote's declared label
  met in — trust and freshness by meet, so composition never widens; the axes
  a remote cannot declare (durability, policy, kind) degrade coverage to
  `partial`.
- **Configured floors apply.** With `[quipu.labels]` floors set, a federated
  query is refused when a local member fails the floor (same check as the
  local path) or when a remote's declared label is below it — and the refusal
  names the remote. **An undeclared remote fails a configured freshness or
  trust floor**, exactly as an unlabelled local graph does: fail-safe at
  enforcement, honest at reporting. With no floor configured, nothing changes.

## Federated queries over REST

`POST /query` with `"federated": true` fans the whole query text out to the
local store and every configured remote:

```json
{ "query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "federated": true }
```

The response carries the merged rows — each tagged with a `_provider` field —
plus the per-member account:

```json
{
  "variables": ["s", "p", "o", "_provider", "_trust", "_freshness"],
  "rows": [
    { "s": "ex:traefik", "p": "ex:port", "o": "443", "_provider": "local" },
    { "s": "ex:nginx", "p": "ex:port", "o": "80", "_provider": "prod",
      "_trust": "urn:trust:partner", "_freshness": "fresh" }
  ],
  "count": 2,
  "providers": [
    { "name": "local", "ok": true, "rows": 1, "error": null, "label": null },
    { "name": "prod", "ok": true, "rows": 1, "error": null,
      "label": { "trust": { "iri": "urn:trust:partner",
                            "chain": "https://quipu.dev/ontology/defaultTrustChain",
                            "rank": 30 },
                 "freshness": "fresh" } }
  ],
  "complete": true,
  "labels": null
}
```

`_trust`/`_freshness` columns appear only when at least one member declares
that axis; `labels` is the composed dataset label (local members' fold with
every remote met in), `null` when nothing local or remote declared anything.

Whole-query federation only: every member gets the same query text and the
results are unioned, not joined across members. The temporal/graph parameters
(`valid_at`, `tx`, `graph`, `row_labels`) shape the *local* evaluator's
context and are refused on a federated query rather than silently meaning
something different per member. SPARQL 1.1 `SERVICE` and write federation are
deliberately out of scope (design §7).
