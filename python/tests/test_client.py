"""quipu-client tests against a stdlib http.server stub.

No network beyond loopback, no live Quipu, no dependencies beyond pytest.
The stub records every request (method, path, headers, body) and returns
canned responses, so each test asserts the exact wire shape the REST doc
specifies."""

from __future__ import annotations

import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from quipu_client import EpisodeResult, KnotResult, QuipuClient, QuipuError


class StubHandler(BaseHTTPRequestHandler):
    """Records requests on the server object; replies from server.responses."""

    def _handle(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        self.server.requests.append(
            {
                "method": self.command,
                "path": self.path,
                "headers": dict(self.headers),
                "body": json.loads(raw) if raw else None,
            }
        )
        status, content_type, body = self.server.responses.get(
            self.path, (200, "application/json", b"{}")
        )
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = _handle
    do_POST = _handle

    def log_message(self, *args):  # keep pytest output clean
        pass


@pytest.fixture()
def stub():
    server = HTTPServer(("127.0.0.1", 0), StubHandler)
    server.requests = []
    server.responses = {}
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    server.url = f"http://127.0.0.1:{server.server_address[1]}"
    yield server
    server.shutdown()
    server.server_close()


def canned(payload, status=200):
    return (status, "application/json", json.dumps(payload).encode())


# ------------------------------------------------------------------- reads


def test_health_is_a_get_with_no_auth_header(stub):
    stub.responses["/health"] = canned({"status": "ok"})
    q = QuipuClient(stub.url, token="secret")
    assert q.health() == {"status": "ok"}
    req = stub.requests[0]
    assert req["method"] == "GET"
    assert req["path"] == "/health"
    # Reads are open — even a token-carrying client must not send the header.
    assert "Authorization" not in req["headers"]


def test_stats_and_version_paths(stub):
    stub.responses["/stats"] = canned({"facts": 3, "entities": 2, "predicates": 1})
    stub.responses["/version"] = canned({"version": "0.9.0", "git_sha": "abc"})
    q = QuipuClient(stub.url)
    assert q.stats()["facts"] == 3
    assert q.version()["git_sha"] == "abc"
    assert [r["path"] for r in stub.requests] == ["/stats", "/version"]
    assert all(r["method"] == "GET" for r in stub.requests)


def test_query_sends_only_the_fields_that_were_set(stub):
    stub.responses["/query"] = canned({"rows": []})
    q = QuipuClient(stub.url)
    q.query("SELECT ?s WHERE { ?s ?p ?o }", graph="urn:g", valid_at="2026-01-01T00:00:00Z")
    req = stub.requests[0]
    assert req["method"] == "POST"
    assert req["headers"]["Content-Type"] == "application/json"
    assert req["body"] == {
        "query": "SELECT ?s WHERE { ?s ?p ?o }",
        "graph": "urn:g",
        "valid_at": "2026-01-01T00:00:00Z",
    }
    # Absent and null are different things to /query — unset optionals
    # (tx, fork, include_kinds, federated) must not appear at all.
    assert "tx" not in req["body"]
    assert "federated" not in req["body"]


def test_query_as_of_tx_pin(stub):
    stub.responses["/query"] = canned({"rows": []})
    QuipuClient(stub.url).query("SELECT * WHERE { ?s ?p ?o }", tx=5)
    assert stub.requests[0]["body"]["tx"] == 5


def test_attribution_headers_when_configured(stub):
    stub.responses["/health"] = canned({"status": "ok"})
    QuipuClient(stub.url, client_id="python-client", task_id="quipu-f79").health()
    headers = stub.requests[0]["headers"]
    assert headers["X-Quipu-Client"] == "python-client"
    assert headers["X-Quipu-Task"] == "quipu-f79"


def test_validate_posts_shapes_and_data(stub):
    stub.responses["/validate"] = canned({"conforms": False})
    QuipuClient(stub.url).validate("@prefix sh: ...", "@prefix ex: ...")
    assert stub.requests[0]["body"] == {
        "shapes": "@prefix sh: ...",
        "data": "@prefix ex: ...",
    }


def test_search_and_hybrid_search_shapes(stub):
    stub.responses["/search"] = canned({"results": []})
    stub.responses["/hybrid_search"] = canned({"results": []})
    q = QuipuClient(stub.url)
    q.search(embedding=[0.1, 0.2], limit=10)
    q.hybrid_search("SELECT ?s WHERE { ?s a <urn:T> }", [0.1], limit=5)
    assert stub.requests[0]["body"] == {"embedding": [0.1, 0.2], "limit": 10}
    assert stub.requests[1]["body"] == {
        "sparql": "SELECT ?s WHERE { ?s a <urn:T> }",
        "embedding": [0.1],
        "limit": 5,
    }


def test_context_shape(stub):
    stub.responses["/context"] = canned(
        {"entities": [], "summary": {"embeddings": {"configured": False}}}
    )
    result = QuipuClient(stub.url).context("traefik", max_entities=10)
    assert stub.requests[0]["body"] == {"query": "traefik", "max_entities": 10}
    assert result["summary"]["embeddings"]["configured"] is False


def test_ask_named_query_returns_typed_result(stub):
    stub.responses["/ask"] = canned(
        {"query": "service_deps", "sparql": "SELECT ...", "columns": ["dep"],
         "rows": [["urn:x"]], "count": 1}
    )
    result = QuipuClient(stub.url).ask("service_deps", {"entity": "urn:svc"})
    assert stub.requests[0]["body"] == {
        "name": "service_deps",
        "params": {"entity": "urn:svc"},
    }
    assert result.columns == ["dep"]
    assert result.count == 1


def test_ask_without_name_lists_the_catalog_as_a_dict(stub):
    stub.responses["/ask"] = canned({"queries": ["service_deps"]})
    result = QuipuClient(stub.url).ask()
    assert stub.requests[0]["body"] == {}
    assert result == {"queries": ["service_deps"]}


def test_export_returns_rdf_text_not_json(stub):
    turtle = "@prefix ex: <http://example.org/> .\nex:a ex:b ex:c .\n"
    stub.responses["/export"] = (200, "text/turtle", turtle.encode())
    result = QuipuClient(stub.url).export(graph="urn:g", format="turtle")
    assert result == turtle
    assert stub.requests[0]["body"] == {"graph": "urn:g", "format": "turtle"}


# ------------------------------------------------------------------ writes


def test_knot_with_token_sends_bearer_and_parses_envelope(stub):
    stub.responses["/knot"] = canned({"tx_id": 7, "count": 2, "conforms": True})
    q = QuipuClient(stub.url, token="s3cret")
    result = q.knot("@prefix ex: <urn:x> .", actor="tester", source="pytest")
    req = stub.requests[0]
    assert req["headers"]["Authorization"] == "Bearer s3cret"
    assert req["body"]["turtle"] == "@prefix ex: <urn:x> ."
    assert req["body"]["actor"] == "tester"
    assert isinstance(result, KnotResult)
    assert (result.tx_id, result.count, result.conforms) == (7, 2, True)


def test_write_without_token_sends_no_authorization_header(stub):
    stub.responses["/knot"] = canned({"tx_id": 1, "count": 1, "conforms": True})
    QuipuClient(stub.url).knot("@prefix ex: <urn:x> .")
    assert "Authorization" not in stub.requests[0]["headers"]


def test_episode_outcome_is_the_contract_not_count(stub):
    stub.responses["/episode"] = canned({"outcome": "unchanged", "count": 0, "tx_id": 0})
    result = QuipuClient(stub.url, token="t").episode(
        "deploy-v2", nodes=[{"name": "myapp", "type": "WebApplication"}]
    )
    assert isinstance(result, EpisodeResult)
    # unchanged + count 0 is a successful idempotent retry, not a failure.
    assert result.outcome == "unchanged"
    assert result.count == 0
    assert stub.requests[0]["body"]["name"] == "deploy-v2"


def test_set_envelope_and_value_passthrough(stub):
    stub.responses["/set"] = canned(
        {"tx_id": 3, "retracted": 1, "asserted": 1,
         "entity": "urn:svc", "predicate": "urn:reports_to"}
    )
    result = QuipuClient(stub.url, token="t").set(
        "urn:svc", "urn:reports_to", {"iri": "urn:new-boss"}, actor="tester"
    )
    assert stub.requests[0]["body"]["value"] == {"iri": "urn:new-boss"}
    assert (result.retracted, result.asserted) == (1, 1)


def test_retract_idempotent_zero_is_quiet(stub):
    stub.responses["/retract"] = canned({"retracted": 0})
    result = QuipuClient(stub.url, token="t").retract(
        "urn:svc", predicate="urn:p", value={"iri": "urn:gone"}
    )
    assert result.retracted == 0


# ---------------------------------------------------------------- refusals


def test_auth_refusal_raises_with_reason_and_body(stub):
    refusal = {
        "endpoint": "/knot",
        "reason": "missing_or_invalid_bearer_token",
        "error": "unauthorized: ...",
    }
    stub.responses["/knot"] = canned(refusal, status=401)
    with pytest.raises(QuipuError) as exc:
        QuipuClient(stub.url).knot("@prefix ex: <urn:x> .")
    assert exc.value.status == 401
    assert exc.value.reason == "missing_or_invalid_bearer_token"
    assert exc.value.body == refusal


def test_shacl_refusal_surfaces_feedback_payload(stub):
    # A SHACL refusal names the violated constraint; that feedback is the
    # whole point of the refusal and must never be swallowed.
    feedback = {
        "error": "SHACL validation failed",
        "conforms": False,
        "results": [
            {"focus": "urn:alice", "constraint": "sh:MinCountConstraintComponent",
             "message": "Less than 1 values on urn:alice->urn:email"}
        ],
    }
    stub.responses["/knot"] = canned(feedback, status=422)
    with pytest.raises(QuipuError) as exc:
        QuipuClient(stub.url, token="t").knot("@prefix ex: <urn:x> .")
    assert exc.value.status == 422
    assert exc.value.body["results"][0]["constraint"] == "sh:MinCountConstraintComponent"
    # The stringified error carries the payload too — a bare log line is
    # still diagnosable.
    assert "MinCountConstraintComponent" in str(exc.value)


def test_retract_bare_string_400_surfaces_guidance(stub):
    body = {"error": 'bare string cannot match an IRI reference; use {"iri": ...}'}
    stub.responses["/retract"] = canned(body, status=400)
    with pytest.raises(QuipuError) as exc:
        QuipuClient(stub.url, token="t").retract(
            "urn:svc", predicate="urn:p", value="http://example.org/boss"
        )
    assert exc.value.status == 400
    assert "iri" in str(exc.value)


def test_non_json_error_body_is_kept_as_text(stub):
    stub.responses["/query"] = (500, "text/plain", b"internal error, sorry")
    with pytest.raises(QuipuError) as exc:
        QuipuClient(stub.url).query("SELECT * WHERE { ?s ?p ?o }")
    assert exc.value.status == 500
    assert exc.value.body == "internal error, sorry"
    assert exc.value.reason is None
