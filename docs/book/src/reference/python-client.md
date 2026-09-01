# Python Client

`quipu-client` is a thin, typed Python client for the [REST API](./rest-api.md),
living in `python/` in the main repository. Standard library only — `urllib`
all the way down, **zero runtime dependencies**, Python >= 3.11. It is a
wrapper, not a re-implementation: request shapes, auth, and error surfaces are
kept honest against the REST reference, and anything not listed here is a call
away with `curl`.

## Installation

```bash
# from a quipu checkout
pip install ./python
```

## Quick start

```python
from quipu_client import QuipuClient, QuipuError

q = QuipuClient("http://localhost:3030", token="secret-for-writes")

q.health()      # {"status": "ok"}
q.stats()       # {"facts": ..., "entities": ..., "predicates": ...}
q.version()     # {"version", "git_sha", "git_dirty", "features"}
```

**Reads are open; writes need a bearer token** — the client mirrors that
contract exactly. `token` is attached as `Authorization: Bearer <token>` on
write calls only, and a client constructed without a token sends no
`Authorization` header at all. The optional `client_id` / `task_id`
constructor arguments become the `X-Quipu-Client` / `X-Quipu-Task`
[attribution headers](./rest-api.md#request-attribution).

## Reads

| Method | Endpoint | Returns |
|---|---|---|
| `health()` | `GET /health` | dict |
| `stats()` | `GET /stats` | dict |
| `version()` | `GET /version` | dict |
| `query(sparql, graph=, valid_at=, tx=, fork=, include_kinds=, federated=)` | `POST /query` | dict |
| `validate(shapes, data)` | `POST /validate` | dict |
| `search(embedding=, query=, limit=, valid_at=, group_ids=, entity_type=)` | `POST /search` | dict |
| `hybrid_search(sparql, embedding, limit=)` | `POST /hybrid_search` | dict |
| `context(query, max_entities=)` | `POST /context` | dict |
| `ask(name=, params=)` | `POST /ask` | `AskResult` (dict when listing the catalog) |
| `export(graph=, group_id=, construct=, format=)` | `POST /export` | `str` — the RDF document itself, not JSON |

Unset optionals are **omitted from the request body entirely**, never sent as
`null` — to `/query`, absent and null are different things (silence never
widens scope).

## Writes

| Method | Endpoint | Returns |
|---|---|---|
| `knot(turtle, shapes=, timestamp=, actor=, source=, graph=, ...)` | `POST /knot` | `KnotResult(tx_id, count, conforms)` |
| `episode(name, nodes=, edges=, replace_snapshot=, source=, timestamp=)` | `POST /episode` | `EpisodeResult(outcome, count, tx_id)` |
| `set(entity, predicate, value, timestamp=, actor=)` | `POST /set` | `SetResult(tx_id, retracted, asserted, ...)` |
| `retract(entity, predicate=, value=, timestamp=, actor=)` | `POST /retract` | `RetractResult(retracted)` |

Every result dataclass keeps the full decoded body in `.raw`.

Two REST-doc contracts worth restating because the types encode them:

- **Branch on `EpisodeResult.outcome`, never on `count`.** `unchanged` is a
  successful idempotent retry — re-posting under a new name because
  `count == 0` looked like failure is how duplicate entities get minted.
- **An edge value is `{"iri": ...}`, a bare string is a literal.** `set` and
  `retract` pass `value` through untranslated, so the server's loud 400 for a
  bare IRI-shaped string reaches you as a `QuipuError` with the guidance
  attached.

## Errors: `QuipuError`

Every non-2xx answer raises `QuipuError` — a refusal is never silent, and
never swallowed:

```python
try:
    q.knot(turtle)
except QuipuError as e:
    e.status    # HTTP status code
    e.body      # decoded JSON body (or raw text when not JSON)
    e.reason    # the stable machine-readable field, e.g.
                # "missing_or_invalid_bearer_token" — None if absent
```

A SHACL refusal's feedback payload — which shape, which constraint, which
focus node — arrives intact in `e.body` and in `str(e)`, because that
feedback is the entire point of the refusal.

## Tests

The suite runs against a stdlib `http.server` stub asserting method, path,
headers, and body per call — no network, no live Quipu:

```bash
python3 -m pytest python/tests -q
```
