# Design: `RemoteProvider` — reaching remote Quipu instances

> **Implementation status (2026-08-01):** ⬜ **Designed, not built.**
> `GraphProvider`, `ProviderStatus`, `LocalProvider` and `FederatedProvider`
> exist in `src/provider.rs`; `[[quipu.federation.remotes]]` parses in
> `src/config.rs` and warns loudly because nothing consumes it. This document
> settles the decisions [quipu#47](https://github.com/scbrown/quipu/issues/47)
> lists as open so the implementation is mechanical.

## 1. The dependency question is already answered

Issue #47 says "no outbound HTTP client dependency exists yet … so this needs a
dep decision." That is **stale**. `Cargo.toml` already carries:

```toml
ureq = { version = "2", default-features = false }
```

and the event-push delivery worker already makes outbound POSTs with it
(`quipu-server`'s push tick, via `store::push::deliver_tick`). So:

**Decision: use `ureq`.** No new dependency, no second HTTP stack in the binary,
and the failure modes are ones this codebase has already met. `ureq` is
blocking, which is correct here — every `GraphProvider` method is synchronous by
trait signature, and the server already runs store work on
`tokio::task::spawn_blocking`.

Rejected: `reqwest` (drags in a second TLS stack and an async runtime for a
sync trait); a hand-rolled client (no).

## 2. The remote contract

A remote is another `quipu-server`. Each trait method maps to one endpoint that
already exists:

| Trait method | Endpoint | Request | Response |
|---|---|---|---|
| `query(sparql)` | `POST /query` | `{"query": "<sparql>"}` | `QueryResult` JSON |
| `entities(type, limit)` | `POST /cord` | `{"type": …, "limit": N}` | `{"entities": […]}` |
| `health()` | `GET /stats` | — | `{"facts", "entities", "predicates"}` |

`/stats` is deliberately the health probe rather than `/health`: it is the
cheapest call that proves the remote can actually reach its store, and it
supplies `fact_count` for `ProviderStatus` in the same round trip. It is
generation-cached on the remote, so polling it is O(1) after the first call.

## 3. Config — three fields the current schema lacks

`RemoteEndpoint` is `{name, url}`. That cannot talk to any remote that took the
project's own advice and set `server.auth_token`, and it cannot bound a hung
peer. Extend it:

```toml
[[quipu.federation.remotes]]
name       = "kota"
url        = "http://quipu.kota.example:3030"
auth_token = "…"     # optional; sent as `Authorization: Bearer …`
timeout_ms = 5000    # optional; default 5000
```

`auth_token` is `Option<String>` and defaults to `None` (open remote).
`timeout_ms` defaults to 5000 — long enough for a real query, short enough that
one dead peer does not dominate a federated call. Both are additive, so existing
config files keep parsing.

Reading a token from config in plaintext is the same posture the server already
takes for its own `server.auth_token`; this design does not change it, and an
env-var indirection is a separate concern for both.

## 4. Failure semantics — the decision that actually matters

`FederatedProvider::query_all` currently does this:

```rust
if let Ok(result) = provider.query(sparql) { … }
```

A provider that errors is **silently skipped**. With only `LocalProvider` that
was nearly unreachable. The moment a remote exists it becomes the common case —
a peer is down, a token is wrong, a network blips — and the caller receives a
`200` with fewer rows and no indication that a third of the federation did not
answer. That is exactly the failure class #53 was about: successful-looking,
silently incomplete.

**Decision: a federated query reports which providers answered.**

- A remote that is unreachable, times out, returns non-2xx, or returns
  undeserializable JSON **does not abort the federated query** — issue #47 is
  explicit about that, and it is right: one dead peer must not deny the whole
  result.
- But the outcome is **reported**, not swallowed. `query_all` returns the merged
  rows *plus* a per-provider outcome list, and the REST/MCP surface carries it:

```json
{
  "results": [ … ],
  "providers": [
    {"name": "local", "ok": true,  "rows": 12},
    {"name": "kota",  "ok": false, "error": "timeout after 5000ms"}
  ],
  "complete": false
}
```

`complete: false` is the one-field answer to "can I trust this result set as
exhaustive?" — the same job the `embeddings` block does for `/context`.

- `health()` keeps issue #47's rule: unreachable → `ProviderStatus { healthy:
  false, message: Some(reason) }`, never an `Err` that aborts `health_all`.

### 4.1 Two existing bugs in `query_all` to fix in the same change

Both are latent today and load-bearing once a second provider exists:

1. **`variables` is taken from the first provider that answers**, and every
   later provider's rows are merged under it. Two providers answering the same
   SPARQL should agree, but a remote running an older quipu may not. Merging
   mismatched rows silently mislabels columns. Fix: if a provider's variable
   list differs from the established one, record it as a provider-level failure
   rather than merging.
2. **`provider_var_added` is computed once, from the first provider.** If the
   first result already contains `_provider`, the flag stays false and *no*
   provider gets tagged — including ones that would have needed it.

## 5. Wiring — retiring the dead config

`unwired_warnings()` currently warns that `federation.remotes` is ignored, and
`config.rs`'s `UNWIRED_TOP_LEVEL` test guard lists `"federation"`. When
`RemoteProvider` lands, **both must be updated in the same change** — the guard
exists precisely to force that, and the repo treats a settable-but-inert knob as
a defect.

Construction happens where the store is built (`quipu-server` startup):
configured remotes become `RemoteProvider`s, joined with a `LocalProvider` in a
`FederatedProvider`. With no remotes configured, the federated path is
byte-identical to today's local-only behaviour.

## 6. Testing

Unit-level against a mock HTTP server, per issue #47's constraint — **no live
interop test** (kota is memory-contended, aegis-vj9g). A tiny
`std::net::TcpListener` on an ephemeral port serving canned responses is
sufficient and adds no dependency:

- [ ] `query` round-trips a `QueryResult` from a canned `/query` response
- [ ] `entities` round-trips a `/cord` response
- [ ] `health` maps `/stats` to `ProviderStatus` with `fact_count`
- [ ] Unreachable remote → `healthy: false`, and `query_all` still returns the
      local rows with `complete: false` and a named failure
- [ ] Timeout is bounded by `timeout_ms`, not by the OS default
- [ ] `auth_token` is sent as `Authorization: Bearer …`
- [ ] Non-2xx and malformed JSON are provider failures, not panics
- [ ] Variable-list mismatch is a provider failure, not a silent merge
- [ ] `_provider` tagging is correct when the first provider already has the key

## 7. Deliberately out of scope

- **SERVICE keyword.** Federating via SPARQL 1.1 `SERVICE` inside a query is a
  different feature: it needs the evaluator to plan sub-queries per endpoint.
  This design federates whole queries, which is what `FederatedProvider` models.
- **Join pushdown.** Every provider gets the full query text and answers
  independently; results are unioned, not joined across providers.
- **Write federation.** Remotes are read-only. There is no distributed
  transaction here and this design does not open one.

## 8. Related

- [federation.md](../book/src/architecture/federation.md) — the user-facing doc,
  whose 🟡 banner comes down when this ships.
- `src/provider.rs`, `src/config.rs` (`FederationConfig` / `RemoteEndpoint`).
- quipu#47 (this feature).
