#!/usr/bin/env python3
"""Run the checked-in W3C SPARQL 1.1 harness against OTHER stores (aegis-hit21a).

The point is one table in which quipu and its peers are scored by the SAME
code at the SAME suite revision. So this module adds no discovery, selection or
comparison logic of its own. It imports `sparql11_evaluation`'s
`discover_cases`, `unsupported_reason`, `expected_result`, `reorder_rows` and
`rows_equal_with_blank_nodes`, and supplies only what differs per system: a
DRIVER that loads fixtures, answers a query, applies an update, and dumps a
graph.

FAIRNESS, stated where the code is (the preregistration is on aegis-nges80):

* Every system gets a fresh in-memory store per case and the same per-request
  timeout.
* SELECT/ASK answers travel as SPARQL JSON results and are parsed by the
  runner's own `expected_json`, so every system's terms are rendered by ONE
  function (`term`).
* GRAPH answers (CONSTRUCT, and the post-update dataset) are compared by
  loading the EXPECTED graph into a fresh instance of THE SYSTEM UNDER TEST
  and dumping both sides through that system with the same SELECT. A
  difference in how two parsers print a literal therefore cannot score as a
  wrong answer, and the comparison measures query semantics. Parsing is
  measured separately by the syntax suites (aegis-mhee08). Blank nodes compare
  up to one consistent renaming.
* A case the runner marks unsupported for quipu (entailment non-goals, the
  variable SERVICE endpoint) is not run here either. A competitor's own
  entailment configuration is a separate, declared arm.
* Disclosure: quipu parses SPARQL with `spargebra` and models RDF with
  `oxrdf`, both from the Oxigraph project. Syntax results are therefore
  partly shared with Oxigraph by construction.

Artifacts are PINNED by version and sha256 (see PINS) and cached under
~/.cache/quipu-bench/competitors. The same Oxigraph binary serves the WatDiv
comparison (aegis-j0yaxj.2): `competitors.py provision oxigraph` prints its
path.

    python3 benchmark/competitors/competitors.py provision oxigraph
    python3 benchmark/competitors/competitors.py run --system oxigraph \\
        --suite /tmp/rdf-tests/sparql/sparql11 --output /tmp/oxigraph.json
    uv run --with rdflib==7.6.0 python3 benchmark/competitors/competitors.py run \\
        --system rdflib --suite /tmp/rdf-tests/sparql/sparql11 --output /tmp/rdflib.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter
from pathlib import Path

# Reuse the quipu runner's discovery and comparison code, from benchmark/public.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "public"))

import sparql11_evaluation as ev  # noqa: E402

CACHE = Path(os.environ.get("QUIPU_BENCH_CACHE", "~/.cache/quipu-bench/competitors")).expanduser()
TIMEOUT_S = 30
CLASSES = ("query-evaluation", "update")

#: Every artifact a run depends on, pinned. A changed hash is a REFUSAL, never
#: a silent re-pin: a different binary is a different measurement.
PINS: dict[str, dict] = {
    "oxigraph": {
        "version": "0.5.11",
        "url": "https://github.com/oxigraph/oxigraph/releases/download/v0.5.11/"
               "oxigraph_v0.5.11_x86_64_linux_gnu",
        "sha256": "66df9e704fbd26cd943757c8c62698505a1193954f86651bb0bb4abf941a917b",
        "file": "oxigraph",
    },
    # rdflib is pinned by the `uv run --with rdflib==<version>` invocation and
    # its version is recorded in the ledger from rdflib.__version__.
}

JSON_RESULTS = "application/sparql-results+json"


# -- provisioning --------------------------------------------------------------

def provision(name: str) -> Path:
    pin = PINS[name]
    target = CACHE / f"{name}-{pin['version']}" / pin["file"]
    if not target.exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        partial = target.with_suffix(".partial")
        with urllib.request.urlopen(pin["url"], timeout=120) as response, partial.open("wb") as fh:
            shutil.copyfileobj(response, fh)
        partial.rename(target)
        target.chmod(0o755)
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    if digest != pin["sha256"]:
        raise SystemExit(f"{name}: sha256 {digest} does not match the pin {pin['sha256']}; refusing")
    return target


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


# -- drivers -------------------------------------------------------------------

class Driver:
    """One fresh, empty store. Subclasses implement five operations."""

    name = "?"
    version = "?"

    def __enter__(self) -> "Driver":
        return self

    def __exit__(self, *exc) -> None:
        self.close()

    def close(self) -> None:
        pass

    def load(self, path: Path, graph: str | None) -> None:
        raise NotImplementedError

    def select(self, query: str):
        """SELECT/ASK -> the runner's (vars, rows) or bool."""
        raise NotImplementedError

    def construct(self, query: str) -> str:
        """CONSTRUCT/DESCRIBE -> N-Triples text."""
        raise NotImplementedError

    def update(self, request: str) -> None:
        raise NotImplementedError


def parse_json_results(body: bytes):
    with tempfile.NamedTemporaryFile("wb", suffix=".srj", delete=False) as fh:
        fh.write(body)
        name = fh.name
    try:
        return ev.expected_json(Path(name))
    finally:
        os.unlink(name)


def rdf_content_type(path: Path) -> str:
    return {".ttl": "text/turtle", ".nt": "application/n-triples", ".rdf": "application/rdf+xml",
            ".nq": "application/n-quads", ".trig": "application/trig"}.get(path.suffix, "text/turtle")


class HttpDriver(Driver):
    """A store spoken to over the SPARQL 1.1 Protocol and Graph Store Protocol."""

    query_path = "/query"
    update_path = "/update"
    store_path = "/store"

    def __init__(self) -> None:
        self.port = free_port()
        self.base = f"http://127.0.0.1:{self.port}"
        self.process = self.start()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", self.port), timeout=0.2):
                    return
            except OSError:
                if self.process.poll() is not None:
                    raise RuntimeError(f"{self.name} exited: {self.process.stderr.read()}")
                time.sleep(0.02)
        raise RuntimeError(f"{self.name} did not listen on {self.port}")

    def start(self) -> subprocess.Popen:
        raise NotImplementedError

    def close(self) -> None:
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()

    def _request(self, path: str, data: bytes | None, headers: dict, method: str = "POST") -> bytes:
        request = urllib.request.Request(self.base + path, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=TIMEOUT_S) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            raise ValueError(f"HTTP {error.code}: {error.read()[:300]!r}") from error

    def load(self, path: Path, graph: str | None) -> None:
        target = f"{self.store_path}?" + ("default" if graph is None else
                                          "graph=" + urllib.parse.quote(graph, safe=""))
        self._request(target, path.read_bytes(), {"Content-Type": rdf_content_type(path)})

    def select(self, query: str):
        body = self._request(self.query_path, query.encode(),
                             {"Content-Type": "application/sparql-query", "Accept": JSON_RESULTS})
        return parse_json_results(body)

    def construct(self, query: str) -> str:
        return self._request(self.query_path, query.encode(),
                             {"Content-Type": "application/sparql-query",
                              "Accept": "application/n-triples"}).decode()

    def update(self, request: str) -> None:
        self._request(self.update_path, request.encode(), {"Content-Type": "application/sparql-update"})


class OxigraphDriver(HttpDriver):
    name = "oxigraph"
    version = PINS["oxigraph"]["version"]

    def load(self, path: Path, graph: str | None) -> None:
        # The Graph Store Protocol gives the payload no base IRI, and fixtures
        # use relative IRIs (sq05.rdf: rdf:resource=""). quipu parses a fixture
        # with its file URI as the base, so Oxigraph gets the same: its OWN
        # parser, with that base, to N-Triples, then loaded (a harness fix,
        # found on :subquery06).
        converted = subprocess.run(
            [str(provision("oxigraph")), "convert", "--from-file", str(path),
             "--from-base", path.resolve().as_uri(), "--to-format", "nt"],
            capture_output=True)
        if converted.returncode:
            raise ValueError(f"oxigraph could not parse {path.name}: {converted.stderr.decode()[:300]}")
        target = f"{self.store_path}?" + ("default" if graph is None else
                                          "graph=" + urllib.parse.quote(graph, safe=""))
        self._request(target, converted.stdout, {"Content-Type": "application/n-triples"})

    def start(self) -> subprocess.Popen:
        binary = provision("oxigraph")
        # No --location: an in-memory store, fresh per case.
        return subprocess.Popen([str(binary), "serve", "--bind", f"127.0.0.1:{self.port}",
                                 "--timeout-s", str(TIMEOUT_S)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)


class RdflibDriver(Driver):
    """rdflib, in process: a Dataset whose default graph is NOT the union."""

    name = "rdflib"

    def __init__(self) -> None:
        import rdflib

        self.rdflib = rdflib
        self.version = rdflib.__version__
        self.ds = rdflib.Dataset(default_union=False)

    def load(self, path: Path, graph: str | None) -> None:
        fmt = {".ttl": "turtle", ".nt": "nt", ".rdf": "xml", ".nq": "nquads", ".trig": "trig"}.get(path.suffix, "turtle")
        target = self.ds.default_context if graph is None else self.ds.graph(self.rdflib.URIRef(graph))
        target.parse(str(path), format=fmt, publicID=path.resolve().as_uri())

    def select(self, query: str):
        return parse_json_results(self.ds.query(query).serialize(format="json"))

    def construct(self, query: str) -> str:
        return self.ds.query(query).graph.serialize(format="nt")

    def update(self, request: str) -> None:
        self.ds.update(request)


DRIVERS = {"oxigraph": OxigraphDriver, "rdflib": RdflibDriver}


# -- scoring -------------------------------------------------------------------

def dump(driver: Driver, graph: str | None) -> list[tuple[str, ...]]:
    query = ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }" if graph is None
             else f"SELECT ?s ?p ?o WHERE {{ GRAPH <{graph}> {{ ?s ?p ?o }} }}")
    variables, rows = driver.select(query)
    aligned = ev.reorder_rows(variables, rows, ["s", "p", "o"])
    if aligned is None:
        raise ValueError("graph dump emitted unexpected bindings")
    return aligned


NUMERIC = ("decimal", "double", "float", "integer")
_NUMERIC_TERM = __import__("re").compile(
    r'^"([^"]*)"\^\^<http://www\.w3\.org/2001/XMLSchema#(decimal|double|float|integer)>$')


_DURATION = __import__("re").compile(
    r'^"(-)?P(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:([\d.]+)S)?)?"\^\^'
    r'<http://www\.w3\.org/2001/XMLSchema#dayTimeDuration>$')


def numeric_value(term: str) -> str:
    """A numeric literal reduced to its VALUE, so "1.0"^^decimal == "1"^^decimal.

    Used ONLY for the secondary `lexical_form_only` tag, never for the verdict:
    the suite, and the table's primary column, use RDF term equality, which is
    the rule quipu itself is held to. Some stores (Oxigraph among them)
    canonicalise numeric literals on write by design; the tag lets the
    published table say "same value, different lexical form" instead of
    presenting that as a wrong answer.
    """
    from decimal import Decimal, InvalidOperation

    if (d := _DURATION.match(term)):
        sign, days, hours, minutes, seconds = d.groups()
        total = (Decimal(days or 0) * 86400 + Decimal(hours or 0) * 3600
                 + Decimal(minutes or 0) * 60 + Decimal(seconds or 0))
        return f"dur:{-total if sign and total else total}"
    bare = term
    if (m := _NUMERIC_TERM.match(term)):
        bare = m.group(1)
    try:
        return f"num:{Decimal(bare).normalize()}"
    except InvalidOperation:
        return term


def same_rows_by_value(actual: list[tuple[str, ...]], expected: list[tuple[str, ...]]) -> bool:
    norm = lambda rows: [tuple(numeric_value(v) for v in row) for row in rows]  # noqa: E731
    return same_rows(norm(actual), norm(expected))


_LANG_TAG = __import__("re").compile(r'^(".*")@([A-Za-z0-9-]+)$', __import__("re").S)


def rdf_term(term: str) -> str:
    """RDF 1.1 term identity for comparison: a language tag is case-INSENSITIVE
    (RDF 1.1 Concepts 3.3: tags may be lowercased, the value space is lower
    case). Applied to every system alike (harness fix, found on :strlang02)."""
    if (m := _LANG_TAG.match(term)):
        return f"{m.group(1)}@{m.group(2).lower()}"
    # RDF 1.1: a simple literal IS an xsd:string literal (harness fix, :ucase01).
    if term.endswith("^^<http://www.w3.org/2001/XMLSchema#string>"):
        return term[: -len("^^<http://www.w3.org/2001/XMLSchema#string>")]
    return term


def same_rows(actual: list[tuple[str, ...]], expected: list[tuple[str, ...]]) -> bool:
    actual = [tuple(rdf_term(v) for v in row) for row in actual]
    expected = [tuple(rdf_term(v) for v in row) for row in expected]
    if any(v.startswith("_:") for row in expected + actual for v in row):
        return ev.rows_equal_with_blank_nodes(actual, expected)
    return Counter(actual) == Counter(expected)


def expected_dump(factory, files: list[tuple[Path, str | None]], graph: str | None) -> list[tuple[str, ...]]:
    """The expected graph, loaded into a fresh instance of the SAME system."""
    with factory() as store:
        for path, into in files:
            store.load(path, into)
        return dump(store, graph)


def run_case(case: "ev.Case", factory) -> dict:
    base = {"id": case.identifier, "name": case.name, "manifest": str(case.manifest)}
    if reason := ev.unsupported_reason(case):
        return {**base, "status": "unsupported", "reason": reason}
    try:
        with factory() as store:
            for path in case.data:
                store.load(path, None)
            for path in case.graph_data:
                store.load(path, path.resolve().as_uri())
            for path, graph in case.update_graph_data:
                store.load(path, graph)
            if case.test_class == "update":
                assert case.update_request
                # Sent RAW, exactly as the quipu runner sends it (no BASE prefix).
                store.update(case.update_request.read_text())
                files = [(p, None) for p in case.expected_data] + list(case.expected_graph_data)
                graphs = {None, *(g for _, g in case.update_graph_data), *(g for _, g in case.expected_graph_data)}
                for graph in graphs:
                    if not same_rows(dump(store, graph), expected_dump(factory, files, graph)):
                        return {**base, "status": "failed",
                                "diagnostic": f"post-update graph differs: {graph or 'default'}"}
                return {**base, "status": "passed"}
            assert case.query and case.result
            query = f"BASE <{case.query.resolve().as_uri()}>\n{case.query.read_text()}"
            if case.result.suffix in {".ttl", ".nt"}:
                with tempfile.TemporaryDirectory() as tmp:
                    got = Path(tmp) / "actual.nt"
                    got.write_text(store.construct(query))
                    actual = expected_dump(factory, [(got, None)], None)
                expected = expected_dump(factory, [(case.result, None)], None)
                passed = same_rows(actual, expected)
            else:
                actual = store.select(query)
                expected = ev.expected_result(case.result)
                if isinstance(actual, bool) or isinstance(expected, bool):
                    passed = actual == expected
                else:
                    aligned = ev.reorder_rows(actual[0], actual[1], expected[0])
                    passed = aligned is not None and same_rows(aligned, expected[1])
                    if aligned is not None and not passed and same_rows_by_value(aligned, expected[1]):
                        return {**base, "status": "failed", "lexical_form_only": True,
                                "diagnostic": "same values, different numeric lexical form "
                                              "(fails RDF term equality)"}
    except (ValueError, RuntimeError, KeyError, json.JSONDecodeError, OSError, AssertionError) as error:
        return {**base, "status": "failed", "diagnostic": str(error)[:400]}
    except Exception as error:  # a competitor library raising its own type is a failed case, not a crash
        return {**base, "status": "failed", "diagnostic": f"{type(error).__name__}: {str(error)[:380]}"}
    return {**base, "status": "passed" if passed else "failed",
            **({} if passed else {"diagnostic": "actual result differs from expected multiset"})}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="cmd", required=True)
    prov = sub.add_parser("provision", help="download + verify a pinned artifact, print its path")
    prov.add_argument("name", choices=sorted(PINS))
    run = sub.add_parser("run", help="score one system on the pinned suite")
    run.add_argument("--system", choices=sorted(DRIVERS), required=True)
    run.add_argument("--suite", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--class", dest="classes", action="append", choices=CLASSES)
    run.add_argument("--limit", type=int)
    run.add_argument("--allow-unpinned-suite", action="store_true")
    args = parser.parse_args(argv)

    if args.cmd == "provision":
        print(provision(args.name))
        return 0

    revision = ev.git_output(args.suite, "rev-parse", "HEAD")
    dirty = bool(ev.git_output(args.suite, "status", "--porcelain"))
    if not args.allow_unpinned_suite and (revision != ev.PINNED_SUITE_REVISION or dirty):
        parser.error(f"suite must be clean at {ev.PINNED_SUITE_REVISION}; got {revision}")
    classes = args.classes or list(CLASSES)
    cases = [c for c in ev.discover_cases(args.suite) if c.test_class in classes]
    if args.limit:
        cases = cases[: args.limit]
    factory = DRIVERS[args.system]
    with factory() as probe:
        version = probe.version
    results = []
    for case in cases:
        row = run_case(case, factory)
        results.append({**row, "class": case.test_class})
    summary = {cls: dict(Counter(r["status"] for r in results if r["class"] == cls), cases=sum(
        1 for r in results if r["class"] == cls), lexical_form_only=sum(
        1 for r in results if r["class"] == cls and r.get("lexical_form_only"))) for cls in classes}
    ledger = {
        "benchmark": "w3c-sparql11-competitor",
        "system": args.system,
        "system_version": version,
        "artifact_sha256": PINS.get(args.system, {}).get("sha256"),
        "suite_revision": revision,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "timeout_s": TIMEOUT_S,
        "classes": summary,
        "results": results,
    }
    args.output.write_text(json.dumps(ledger, indent=2) + "\n")
    print(json.dumps({args.system: summary}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
