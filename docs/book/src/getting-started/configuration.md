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
```

## Config Fields

| Field | Default | Description |
|-------|---------|-------------|
| `store_path` | `.bobbin/quipu/quipu.db` | SQLite database path |
| `base_ns` | aegis ontology NS | Base namespace for minted IRIs (set before first write; `--base-ns` overrides per CLI call) |
| `server.enabled` | `false` | Enable REST API server |
| `server.bind` | `127.0.0.1:3030` | Server bind address |

## Not wired into the `quipu` CLI / `quipu-server`

These keys parse but the shipped binaries do **not** act on them — they exist for
embedders that drive quipu as a library, or are planned. The binaries print a
`warning:` if you set them, rather than accepting them silently:

| Field | Status |
|-------|--------|
| `federation.remotes` | **Unimplemented.** There is no remote `GraphProvider`; remotes are ignored. See [Federation](../architecture/federation.md). |
| `vector.backend = "lancedb"` | **Embedder-only.** The CLI/server never install a non-SQLite backend; queries always use the SQLite vectors table. A host embedding quipu can install one via `Store::set_local_vector_backend`. |

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
