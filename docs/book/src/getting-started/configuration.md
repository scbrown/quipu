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

# Federation: connect to remote Quipu instances
[[quipu.federation.remotes]]
name = "prod"
url = "http://quipu.example:3030"
```

## Config Fields

| Field | Default | Description |
|-------|---------|-------------|
| `store_path` | `.bobbin/quipu/quipu.db` | SQLite database path |
| `base_ns` | aegis ontology NS | Base namespace for minted IRIs (set before first write; `--base-ns` overrides per CLI call) |
| `server.enabled` | `false` | Enable REST API server |
| `server.bind` | `127.0.0.1:3030` | Server bind address |
| `federation.remotes` | `[]` | Remote Quipu endpoints |

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
