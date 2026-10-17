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
# Require this bearer on write endpoints.
# auth_token = "current-token"
# During a credential rotation only, accept the old bearer for at most 24h.
# Use an absolute deadline so restarting cannot renew the old bearer.
# previous_auth_token = "old-token"
# previous_auth_token_expires_at_epoch_secs = 2000000000

[quipu.events]
# Event-log retention. Unset (the default) keeps every event forever.
# When set, the server hourly deletes events older than this many days —
# but never an event a registered consumer has not yet committed past, so
# a lagging consumer's replay is never broken (its backlog just stays on
# disk). A consumer registering AFTER a prune replays from the retained
# prefix, not from genesis.
# retention_days = 90

# Read-only databases mounted alongside the store. Each block is one layer:
# a knowledge pack, a shared reference database, a frozen archive someone
# handed you. Their named graphs become readable by `GRAPH <iri>` / `FROM <iri>`
# without changing what any existing query returns.
# [[quipu.attachments]]
# alias = "reference"
# path = "/srv/quipu/reference.pendant.db"
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
| `server.previous_auth_token` | unset | Previous write bearer accepted temporarily during rotation; requires `server.auth_token` and a positive grace duration |
| `server.previous_auth_token_expires_at_epoch_secs` | unset | Absolute UTC Unix epoch expiry for the previous bearer; at most 24 hours away when starting; expired previous bearers are ignored |
| `server.read_only` | `false` | Refuse all write endpoints |
| `server.cors_allowed_origins` | `[]` | CORS allowlist for the UI/API |
| `server.read_pool_size` | `4` | Read-only connection pool size (0 = all reads take the writer lock) |
| `server.update_full_copy_max_facts` | `250000` | Facts an unsliceable `/update` (open subject AND open predicate) may copy before it is refused with advice; 0 = unbounded |
| `events.retention_days` | unset (keep forever) | Prune events older than N days, never past any registered consumer's committed offset |
| `labels.min_freshness` | unset | Graph-label floor: refuse results staler than this |
| `labels.min_trust_rank` / `labels.min_trust_chain` | unset | Trust floors on the query path |
| `labels.deny_policy_tokens` | `[]` | Policy-class tokens that exclude a graph from results |
| `labels.deny_data_kinds` | `[]` | Refuse queries composing graphs of these `dataKind` tokens (a blocklist — undeclared kinds pass) |
| `search.default_limit` | `10` | Result limit when the caller passes none |
| `search.keyword` | `false` | Opt-in SQLite FTS5 keyword search; activation installs schema/triggers without a bulk startup migration. Existing facts require explicit bounded backfill; reads refuse until complete. |
| `search.hybrid` | `false` | Enable lexical/vector fusion after offline evaluation; no startup backfill. |
| `search.mode` | `"semantic"` | Default mode for requests omitting it: semantic, keyword or hybrid. |
| `search.alpha` | `0.5` | Semantic contribution in hybrid mode; 0 and 1 use exact pure endpoints. |
| `search.fusion` | `"rrf"` | Weighted reciprocal ranks or `"weighted"` min-max scores. |
| `search.rrf_k` | `60.0` | Positive RRF denominator offset. |
| `search.named_graphs` | `false` | Explicit graph scope for semantic and hybrid search, and automatic named-only embedding text. Keyword search accepts an explicit scope without it. Prepare existing named-only vectors with bounded backfill before enabling. Disabling retains the ROOT exclusion for prepared vectors. |
| `search.max_limit` | `1000` | Hard cap on requested result limits |
| `search.max_sparql_rows` | `10000` | Cap on SPARQL result rows |
| `search.query_timeout_ms` | `30000` | SPARQL evaluation deadline |
| `search.max_join_rows` | `1000000` | Abort a join once an intermediate exceeds this |
| `search.oversample_factor` | `10` | Vector-search oversampling before filtering |
| `shacl.validate_on_write` | `false` | Validate episode ingest against the stored shapes |
| `owl.validate_on_write` | `true` | Enforce `owl:disjointWith` / `owl:FunctionalProperty` at write time (with functional-property supersede); set false for an explicitly informal deployment |
| `governance.enforce_on_write` | `false` | Evaluate action-boundary policies against every write (the write-time gate) |
| `governance.validate_placement` | `false` | Check SARC class↔placement rules when a write defines/amends a policy |
| `governance.verify_transitions` | `false` | Refuse a write landing an `aegis:TransitionEvent` whose signature is missing or does not verify under a registered key |
| `governance.enforce_authority` | `false` | Make a supplied principal chain binding for graph writes |
| `governance.hardware_verdict_schemes` | `false` | Accept WebAuthn and SSHSIG `sk-ssh-ed25519` verdict signatures beside ed25519; while off, those verdicts and registrations declaring them are refused |
| `governance.quarantine.enabled` | `true` | Record a sealed, digest-only quarantine entry for each verdict of a refused write, outside the governed graph, so `quipu audit replay <verdict>` can re-derive the refusal |
| `governance.quarantine.retain_full` | `[]` | Graph IRIs (or `"*"`) whose refused attempts are also kept in full, sealed; purge with `quipu audit quarantine purge` |
| `resolution.enabled` | `false` | Entity resolution (dedup) on the episode write path |
| `resolution.threshold` / `top_k` / `strict_mode` | `0.85` / `3` / `false` | Match threshold, candidate count, refuse-on-ambiguity |
| `embedding.auto_embed` | `false` | Auto-embed entities on write (needs model/tokenizer paths) |
| `embedding.model_path` / `tokenizer_path` | unset | ONNX model + tokenizer for embeddings |
| `embedding.dimension` / `max_sequence_length` / `embed_batch_size` | `384` / `256` / `32` | Embedding runtime parameters |
| `vector.backend` | `sqlite` | `sqlite` or `lancedb`; selects the store's vector backend at open (see below) |
| `federation.remotes` | `[]` | Remote quipu endpoints (`{name, url, auth_token?, timeout_ms?}`); health-checked at startup, queried via `federated: true` |
| `attachments` | `[]` | Read-only databases mounted alongside the store (`[[quipu.attachments]]` with `alias` and `path`); see below |

## Zero-downtime bearer rotation

Quipu captures bearer configuration at startup. To rotate without a write
freeze, configure the new value as `server.auth_token`, the old value as
`server.previous_auth_token`, and a short
`server.previous_auth_token_expires_at_epoch_secs`, then restart `quipu-server`. Both
bearers authenticate writes until the grace deadline; request telemetry records
`authenticated_current` or `authenticated_previous` without recording either
secret.

After consumers have switched, remove both `previous_auth_token` fields and
restart again. That explicitly invalidates the old bearer immediately. If the
cleanup restart is delayed, the old bearer still expires at the configured absolute
deadline, and a restart never extends it. On restart, an expired previous bearer is dropped with a warning; the current
bearer remains required and valid. Forgotten cleanup cannot cause a delayed
startup outage. Invalid pairs, identical bearers, and deadlines over
24 hours away still make startup fail closed.

## Attachments

`[[quipu.attachments]]` mounts other SQLite databases alongside your store, so
their named graphs are readable in the same query as your own. Both binaries
honour it at open — every `quipu` subcommand and `quipu-server` alike.

```toml
[[quipu.attachments]]
alias = "reference"
path = "/srv/quipu/reference.pendant.db"

[[quipu.attachments]]
alias = "tenant_a"
path = "packs/tenant-a.db"
```

| Key | Required | Description |
|-----|----------|-------------|
| `alias` | Yes | SQLite schema name the file mounts under, and the `source` its contributed graphs carry in the graph registry. Must match `^[a-z][a-z0-9_]*$` |
| `path` | Yes | Path to the database file; relative paths resolve against the working directory |

What to expect:

- **Nothing silently changes.** The default dataset stays your own ROOT alone,
  so an attachment is visible only to a query that names one of its graphs —
  `GRAPH <iri>`, `FROM <iri>`, or a dataset that includes it.
- **Mounts are read-only, always.** There is no `read_only = false`:
  cross-database writes are permanently out of scope, so the key would be an
  affordance for something the store refuses anyway.
- **A bad declaration refuses the open** — it never degrades into fewer rows. A
  missing file, an invalid or duplicate alias, a file with no `g` column (it
  predates named graphs), or one whose term space collides with yours all fail
  at startup, naming the alias and the remedy. A term-space collision is fixed
  with `quipu db respace`.
- **The packs you already build are attachable.** `quipu pack --space N` ships a
  pack in its own term space, which is exactly what a collision-free mount
  needs.

See what is actually mounted — including deep freeze's archives, which no
config declares:

```bash
quipu db attach --list --db my.db
```

`quipu-server` prints the same list to stderr at startup.

## Choosing a vector backend

`[quipu.vector] backend` selects the store's vector backend at open, for both
binaries: search, entity resolution, auto-embedding and the MCP/REST search
tools all go through the selected one.

```toml
[quipu.vector]
backend = "lancedb"
lancedb_path = ".bobbin/quipu/quipu-vectors"
```

**`lancedb` requires a binary built with the `lancedb` feature, and the release
binaries are not.** The feature drags in protoc and the whole datafusion tree,
which most deployments do not need. A binary that lacks it **refuses**
`backend = "lancedb"` at startup and names the rebuild — it does not fall back
to the SQLite table, because a deployment that has run `quipu migrate-vectors`
would then have every search answered out of the store it migrated away from.

Move existing embeddings across with
`quipu migrate-vectors --from sqlite --to lancedb`. See
[LanceDB Vector Backend](../architecture/lancedb.md).

## Configuration coverage

Every documented key above is read by the shipped binaries. `unwired_warnings()`
reports any configured key that parses but is not acted on, so an inert setting
produces a warning.

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

## Read-only follower serving

A file-level replication operator can serve a changing follower with:

```toml
[quipu.server.replica]
primary_url = "https://primary.example"
```

Start `quipu-server --read-only --db /path/to/follower.db` from that configuration
directory. Keep `--read-only` in the service invocation: it refuses startup if
neither config nor `--primary-url URL` supplies a valid primary, even if the
config file disappears. `--primary-url URL` overrides the configured primary.
This is an opt-in serving role, separate from the older `server.read_only`
HTTP-only access-control flag. The default primary serving role is unchanged. Server startup now refuses invalid
or unreadable existing configuration instead of silently falling back to defaults.
Replica mode opens **only existing SQLite databases read-only**, with no schema
setup or migration. Every read connection explicitly waits up to one second for
short apply locks. It never falls back to opening a writable database. SQLite
may create WAL/SHM sidecars; the directory must support the follower's locking
protocol. Do not use `immutable=1` for a database that replication updates.

The replica does not initialize a keyword index, materialize OWL, register write
observers, load a signing identity, reset/checkpoint WAL, deliver subscriptions,
refresh graph-count snapshots, or prune events. Read policies (namespace, search
limits, labels and resolution) are still configured. Keyword search requires an
index already present on the primary. Attachments and external vector backends
currently refuse startup rather than silently dropping configured providers.
Graph-count metrics remain unavailable until an independent read-only refresh
contract is implemented; they must not be interpreted as healthy zeros.

Mutating HTTP routes return **503** with `reason: "read_only_replica"` and the
configured `primary` URL. Writes are never forwarded or redirected, including
signed writes: the refusal occurs before credential/nonce settlement. SQLite's
read-only handle independently refuses a planted writer that bypasses HTTP.
This role does not provision, replicate, promote or fail over a database.

### Lag observations

`GET /watermark` returns the serving store's `store_id` and `watermark_tx` using
indexed reads. A replica polls that endpoint on its configured primary every
15 seconds, with a two-second HTTP timeout, no redirects and a bounded response.
This is its only background task and performs no database writes.

Responses include `X-Quipu-Replica-Lag-Tx`. The metrics are:

- `quipu_replica_lag_transactions`: primary minus follower transaction IDs;
  **-1 means UNKNOWN**.
- `quipu_replica_lag_known`: 1 only for a successful observation no older than
  30 seconds; otherwise 0.

A failed request, malformed response, mismatched store lineage, primary behind
the follower, unavailable local connection, or stale observation gives UNKNOWN,
not zero. Values describe the most recent bounded observation, not an atomic
cross-host snapshot, elapsed seconds or proof of identical contents. The
replication operator must establish the same primary lineage and common-prefix
contract before interpreting transaction distance. A primary without
`/watermark` yields UNKNOWN until upgraded; no expensive fallback query runs.

`just replica-test /path/to/quipu-server /path/to/quipu` checks a scratch follower:
100 positive reads, unchanged main DB hash/counter/mtime across start and stop,
503 write responses, lag failure handling and a pending-subscription trap that
must never deliver. Library/server tests also plant a write against the actual
read-only connection. The scratch test does not change or probe production.

## Private bounded query specimen

`server.query_capture` is omitted by default. An omitted configuration or
`enabled = false` creates no capture worker, file or capture-state log. An
operator can explicitly configure `enabled`, an exact lowercase `query_sha256`,
`query_bytes` (1–4096), a unique alphanumeric/underscore/hyphen `window_id`, and
absolute `start_epoch_secs`/`end_epoch_secs`. The window is at most five minutes;
startup refuses an expired window or a start more than fifteen minutes ahead.
A parsed selector/window that fails validation disables the diagnostic without
changing normal query results; malformed configuration still follows normal
strict configuration-loading errors. There is no HTTP endpoint to arm it.

Only the first matching Seeds-category POST `/query` request can consume the
claim. Eligibility is the intersection of the absolute UTC window and its
startup-pinned monotonic start/end, so clock jumps cannot extend or start it early. At startup the
server exclusively claims `query-captures/<window_id>.json` beside the canonical
store file, using a same-owner private directory and descriptor-relative,
no-follow exclusive creation. Existing claims are never overwritten, including
empty files left by a crash: restarting cannot renew the attempt. Unsupported
platforms refuse enabled capture. Operators retain and adjudicate these private
artifacts; the server does not delete or retry them.

A capacity-one worker writes at most 16 KiB outside store locks. The record
contains exact raw SPARQL, bounded planning options, UTC, build identity and the
existing request/process/query/input digests. Headers, response bodies and
unknown JSON fields are excluded; omitted fields are explicitly flagged. Raw
query literals can be sensitive, and a digest is not anonymization. The Seeds
client category is an unauthenticated attribution label, not proof of caller
identity. Serialization overflow consumes the attempt and records refusal
without a truncated query. File failures leave an UNKNOWN partial or empty
claim; no request is retried. Default-off source deployment does not authorize
arming: use a separately approved window and flag-off rollback, then read back
that the flag is off.
