# Federation

Quipu supports federated queries across multiple graph providers through
the `GraphProvider` trait. This allows agents to query a local store and
remote Quipu instances in a single operation.

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

## Configuration

Remote endpoints are configured in `.bobbin/config.toml`:

```toml
[quipu]
store_path = ".bobbin/quipu/quipu.db"

[[quipu.federation.remotes]]
name = "prod"
url = "http://quipu.svc:3030"

[[quipu.federation.remotes]]
name = "staging"
url = "http://quipu-staging.svc:3030"
```

## Result Tagging

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
