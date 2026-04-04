# Configuration

Quipu is configured via `.bobbin/config.toml` in your project directory
or `~/.config/bobbin/config.toml` for global defaults.

## Config File

```toml
[quipu]
# Path to the SQLite triple store
store_path = ".bobbin/quipu/quipu.db"

# Directory containing OWL/SHACL schema files (optional)
schema_path = "schemas/"

[quipu.server]
# Enable the REST API server
enabled = false
# Bind address
bind = "127.0.0.1:3030"

# Federation: connect to remote Quipu instances
[[quipu.federation.remotes]]
name = "prod"
url = "http://quipu.svc:3030"
```

## Config Fields

| Field | Default | Description |
|-------|---------|-------------|
| `store_path` | `.bobbin/quipu/quipu.db` | SQLite database path |
| `schema_path` | None | Directory for schema files |
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
