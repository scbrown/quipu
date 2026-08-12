# Federation

> **Implementation status (2026-08-12):** 🟡 **Partial — remote provider
> built, health-checked at startup, not yet on the query path.** Built &
> tested in `src/provider.rs`: the `GraphProvider` trait, `ProviderStatus`,
> `LocalProvider`, `FederatedProvider`, and — behind the `remote` feature —
> `RemoteProvider` plus `federated_from_config()`. `quipu-server` constructs
> the federated provider from `[[quipu.federation.remotes]]` at startup and
> health-checks every remote (the config key is consumed, and the old
> "federation is unimplemented" warning is gone — `src/config.rs` now tests
> that it must NOT warn). **Gap:** no query route dispatches through the
> federated provider yet — a remote's facts are reachable to an embedder
> calling `query_all`, not to a `quipu-server` client — and
> `RemoteProvider`/`federated_from_config` are not re-exported from `lib.rs`.
> See `docs/design/federation-remote-provider.md`.

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
