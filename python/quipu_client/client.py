"""Thin client over the Quipu REST API — stdlib urllib, zero dependencies.

Request shapes are kept honest against
``docs/book/src/reference/rest-api.md``. The authoritative write-endpoint
list is ``http_auth::WRITE_ENDPOINTS`` in the server; this client mirrors
the documented split — reads are open, writes send ``Authorization: Bearer``
when a token is configured, and never send the header without one.
"""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from typing import Any

from .types import AskResult, EpisodeResult, KnotResult, RetractResult, SetResult


class QuipuError(Exception):
    """A non-2xx answer from Quipu, with the refusal payload attached.

    A refusal is never silent on the server (auth failures and SHACL
    refusals both return a JSON body naming the cause), so it must never be
    silent here either: ``body`` carries the decoded JSON (or raw text) and
    ``reason`` the stable machine-readable field when the server sent one.
    """

    def __init__(self, status: int, body: Any, path: str):
        self.status = status
        self.body = body
        self.path = path
        self.reason = body.get("reason") if isinstance(body, dict) else None
        detail = json.dumps(body) if isinstance(body, (dict, list)) else str(body)
        super().__init__(f"quipu {path} -> HTTP {status}: {detail}")


def _drop_none(fields: dict[str, Any]) -> dict[str, Any]:
    """Omit unset optionals entirely — absent and null are not the same
    thing to endpoints like /query, where silence never widens scope."""
    return {k: v for k, v in fields.items() if v is not None}


class QuipuClient:
    """Client for one ``quipu-server``.

    ``token`` is only attached to write requests, matching the server's
    contract (reads are open). ``client_id`` / ``task_id`` become the
    optional ``X-Quipu-Client`` / ``X-Quipu-Task`` attribution headers.
    """

    def __init__(
        self,
        base_url: str,
        *,
        token: str | None = None,
        timeout: float = 30.0,
        client_id: str | None = None,
        task_id: str | None = None,
    ):
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout
        self.client_id = client_id
        self.task_id = task_id

    # ------------------------------------------------------------ transport

    def _request(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
        *,
        write: bool = False,
        raw_response: bool = False,
    ) -> Any:
        headers: dict[str, str] = {}
        data = None
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        if write and self.token is not None:
            headers["Authorization"] = f"Bearer {self.token}"
        if self.client_id is not None:
            headers["X-Quipu-Client"] = self.client_id
        if self.task_id is not None:
            headers["X-Quipu-Task"] = self.task_id

        req = urllib.request.Request(
            self.base_url + path, data=data, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                payload = resp.read()
        except urllib.error.HTTPError as e:
            # The refusal body is the diagnosis (SHACL feedback, auth
            # reason, value-shape guidance) — read it before raising.
            raise QuipuError(e.code, _decode(e.read()), path) from None

        if raw_response:
            return payload.decode("utf-8")
        return _decode(payload)

    # ---------------------------------------------------------------- reads

    def health(self) -> dict[str, Any]:
        """``GET /health`` -> ``{"status": "ok"}``."""
        return self._request("GET", "/health")

    def stats(self) -> dict[str, Any]:
        """``GET /stats`` -> fact/entity/predicate counts."""
        return self._request("GET", "/stats")

    def version(self) -> dict[str, Any]:
        """``GET /version`` -> version, git_sha, git_dirty, features."""
        return self._request("GET", "/version")

    def query(
        self,
        query: str,
        *,
        graph: str | None = None,
        valid_at: str | None = None,
        tx: int | None = None,
        fork: str | None = None,
        include_kinds: list[str] | None = None,
        federated: bool | None = None,
    ) -> dict[str, Any]:
        """``POST /query`` — SPARQL. ``tx`` is the as-of transaction pin;
        ``graph`` scopes the default graph without a FROM clause; ``fork``
        and ``graph`` are mutually exclusive server-side."""
        body = _drop_none(
            {
                "query": query,
                "graph": graph,
                "valid_at": valid_at,
                "tx": tx,
                "fork": fork,
                "include_kinds": include_kinds,
                "federated": federated,
            }
        )
        return self._request("POST", "/query", body)

    def validate(self, shapes: str, data: str) -> dict[str, Any]:
        """``POST /validate`` — dry-run SHACL validation, writes nothing."""
        return self._request("POST", "/validate", {"shapes": shapes, "data": data})

    def search(
        self,
        *,
        embedding: list[float] | None = None,
        query: str | None = None,
        limit: int | None = None,
        valid_at: str | None = None,
        group_ids: list[str] | None = None,
        entity_type: str | None = None,
    ) -> dict[str, Any]:
        """``POST /search`` — vector similarity. ``group_ids`` is a
        best-effort provenance filter, not an isolation boundary."""
        body = _drop_none(
            {
                "embedding": embedding,
                "query": query,
                "limit": limit,
                "valid_at": valid_at,
                "group_ids": group_ids,
                "entity_type": entity_type,
            }
        )
        return self._request("POST", "/search", body)

    def hybrid_search(
        self,
        sparql: str,
        embedding: list[float],
        *,
        limit: int | None = None,
    ) -> dict[str, Any]:
        """``POST /hybrid_search`` — SPARQL filter + vector ranking."""
        body = _drop_none({"sparql": sparql, "embedding": embedding, "limit": limit})
        return self._request("POST", "/hybrid_search", body)

    def context(self, query: str, *, max_entities: int | None = None) -> dict[str, Any]:
        """``POST /context`` — the knowledge context pipeline. Check
        ``summary.embeddings`` before reading an empty ``entities`` list as
        "nothing relevant"."""
        body = _drop_none({"query": query, "max_entities": max_entities})
        return self._request("POST", "/context", body)

    def ask(
        self, name: str | None = None, params: dict[str, Any] | None = None
    ) -> AskResult | dict[str, Any]:
        """``POST /ask`` — run a stored named query. With no ``name`` the
        catalog listing comes back as a plain dict."""
        body = _drop_none({"name": name, "params": params})
        result = self._request("POST", "/ask", body)
        if name is None or name == "list":
            return result
        return AskResult.from_body(result)

    def export(
        self,
        *,
        graph: str | None = None,
        group_id: str | None = None,
        construct: str | None = None,
        format: str | None = None,
    ) -> str:
        """``POST /export`` — returns the RDF document itself (Turtle or
        N-Triples), not JSON. ``graph``/``group_id``/``construct`` are
        mutually exclusive server-side; omit all three for ROOT."""
        body = _drop_none(
            {
                "graph": graph,
                "group_id": group_id,
                "construct": construct,
                "format": format,
            }
        )
        return self._request("POST", "/export", body, raw_response=True)

    # --------------------------------------------------------------- writes

    def knot(
        self,
        turtle: str,
        *,
        shapes: str | None = None,
        timestamp: str | None = None,
        actor: str | None = None,
        source: str | None = None,
        graph: str | None = None,
        replace_snapshot: str | None = None,
        snapshot: str | None = None,
    ) -> KnotResult:
        """``POST /knot`` — assert Turtle. Pass ``actor`` and ``source``:
        omitting both lands facts with no audit trail at all."""
        body = _drop_none(
            {
                "turtle": turtle,
                "shapes": shapes,
                "timestamp": timestamp,
                "actor": actor,
                "source": source,
                "graph": graph,
                "replace_snapshot": replace_snapshot,
                "snapshot": snapshot,
            }
        )
        return KnotResult.from_body(self._request("POST", "/knot", body, write=True))

    def episode(
        self,
        name: str,
        *,
        nodes: list[dict[str, Any]] | None = None,
        edges: list[dict[str, Any]] | None = None,
        replace_snapshot: bool | None = None,
        source: str | None = None,
        timestamp: str | None = None,
    ) -> EpisodeResult:
        """``POST /episode`` — idempotent ingest. Branch on
        ``result.outcome``; ``unchanged`` is success, and retrying after a
        lost response is safe under the same name."""
        body = _drop_none(
            {
                "name": name,
                "nodes": nodes,
                "edges": edges,
                "replace_snapshot": replace_snapshot,
                "source": source,
                "timestamp": timestamp,
            }
        )
        return EpisodeResult.from_body(
            self._request("POST", "/episode", body, write=True)
        )

    def set(
        self,
        entity: str,
        predicate: str,
        value: Any,
        *,
        timestamp: str | None = None,
        actor: str | None = None,
    ) -> SetResult:
        """``POST /set`` — atomic single-value supersede. An edge value must
        be ``{"iri": ...}``; a bare IRI-shaped string is a loud 400."""
        body = _drop_none(
            {
                "entity": entity,
                "predicate": predicate,
                "value": value,
                "timestamp": timestamp,
                "actor": actor,
            }
        )
        return SetResult.from_body(self._request("POST", "/set", body, write=True))

    def retract(
        self,
        entity: str,
        *,
        predicate: str | None = None,
        value: Any | None = None,
        timestamp: str | None = None,
        actor: str | None = None,
    ) -> RetractResult:
        """``POST /retract``. An IRI-object triple needs
        ``value={"iri": ...}`` — a bare string is matched as a literal and
        the server refuses (400) when that cannot match."""
        body = _drop_none(
            {
                "entity": entity,
                "predicate": predicate,
                "value": value,
                "timestamp": timestamp,
                "actor": actor,
            }
        )
        return RetractResult.from_body(
            self._request("POST", "/retract", body, write=True)
        )


def _decode(payload: bytes) -> Any:
    """JSON when it is JSON, text when it is not — an error body is evidence
    either way and must survive intact."""
    text = payload.decode("utf-8", errors="replace")
    try:
        return json.loads(text)
    except ValueError:
        return text
