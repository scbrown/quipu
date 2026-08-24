# Configuration

Quipu is configured via `.bobbin/config.toml` in your project directory
or `~/.config/bobbin/config.toml` for global defaults.

## Config File

```toml
[quipu]
# Path to the SQLite triple store
store_path = ".bobbin/quipu/quipu.db"

# Base namespace new IRIs are minted under (optional; default is the aegis
# ontology namespace). A non-aegis deployment MUST set this before its first
# write — an IRI namespace is data identity and cannot be changed afterwards
# without re-ingesting every episode.
base_ns = "http://example.org/kb/"

[quipu.server]
# Enable the REST API server
enabled = false
# Bind address
bind = "127.0.0.1:3030"

[quipu.events]
# Event-log retention. Unset (the default) keeps every event forever.
# When set, the server hourly deletes events older than this many days —
# but never an event a registered consumer has not yet committed past, so
# a lagging consumer's replay is never broken (its backlog just stays on
# disk). A consumer registering AFTER a prune replays from the retained
# prefix, not from genesis.
# retention_days = 90
```

## Config Fields

Every key below is wired — `src/config.rs` carries a test
(`config_knobs_are_wired_or_listed_unwired`) that fails if a documented knob
stops being read.

| Field | Default | Description |
|-------|---------|-------------|
| `store_path` | `.bobbin/quipu/quipu.db` | SQLite database path |
| `base_ns` | aegis ontology NS | Base namespace for minted IRIs (set before first write; `--base-ns` overrides per CLI call) |
| `server.enabled` | `false` | Enable REST API server |
| `server.bind` | `127.0.0.1:3030` | Server bind address |
| `server.auth_token` | unset | Bearer token required on write endpoints when set |
| `server.read_only` | `false` | Refuse all write endpoints |
| `server.cors_allowed_origins` | `[]` | CORS allowlist for the UI/API |
| `server.read_pool_size` | `4` | Read-only connection pool size (0 = all reads take the writer lock) |
| `events.retention_days` | unset (keep forever) | Prune events older than N days, never past any registered consumer's committed offset |
| `labels.min_freshness` | unset | Graph-label floor: refuse results staler than this |
| `labels.min_trust_rank` / `labels.min_trust_chain` | unset | Trust floors on the query path |
| `labels.deny_policy_tokens` | `[]` | Policy-class tokens that exclude a graph from results |
| `labels.deny_data_kinds` | `[]` | Refuse queries composing graphs of these `dataKind` tokens (a blocklist — undeclared kinds pass) |
| `search.default_limit` | `10` | Result limit when the caller passes none |
| `search.max_limit` | `1000` | Hard cap on requested result limits |
| `search.max_sparql_rows` | `10000` | Cap on SPARQL result rows |
| `search.query_timeout_ms` | `30000` | SPARQL evaluation deadline |
| `search.max_join_rows` | `1000000` | Abort a join once an intermediate exceeds this |
| `search.oversample_factor` | `10` | Vector-search oversampling before filtering |
| `shacl.validate_on_write` | `false` | Validate episode ingest against the stored shapes |
| `owl.validate_on_write` | `false` | Enforce `owl:disjointWith` / `owl:FunctionalProperty` at write time (with functional-property supersede) |
| `governance.enforce_on_write` | `false` | Evaluate action-boundary policies against every write (the write-time gate) |
| `governance.validate_placement` | `false` | Check SARC class↔placement rules when a write defines/amends a policy |
| `governance.enforce_authority` | `false` | Make a supplied principal chain binding for graph writes |
| `resolution.enabled` | `false` | Entity resolution (dedup) on the episode write path |
| `resolution.threshold` / `top_k` / `strict_mode` | `0.85` / `3` / `false` | Match threshold, candidate count, refuse-on-ambiguity |
| `embedding.auto_embed` | `false` | Auto-embed entities on write (needs model/tokenizer paths) |
| `embedding.model_path` / `tokenizer_path` | unset | ONNX model + tokenizer for embeddings |
| `embedding.dimension` / `max_sequence_length` / `embed_batch_size` | `384` / `256` / `32` | Embedding runtime parameters |
| `vector.backend` | `sqlite` | `sqlite` or `lancedb` (embedder-only; see below) |
| `federation.remotes` | `[]` | Remote quipu endpoints (`{name, url, auth_token?, timeout_ms?}`); health-checked at startup, queried via `federated: true` |

## Not wired into the `quipu` CLI / `quipu-server`

These keys parse but the shipped binaries do **not** act on them — they exist for
embedders that drive quipu as a library, or are planned. The binaries print a
`warning:` if you set them, rather than accepting them silently:

| Field | Status |
|-------|--------|
| `vector.backend = "lancedb"` | **Embedder-only.** The CLI/server never install a non-SQLite backend; queries always use the SQLite vectors table. A host embedding quipu can install one via `Store::set_local_vector_backend`. |

(`federation.remotes` used to sit here; it is fully wired now — health-checked
at startup and queried per-request via `federated: true` on `POST /query`. See
[Federation](../architecture/federation.md).)

## Priority Order

Configuration is resolved in this order (highest priority first):

1. **CLI flags** (`--db`, `--bind`)
2. **Project config** (`.bobbin/config.toml` in working directory)
3. **Global config** (`~/.config/bobbin/config.toml`)
4. **Built-in defaults**

## CLI Overrides

CLI flags always take precedence:

```bash
quipu read "SELECT ..." --db /tmp/test.db    # Overrides store_path
quipu-server --bind 0.0.0.0:8080             # Overrides server.bind
```
