# quipu-client

Thin, typed Python client for the [Quipu](https://github.com/scbrown/quipu)
REST API. Standard library only — `urllib`, zero runtime dependencies,
Python >= 3.11.

```bash
pip install ./python          # from a quipu checkout
```

```python
from quipu_client import QuipuClient, QuipuError

q = QuipuClient("http://localhost:3030", token="secret-for-writes")

q.health()                    # {"status": "ok"} — reads need no token
res = q.episode("deploy-v2", nodes=[{"name": "myapp", "type": "WebApplication"}])
res.outcome                   # "created" | "updated" | "unchanged" — branch on
                              # this, never on count (an idempotent retry is
                              # "unchanged" and that is success)

try:
    q.knot("@prefix ex: <http://example.org/> . ex:a ex:b 42 .")
except QuipuError as e:
    e.status                  # HTTP status
    e.body                    # full response body — a SHACL refusal's
    e.reason                  # feedback payload is never swallowed
```

The request shapes are kept honest against
`docs/book/src/reference/rest-api.md` in the main repository; see
`docs/book/src/reference/python-client.md` for the full method list.

Tests run against a stdlib `http.server` stub — no network, no live Quipu:

```bash
python3 -m pytest python/tests -q
```
