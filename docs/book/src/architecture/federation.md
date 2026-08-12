# Federation

> **Implementation status (2026-08-12):** ✅ **Built.** In `src/provider/`
> (with tests): the `GraphProvider` trait, `ProviderStatus`, `LocalProvider`,
> `FederatedProvider` with outcome-reporting `query_all`, and — behind the
> `remote` feature — `RemoteProvider` plus `federated_from_config()`
> (re-exported from `lib.rs`). `quipu-server` health-checks every configured
> remote at startup, and `POST /query` with `"federated": true` fans the query
> out through the federated provider per request (quipu-tkh). See
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
name       = "prod"
url        = "http://quipu.example:3030"
auth_token = "…"     # optional; sent as `Authorization: Bearer …`
timeout_ms = 5000    # optional; default 5000
```

`quipu-server` builds the federated provider from these at startup and
health-checks every remote (reported on stderr), so a dead peer or a wrong
token is visible without waiting for a federated query to be issued.

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
  "variables": ["s", "p", "o", "_provider"],
  "rows": [
    { "s": "ex:traefik", "p": "ex:port", "o": "443", "_provider": "local" },
    { "s": "ex:nginx", "p": "ex:port", "o": "80", "_provider": "prod" }
  ],
  "count": 2,
  "providers": [
    { "name": "local", "ok": true, "rows": 1, "error": null },
    { "name": "prod", "ok": true, "rows": 1, "error": null }
  ],
  "complete": true
}
```

Whole-query federation only: every member gets the same query text and the
results are unioned, not joined across members. The temporal/graph parameters
(`valid_at`, `tx`, `graph`, `row_labels`) shape the *local* evaluator's
context and are refused on a federated query rather than silently meaning
something different per member. SPARQL 1.1 `SERVICE` and write federation are
deliberately out of scope (design §7).
