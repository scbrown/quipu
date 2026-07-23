# Federation

> **Implementation status (2026-07-23, weaver):** 🟡 **Partial.** Built &
> tested: the `GraphProvider` trait, `ProviderStatus`, `LocalProvider`, and
> `FederatedProvider` (`query_all` with `_provider` tagging, `health_all`) in
> `src/provider.rs`; the `[[quipu.federation.remotes]]` config schema parses in
> `src/config.rs` and **warns loudly when set** (federation is unimplemented —
> config.rs, with a test). **Gap:** no `RemoteProvider`, so the local+remote
> headline is not real and `federation.remotes` is inert — tracked in
> [quipu#47](https://github.com/scbrown/quipu/issues/47). Verified by grep
> against `main` (fcf75c2). See the detailed banner below.
>
> **Status: trait-only, not wired.** The `GraphProvider` trait and
> `FederatedProvider` exist as library primitives, but quipu ships **no remote
> provider** — only `LocalProvider`. Nothing constructs a `FederatedProvider` from
> config, and the `[[quipu.federation.remotes]]` keys below are **read by
> nothing**: setting them does not federate anything, and the `quipu`/`quipu-server`
> binaries print a `warning:` if you do. Everything past this banner describes the
> trait surface an embedder could build on, and the *intended* result shape — not a
> capability the shipped binaries provide. "Query local and remote instances in a
> single operation" is not yet true; the remote half does not exist.

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

// Query all providers, results tagged with _provider field
let result = federation.query_all("SELECT ?s ?p ?o WHERE { ?s ?p ?o }").unwrap();

// Health check all
let statuses = federation.health_all();
for s in &statuses {
    println!("{}: healthy={}, facts={:?}", s.name, s.healthy, s.fact_count);
}
```

## Configuration (planned — currently inert)

> **These keys are read by nothing.** They are shown as the *intended*
> shape for when a remote provider exists. Today, writing them into
> `.bobbin/config.toml` parses and does nothing, and the binaries warn. Do not rely
> on them.

The intended form:

```toml
[quipu]
store_path = ".bobbin/quipu/quipu.db"

# NOT YET IMPLEMENTED — remotes are ignored.
[[quipu.federation.remotes]]
name = "prod"
url = "http://quipu.example:3030"
```

## Result Tagging (intended)

Federated query results include a `_provider` field so you can tell
which source each result came from:

```json
{
  "rows": [
    { "s": "ex:traefik", "p": "ex:port", "o": "443", "_provider": "local" },
    { "s": "ex:nginx", "p": "ex:port", "o": "80", "_provider": "prod" }
  ]
}
```
