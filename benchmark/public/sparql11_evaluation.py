#!/usr/bin/env python3
"""Run pinned W3C SPARQL 1.1 classes against dedicated Quipu stores."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
from datetime import datetime, timezone
import re
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

PINNED_SUITE_REVISION = "369a90d1a60c021b746df2e411da0ff36258a758"
APPROVED = "dawgt:approval dawgt:Approved"
TYPE = re.compile(
    r"rdf:type\s+mf:(QueryEvaluationTest|UpdateEvaluationTest|ProtocolTest|CSVResultFormatTest)"
)
NAME = re.compile(r'mf:name\s+"((?:[^"\\]|\\.)*)"', re.S)
QUERY = re.compile(r"qt:query\s+<([^>]+)>")
DATA = re.compile(r"qt:data\s+<([^>]+)>")
GRAPH_DATA = re.compile(r"qt:graphData\s+<([^>]+)>")
RESULT = re.compile(r"mf:result\s+<([^>]+)>")
ENTAILMENT_REGIMES = re.compile(r"sd:entailmentRegime\s*\((.*?)\)", re.S)
ENTAILMENT_PROFILES = re.compile(r"sd:EntailmentProfile\s*\((.*?)\)", re.S)

CLASS_MANIFESTS = {
    "query-evaluation": "manifest-sparql11-query.ttl",
    "protocol": "protocol/manifest.ttl",
    "update": "manifest-sparql11-update.ttl",
    "entailment": "entailment/manifest.ttl",
    "result-format": "manifest-sparql11-results.ttl",
}

ENTAILMENT_BUCKET_IDS = {
    "RDF": "bind01 bind02 bind03 bind04 bind05 bind06 bind07 bind08 owlds02 paper-sparqldl-Q5 rdf01 rdf02 rdf03 rdf04 sparqldl-01 sparqldl-04".split(),
    "RDFS": "owlds01 paper-sparqldl-Q1-rdfs parent2 rdfs01 rdfs02 rdfs03 rdfs04 rdfs05 rdfs06 rdfs07 rdfs08 rdfs09 rdfs10 rdfs11 rdfs12 rdfs13 sparqldl-02 sparqldl-03 sparqldl-05".split(),
    "D": "d-ent-01 sparqldl-06".split(),
    "OWL-RDF-Based": "lang paper-sparqldl-Q1 paper-sparqldl-Q4 plainLit sparqldl-07 sparqldl-08 sparqldl-09 sparqldl-10 sparqldl-11 sparqldl-12 sparqldl-13".split(),
    "OWL-Direct": "paper-sparqldl-Q2 paper-sparqldl-Q3 parent10 parent3 parent4 parent5 parent6 parent7 parent8 parent9 simple1 simple2 simple3 simple4 simple5 simple6 simple7 simple8".split(),
    "RIF": "rif01 rif03 rif04 rif06".split(),
}
ENTAILMENT_BUCKET = {
    f":{identifier}": bucket
    for bucket, identifiers in ENTAILMENT_BUCKET_IDS.items()
    for identifier in identifiers
}
# The regime commitments, as DATA. Everything downstream derives from this map:
# the ledger records what it finds here, and the published table reads the ledger.
# Before aegis-1gp76j the string "deliberate-non-goal" was asserted as a constant
# in run_case() and printed as a literal by conformance_report.py, so the ledger
# did not RECORD a commitment -- it restated one, and no reader could tell.
ENTAILMENT_COMMITMENT = {
    # Answerable by RDFS closure over the query's default graph.
    "RDF": "goal",
    "RDFS": "goal",
    # Need a real DL/RL reasoner; quipu's owl layer is a write gate with zero
    # axioms today. Non-goal until the design (aegis-b5moll) is accepted.
    "OWL-Direct": "deliberate-non-goal",
    "OWL-RDF-Based": "deliberate-non-goal",
    # RIF is a rule-interchange format and D is datatype entailment; neither is
    # asked of quipu. Deliberate, not deferred.
    "RIF": "deliberate-non-goal",
    "D": "deliberate-non-goal",
}

# WHY each non-goal regime is a non-goal, in the words the published page shows
# (aegis-1gp76j item 3). This used to be `f"{bucket} entailment regime is not
# implemented"` for every bucket, which reads as a BACKLOG -- "not implemented
# yet" -- when the decision is that we are deliberately not implementing it. The
# reasoning existed only in the comment on ENTAILMENT_COMMITMENT above, i.e.
# where a reader of the conformance page never stands.
#
# Keep these keyed to ENTAILMENT_COMMITMENT: a bucket that becomes a `goal` must
# lose its reason, and the assertion below enforces that rather than trusting it.
ENTAILMENT_NON_GOAL_REASON = {
    "OWL-Direct": (
        "OWL-Direct entailment is a deliberate non-goal pending a design "
        "(aegis-b5moll): it needs a real DL reasoner, and this store's OWL layer "
        "is a write gate with no axioms -- no amount of RDFS closure reaches it"
    ),
    "OWL-RDF-Based": (
        "OWL-RDF-Based entailment is a deliberate non-goal pending the same "
        "design (aegis-b5moll): it needs an RL rule set or an external reasoner, "
        "not an extension of the RDFS closure"
    ),
    "RIF": (
        "RIF entailment is a deliberate non-goal: RIF is a rule-interchange "
        "format, not a semantics asked of this store"
    ),
    "D": (
        "D entailment (datatype entailment) is a deliberate non-goal: no "
        "consumer asks for datatype entailment beyond simple and RDFS"
    ),
}

ENTAILMENT_REASON = {
    bucket: ENTAILMENT_NON_GOAL_REASON[bucket]
    for bucket in ENTAILMENT_BUCKET_IDS
    if ENTAILMENT_COMMITMENT.get(bucket) != "goal"
}

# A non-goal without a stated reason is exactly the state item 3 existed to end,
# and a goal carrying one is a stale rationale that would be published as live.
# Both are caught here rather than at review time.
assert set(ENTAILMENT_REASON) == {
    bucket
    for bucket in ENTAILMENT_BUCKET_IDS
    if ENTAILMENT_COMMITMENT.get(bucket) != "goal"
}, "every non-goal regime needs a published reason, and only non-goals may have one"


# Buckets quipu can answer with a MATERIALISED closure, and the `--entailment`
# regime name that selects it.
#
# Separate from ENTAILMENT_COMMITMENT on purpose: a bucket can be a `goal` and
# still belong here only when a closure exists for it. RDF is a goal and is
# deliberately ABSENT — applying the RDFS closure to RDF-regime cases broke
# `owlds02` (15/16 -> 14/16) while fixing six RDFS ones, measured. A stronger
# regime is not a safer default.
#
# Adding a regime is a change to this dict. It used to be a bare `== "RDFS"`
# literal beside a `== "goal"` lookup, which meant a third goal regime would
# have been silently answered under simple entailment — a wrong number, not an
# error (wu, review of #150).
CLOSURE_REGIME = {"RDFS": "rdfs"}


@dataclass(frozen=True)
class Case:
    test_class: str
    manifest: Path
    identifier: str
    name: str
    kind: str
    query: Path | None
    data: tuple[Path, ...]
    graph_data: tuple[Path, ...]
    result: Path | None
    protocol_requests: tuple["ProtocolRequest", ...] = ()
    protocol_graph_data: tuple[tuple[Path, str], ...] = ()
    update_request: Path | None = None
    expected_data: tuple[Path, ...] = ()
    expected_graph_data: tuple[tuple[Path, str], ...] = ()
    update_graph_data: tuple[tuple[Path, str], ...] = ()
    entailment_regimes: tuple[str, ...] = ()
    entailment_profiles: tuple[str, ...] = ()


@dataclass(frozen=True)
class ProtocolRequest:
    method: str
    path: str
    content_type: str | None
    body: bytes | None
    status_family: int
    expected_boolean: bool | None
    expected_format: str | None


def git_output(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(cwd), *args], check=True, text=True, capture_output=True
    ).stdout.strip()


def executable_path(value: Path) -> Path:
    """Resolve an explicit path or a command available on PATH."""
    if value.parent == Path(".") and not value.exists():
        if located := shutil.which(str(value)):
            return Path(located).resolve()
        raise ValueError(f"executable not found: {value}")
    return value.resolve()


def turtle_statements(text: str) -> list[str]:
    """Split the pinned manifests at top-level Turtle statement terminators."""
    statements: list[str] = []
    start = 0
    square = round_ = 0
    quote: str | None = None
    iri = False
    escaped = comment = False
    index = 0
    while index < len(text):
        char = text[index]
        if comment:
            if char == "\n":
                comment = False
            index += 1
            continue
        if quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif text.startswith(quote, index):
                index += len(quote)
                quote = None
                continue
            index += 1
            continue
        if iri:
            if char == ">":
                iri = False
            index += 1
            continue
        if char == "#":
            comment = True
        elif text.startswith('"""', index) or text.startswith("'''", index):
            quote = text[index : index + 3]
            index += 3
            continue
        elif char in "\"'":
            quote = char
        elif char == "<":
            iri = True
        elif char == "[":
            square += 1
        elif char == "]":
            square -= 1
        elif char == "(":
            round_ += 1
        elif char == ")":
            round_ -= 1
        elif char == "." and square == 0 and round_ == 0:
            statements.append(text[start : index + 1])
            start = index + 1
        index += 1
    return statements


def includes(manifest: Path) -> list[Path]:
    text = manifest.read_text()
    match = re.search(r"mf:include\s*\((.*?)\)", text, re.S)
    if not match:
        return [manifest]
    return [manifest.parent / value for value in re.findall(r"<([^>]+)>", match.group(1))]


def resolve(base: Path, match: re.Match[str] | None) -> Path | None:
    return base / match.group(1) if match else None


def balanced_region(text: str, start: int, opening: str, closing: str) -> str:
    """Return one balanced Turtle collection/property-list, preserving literals."""
    depth = 0
    quote: str | None = None
    escaped = False
    index = start
    while index < len(text):
        if quote:
            if escaped:
                escaped = False
            elif text[index] == "\\":
                escaped = True
            elif text.startswith(quote, index):
                index += len(quote)
                quote = None
                continue
        elif text.startswith('"""', index) or text.startswith("'''", index):
            quote = text[index : index + 3]
            index += 3
            continue
        elif text[index] in "\"'":
            quote = text[index]
        elif text[index] == opening:
            depth += 1
        elif text[index] == closing:
            depth -= 1
            if depth == 0:
                return text[start : index + 1]
        index += 1
    raise ValueError(f"unterminated Turtle {opening}{closing} region")


def turtle_string(block: str, predicate: str) -> str | None:
    match = re.search(rf"{re.escape(predicate)}\s+(\"\"\"|'''|\"|')", block)
    if not match:
        return None
    quote = match.group(1)
    start = match.end()
    end = block.find(quote, start)
    if end < 0:
        raise ValueError(f"unterminated literal for {predicate}")
    return bytes(block[start:end], "utf-8").decode("unicode_escape")


def parse_protocol(statement: str, base: Path) -> tuple[tuple[ProtocolRequest, ...], tuple[tuple[Path, str], ...]]:
    graph_data = tuple(
        (base / path, label)
        for path, label in re.findall(
            r"ut:graphData\s*\[\s*ut:graph\s*<([^>]+)>\s*;\s*rdfs:label\s*\"([^\"]+)\"",
            statement,
            re.S,
        )
    )
    marker = re.search(r"ht:requests\s*\(", statement)
    if not marker:
        return (), graph_data
    collection = balanced_region(statement, marker.end() - 1, "(", ")")
    requests: list[ProtocolRequest] = []
    index = 0
    while (start := collection.find("[", index)) >= 0:
        block = balanced_region(collection, start, "[", "]")
        index = start + len(block)
        if "a ht:Request" not in block:
            continue
        method = turtle_string(block, "ht:methodName")
        path = turtle_string(block, "ht:absolutePath")
        if not method or path is None:
            raise ValueError("protocol request lacks method or absolute path")
        content_type = turtle_string(block, "ht:fieldValue")
        body = turtle_string(block, "cnt:chars")
        status = re.search(r"mf:expectedStatus\s+hts:StatusCode([2345])xx", block)
        expected_boolean = None
        if match := re.search(r"mf:expectedBoolean\s+(true|false)", block):
            expected_boolean = match.group(1) == "true"
        requests.append(
            ProtocolRequest(
                method=method,
                path=path,
                content_type=content_type,
                body=body.encode("utf-16") if content_type and "UTF-16" in content_type and body is not None else body.encode() if body is not None else None,
                status_family=int(status.group(1)) if status else 2,
                expected_boolean=expected_boolean,
                expected_format=turtle_string(block, "mf:expectedFormat"),
            )
        )
    return tuple(requests), graph_data


def property_block(statement: str, predicate: str) -> str:
    match = re.search(rf"{re.escape(predicate)}\s*\[", statement)
    return balanced_region(statement, match.end() - 1, "[", "]") if match else ""


def update_manifest_parts(statement: str, base: Path) -> tuple[Path | None, tuple[Path, ...], tuple[tuple[Path, str], ...], tuple[Path, ...], tuple[tuple[Path, str], ...]]:
    action = property_block(statement, "mf:action")
    result = property_block(statement, "mf:result")
    request = re.search(r"ut:request\s*<([^>]+)>", action)
    def data(block: str) -> tuple[Path, ...]:
        return tuple(base / item for item in re.findall(r"ut:data\s*<([^>]+)>", block))
    def graphs(block: str) -> tuple[tuple[Path, str], ...]:
        return tuple((base / path, label) for path, label in re.findall(
            r"ut:graphData\s*\[\s*ut:graph\s*<([^>]+)>\s*;\s*rdfs:label\s*\"([^\"]+)\"", block, re.S))
    return (base / request.group(1) if request else None, data(action), graphs(action), data(result), graphs(result))


def parse_manifest(test_class: str, manifest: Path) -> list[Case]:
    cases: list[Case] = []
    for statement in turtle_statements(manifest.read_text()):
        kind = TYPE.search(statement)
        if not kind or APPROVED not in statement:
            continue
        subject = statement.lstrip().split(None, 1)[0]
        name = NAME.search(statement)
        graph_data = tuple(manifest.parent / item for item in GRAPH_DATA.findall(statement))
        protocol_requests, protocol_graph_data = parse_protocol(statement, manifest.parent)
        update_request, update_data, update_graphs, expected_data, expected_graphs = update_manifest_parts(statement, manifest.parent)
        regimes = ENTAILMENT_REGIMES.search(statement)
        profiles = ENTAILMENT_PROFILES.search(statement)
        cases.append(
            Case(
                test_class=test_class,
                manifest=manifest,
                identifier=subject,
                name=bytes(name.group(1), "utf-8").decode("unicode_escape") if name else subject,
                kind=kind.group(1),
                query=resolve(manifest.parent, QUERY.search(statement)),
                data=update_data if kind.group(1) == "UpdateEvaluationTest" else tuple(manifest.parent / item for item in DATA.findall(statement)),
                graph_data=() if kind.group(1) == "UpdateEvaluationTest" else graph_data,
                result=resolve(manifest.parent, RESULT.search(statement)),
                protocol_requests=protocol_requests,
                protocol_graph_data=protocol_graph_data,
                update_request=update_request,
                expected_data=expected_data,
                expected_graph_data=expected_graphs,
                update_graph_data=update_graphs,
                entailment_regimes=tuple(re.findall(r"ent:([A-Za-z0-9-]+)", regimes.group(1))) if regimes else (),
                entailment_profiles=tuple(re.findall(r"pr:([A-Za-z0-9-]+)", profiles.group(1))) if profiles else (),
            )
        )
    return cases


def discover_cases(suite: Path) -> list[Case]:
    cases: list[Case] = []
    seen: set[tuple[str, Path, str]] = set()
    for test_class, relative in CLASS_MANIFESTS.items():
        root = suite / relative
        for manifest in includes(root):
            for case in parse_manifest(test_class, manifest):
                key = (test_class, manifest, case.identifier)
                if key not in seen:
                    seen.add(key)
                    cases.append(case)
    return cases


def term(kind: str, value: str, datatype: str | None = None, lang: str | None = None) -> str:
    if kind == "uri":
        # `quipu read` renders resolved references as their bare IRI. Keep the
        # expected side in that same machine-comparison representation; angle
        # brackets are Turtle syntax, not part of the RDF term's lexical value.
        return value
    if kind == "bnode":
        return f"_:{value}"
    escaped = value.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
    if lang:
        return f'"{escaped}"@{lang}'
    if datatype:
        integer = "http://www.w3.org/2001/XMLSchema#integer"
        boolean = "http://www.w3.org/2001/XMLSchema#boolean"
        if datatype in {integer, boolean}:
            return value
        return f'"{escaped}"^^<{datatype}>'
    return f'"{escaped}"'


def expected_json(path: Path) -> tuple[list[str], list[tuple[str, ...]]] | bool:
    value = json.loads(path.read_text())
    if "boolean" in value:
        return bool(value["boolean"])
    variables = value.get("head", {}).get("vars", [])
    rows = []
    for binding in value.get("results", {}).get("bindings", []):
        rows.append(
            tuple(
                term(item["type"], item["value"], item.get("datatype"), item.get("xml:lang"))
                if (item := binding.get(variable))
                else "(unbound)"
                for variable in variables
            )
        )
    return variables, rows


def expected_xml(path: Path) -> tuple[list[str], list[tuple[str, ...]]] | bool:
    root = ET.parse(path).getroot()
    ns = {"s": "http://www.w3.org/2005/sparql-results#"}
    boolean = root.find("s:boolean", ns)
    if boolean is not None:
        return (boolean.text or "").strip().lower() == "true"
    variables = [item.attrib["name"] for item in root.findall("s:head/s:variable", ns)]
    rows = []
    for result in root.findall("s:results/s:result", ns):
        bindings = {item.attrib["name"]: item for item in result.findall("s:binding", ns)}
        row = []
        for variable in variables:
            binding = bindings.get(variable)
            if binding is None or len(binding) == 0:
                row.append("(unbound)")
                continue
            value = binding[0]
            kind = value.tag.rsplit("}", 1)[-1]
            row.append(
                term(
                    "uri" if kind == "uri" else "bnode" if kind == "bnode" else "literal",
                    value.text or "",
                    value.attrib.get("datatype"),
                    value.attrib.get("{http://www.w3.org/XML/1998/namespace}lang"),
                )
            )
        rows.append(tuple(row))
    return variables, rows


def expected_delimited(path: Path) -> tuple[list[str], list[tuple[str, ...]]]:
    delimiter = "\t" if path.suffix == ".tsv" else ","
    with path.open(newline="") as handle:
        parsed = list(csv.reader(handle, delimiter=delimiter))
    variables = [value.lstrip("?") for value in parsed[0]]
    rows = [tuple(value if value else "(unbound)" for value in row) for row in parsed[1:]]
    return variables, rows


def delimited_result(body: bytes, suffix: str) -> tuple[list[str], list[tuple[str, ...]]]:
    """Parse an observed CSV/TSV representation without erasing term spelling."""
    delimiter = "\t" if suffix == ".tsv" else ","
    parsed = list(csv.reader(body.decode("utf-8").splitlines(), delimiter=delimiter))
    if not parsed:
        raise ValueError("HTTP result body is empty")
    variables = [value.lstrip("?") for value in parsed[0]]
    return variables, [tuple(value if value else "(unbound)" for value in row) for row in parsed[1:]]


def reserve_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def run_result_format_case(
    case: Case, quipu: Path, server: Path, database: Path
) -> tuple[list[str], list[tuple[str, ...]]] | bool:
    """Exercise the negotiated HTTP serializer used by real SPARQL clients."""
    assert case.query and case.result
    port = reserve_port()
    process = subprocess.Popen(
        [str(server), "--db", str(database), "--bind", f"127.0.0.1:{port}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        health = f"http://127.0.0.1:{port}/health"
        for _ in range(100):
            try:
                with urllib.request.urlopen(health, timeout=0.2):
                    break
            except (urllib.error.URLError, TimeoutError):
                if process.poll() is not None:
                    raise ValueError(f"quipu-server exited before health: {process.stderr.read().strip()}")
                time.sleep(0.02)
        else:
            raise ValueError("quipu-server did not become healthy")

        suffix = case.result.suffix
        accept = {
            ".csv": "text/csv",
            ".tsv": "text/tab-separated-values",
            ".srj": "application/sparql-results+json",
            ".srx": "application/sparql-results+xml",
        }[suffix]
        query = f"BASE <{case.query.resolve().as_uri()}>\n{case.query.read_text()}".encode()
        request = urllib.request.Request(
            f"http://127.0.0.1:{port}/query",
            data=query,
            headers={"Content-Type": "application/sparql-query", "Accept": accept},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=10) as response:
            if response.headers.get_content_type() != accept:
                raise ValueError(
                    f"expected Content-Type {accept}, got {response.headers.get('Content-Type')}"
                )
            body = response.read()
        if suffix in {".csv", ".tsv"}:
            return delimited_result(body, suffix)
        observed_path = database.with_suffix(suffix)
        observed_path.write_bytes(body)
        return expected_json(observed_path) if suffix == ".srj" else expected_xml(observed_path)
    finally:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def run_protocol_case(case: Case, server: Path, database: Path) -> None:
    """Execute every HTTP request in one approved ProtocolTest in manifest order."""
    port = reserve_port()
    process = subprocess.Popen(
        [str(server), "--db", str(database), "--bind", f"127.0.0.1:{port}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        health = f"http://127.0.0.1:{port}/health"
        for _ in range(100):
            try:
                with urllib.request.urlopen(health, timeout=0.2):
                    break
            except (urllib.error.URLError, TimeoutError):
                if process.poll() is not None:
                    raise ValueError(f"quipu-server exited before health: {process.stderr.read().strip()}")
                time.sleep(0.02)
        else:
            raise ValueError("quipu-server did not become healthy")

        if not case.protocol_requests:
            raise ValueError("manifest protocol case contains no HTTP requests")
        for request_spec in case.protocol_requests:
            query = urllib.parse.urlsplit(request_spec.path).query
            fields = urllib.parse.parse_qs(query)
            is_update = "update" in fields or (
                request_spec.content_type == "application/sparql-update"
                or request_spec.content_type == "application/sparql-update; charset=UTF-16"
                or request_spec.body is not None
                and request_spec.content_type == "application/x-www-form-urlencoded"
                and b"update=" in request_spec.body
            )
            endpoint = "/update" if is_update else "/query"
            suffix = "?" + query if query else ""
            headers = {}
            if request_spec.content_type:
                headers["Content-Type"] = request_spec.content_type
            elif request_spec.body is not None:
                # urllib otherwise invents application/x-www-form-urlencoded,
                # defeating the manifest's missing-Content-Type negative case.
                headers["Content-Type"] = ""
            http_request = urllib.request.Request(
                f"http://127.0.0.1:{port}{endpoint}{suffix}",
                data=request_spec.body,
                headers=headers,
                method=request_spec.method,
            )
            try:
                response = urllib.request.urlopen(http_request, timeout=10)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                status = response.status
                body = response.read()
                content_type = response.headers.get_content_type()
            if status // 100 != request_spec.status_family:
                raise ValueError(
                    f"{request_spec.method} {endpoint} expected {request_spec.status_family}xx, got {status}: {body.decode(errors='replace')}"
                )
            if request_spec.expected_boolean is not None:
                try:
                    observed = bool(json.loads(body)["boolean"])
                except (json.JSONDecodeError, KeyError, TypeError) as error:
                    raise ValueError(f"expected SPARQL boolean response, got {content_type}") from error
                if observed != request_spec.expected_boolean:
                    raise ValueError(
                        f"expected boolean {request_spec.expected_boolean}, got {observed}"
                    )
            formats = {
                "boolean": {"application/sparql-results+json", "application/sparql-results+xml"},
                "tabular": {"application/sparql-results+json", "application/sparql-results+xml", "text/csv", "text/tab-separated-values"},
                "RDF": {"text/turtle", "application/n-triples"},
            }
            if request_spec.expected_format and content_type not in formats[request_spec.expected_format]:
                raise ValueError(
                    f"expected {request_spec.expected_format} response, got {content_type}"
                )
    finally:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def read_graph(quipu: Path, database: Path, graph: str | None) -> Counter[tuple[str, str, str]]:
    query = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"
    if graph:
        query = f"SELECT ?s ?p ?o FROM <{graph}> WHERE {{ ?s ?p ?o }}"
    selected = subprocess.run([str(quipu), "read", query, "--db", str(database)], text=True, capture_output=True)
    if selected.returncode or "query error:" in selected.stderr:
        raise ValueError(selected.stderr.strip())
    variables, rows = actual_result(selected.stdout)
    aligned = reorder_rows(variables, rows, ["s", "p", "o"])
    if aligned is None:
        raise ValueError("dataset comparison emitted unexpected bindings")
    return Counter(aligned)


def run_update_case(case: Case, quipu: Path, server: Path, database: Path, temporary: Path) -> None:
    assert case.update_request
    port = reserve_port()
    process = subprocess.Popen([str(server), "--db", str(database), "--bind", f"127.0.0.1:{port}"], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    try:
        for _ in range(100):
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=0.2): break
            except (urllib.error.URLError, TimeoutError): time.sleep(0.02)
        request = urllib.request.Request(f"http://127.0.0.1:{port}/update", data=case.update_request.read_bytes(), headers={"Content-Type": "application/sparql-update"}, method="POST")
        with urllib.request.urlopen(request, timeout=20) as response:
            response.read()
        expected_db = temporary / "expected-update.db"
        for fixture in case.expected_data:
            loaded = subprocess.run([str(quipu), "knot", str(fixture), "--db", str(expected_db)], text=True, capture_output=True)
            if loaded.returncode: raise ValueError(loaded.stderr.strip())
        for fixture, graph in case.expected_graph_data:
            loaded = subprocess.run([str(quipu), "knot", str(fixture), "--graph", graph, "--db", str(expected_db)], text=True, capture_output=True)
            if loaded.returncode: raise ValueError(loaded.stderr.strip())
        graphs = {None, *(graph for _, graph in case.update_graph_data), *(graph for _, graph in case.expected_graph_data)}
        for graph in graphs:
            if read_graph(quipu, database, graph) != read_graph(quipu, expected_db, graph):
                raise ValueError(f"post-update graph differs from expected dataset: {graph or 'default'}")
    finally:
        process.terminate()
        try: process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait()


def expected_result(path: Path) -> tuple[list[str], list[tuple[str, ...]]] | bool:
    if path.suffix == ".srj":
        return expected_json(path)
    if path.suffix == ".srx":
        return expected_xml(path)
    if path.suffix in {".csv", ".tsv"}:
        return expected_delimited(path)
    raise ValueError(f"unsupported expected result format {path.suffix}")


def actual_result(stdout: str) -> tuple[list[str], list[tuple[str, ...]]] | bool:
    stripped = stdout.strip()
    if stripped in {"true", "false"}:
        return stripped == "true"
    lines = stdout.splitlines()
    if len(lines) < 3:
        raise ValueError("query emitted no result table")
    variables = lines[0].split("\t") if lines[0] else []
    rows = []
    reported_count = None
    for line in lines[2:]:
        if match := re.fullmatch(r"(\d+) results", line):
            reported_count = int(match.group(1))
            continue
        if not line:
            continue
        rows.append(tuple(line.split("\t")))
    if not variables and reported_count is not None:
        rows = [()] * reported_count
    return variables, rows


def actual_graph(stdout: str) -> Counter[tuple[str, str, str]]:
    """Parse the CLI's machine-stable tab-separated CONSTRUCT rendering."""
    triples: Counter[tuple[str, str, str]] = Counter()
    reported_count = None
    for line in stdout.splitlines():
        if match := re.fullmatch(r"(\d+) triples", line):
            reported_count = int(match.group(1))
        elif line:
            values = line.split("\t")
            if len(values) != 3:
                raise ValueError("graph result emitted a malformed triple row")
            triples[tuple(values)] += 1
    if reported_count is None or reported_count != sum(triples.values()):
        raise ValueError("graph result count does not match emitted triples")
    return triples


def expected_graph(path: Path, quipu: Path, database: Path) -> Counter[tuple[str, str, str]]:
    """Parse an RDF expected result through Quipu's own pinned RDF loader."""
    loaded = subprocess.run(
        [str(quipu), "knot", str(path), "--db", str(database)],
        text=True,
        capture_output=True,
    )
    if loaded.returncode:
        raise ValueError(loaded.stderr.strip())
    selected = subprocess.run(
        [str(quipu), "read", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "--db", str(database)],
        text=True,
        capture_output=True,
    )
    if selected.returncode or "query error:" in selected.stderr:
        raise ValueError(selected.stderr.strip())
    variables, rows = actual_result(selected.stdout)
    aligned = reorder_rows(variables, rows, ["s", "p", "o"])
    if aligned is None:
        raise ValueError("expected graph parser emitted unexpected bindings")
    return Counter(aligned)


def reorder_rows(
    variables: list[str], rows: list[tuple[str, ...]], expected_variables: list[str]
) -> list[tuple[str, ...]] | None:
    """Align bindings by variable name; SPARQL result column order is immaterial."""
    if set(variables) != set(expected_variables):
        return None
    positions = {variable: index for index, variable in enumerate(variables)}
    return [tuple(row[positions[variable]] for variable in expected_variables) for row in rows]


def rows_equal_with_blank_nodes(
    actual: list[tuple[str, ...]], expected: list[tuple[str, ...]]
) -> bool:
    """Compare unordered result rows modulo one global blank-node renaming."""
    if len(actual) != len(expected):
        return False

    def match(
        remaining: list[tuple[str, ...]],
        index: int,
        forward: dict[str, str],
        reverse: dict[str, str],
    ) -> bool:
        if index == len(actual):
            return True
        row = actual[index]
        for candidate_index, candidate in enumerate(remaining):
            if len(row) != len(candidate):
                continue
            next_forward = forward.copy()
            next_reverse = reverse.copy()
            compatible = True
            for observed, wanted in zip(row, candidate, strict=True):
                observed_blank = observed.startswith("_:")
                wanted_blank = wanted.startswith("_:")
                if observed_blank != wanted_blank:
                    compatible = False
                    break
                if not observed_blank:
                    if observed != wanted:
                        compatible = False
                        break
                    continue
                if next_forward.get(observed, wanted) != wanted or next_reverse.get(
                    wanted, observed
                ) != observed:
                    compatible = False
                    break
                next_forward[observed] = wanted
                next_reverse[wanted] = observed
            if compatible and match(
                remaining[:candidate_index] + remaining[candidate_index + 1 :],
                index + 1,
                next_forward,
                next_reverse,
            ):
                return True
        return False

    return match(expected, 0, {}, {})


def normalize_delimited_numeric(value: str) -> str:
    """Ignore the case of the exponent marker permitted by SPARQL TSV."""
    if re.fullmatch(r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)[eE][+-]?\d+", value):
        return value.lower()
    return value


def unsupported_reason(case: Case) -> str | None:
    if case.test_class == "entailment":
        bucket = ENTAILMENT_BUCKET.get(case.identifier, "")
        if ENTAILMENT_COMMITMENT.get(bucket) == "goal":
            # Falls through and RUNS. A goal regime scored from a short-circuit
            # is 0/n by construction, which reads as a failing engine rather
            # than as a runner that never asked it anything.
            return None
        return ENTAILMENT_REASON.get(bucket, "unknown entailment regime is not implemented")
    if case.kind not in {"QueryEvaluationTest", "CSVResultFormatTest", "ProtocolTest", "UpdateEvaluationTest"}:
        return f"test kind {case.kind} is not executable by this runner"
    if case.test_class not in {"protocol", "update"} and not case.query:
        return "manifest action has no qt:query"
    if case.test_class not in {"protocol", "update"} and not case.result:
        return "manifest case has no expected mf:result"
    if case.result and case.result.suffix not in {".srj", ".srx", ".csv", ".tsv", ".ttl", ".nt"}:
        return f"expected result format {case.result.suffix or '<none>'} is not comparable"
    return None


def provenance() -> tuple[str, str]:
    """When this ledger was produced, and BY WHAT.

    `generated_at` alone is forgeable by accident: a local re-derive stamps it
    exactly as CI does, so a reader could not tell a page backed by the pinned
    runner from one backed by somebody's laptop. `generated_by` carries the CI
    run URL when GitHub Actions produced it and the literal "local" otherwise,
    which turns "only CI-produced ledgers go on the page" from a convention
    nobody can check into a property of the ARTIFACT (aegis-1gp76j).

    This is not hypothetical bookkeeping: a locally-run ledger takes
    `quipu_revision` from the repo HEAD and `quipu_version` from whatever binary
    was to hand, so it can credit a commit that never produced it.
    """
    at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    server = os.environ.get("GITHUB_SERVER_URL")
    repo = os.environ.get("GITHUB_REPOSITORY")
    run_id = os.environ.get("GITHUB_RUN_ID")
    if server and repo and run_id:
        return at, f"{server}/{repo}/actions/runs/{run_id}"
    return at, "local"


def run_case(case: Case, quipu: Path, server: Path) -> dict[str, object]:
    base = {
        "id": case.identifier,
        "name": case.name,
        "manifest": str(case.manifest),
        "query": str(case.query) if case.query else None,
        "result": str(case.result) if case.result else None,
    }
    if case.test_class == "entailment":
        base.update(
            entailment_regimes=list(case.entailment_regimes),
            entailment_profiles=list(case.entailment_profiles),
            decision_bucket=ENTAILMENT_BUCKET.get(case.identifier),
            commitment=ENTAILMENT_COMMITMENT.get(
                ENTAILMENT_BUCKET.get(case.identifier, ""), "deliberate-non-goal"
            ),
        )
    if reason := unsupported_reason(case):
        return {**base, "status": "unsupported", "reason": reason}
    with tempfile.TemporaryDirectory(prefix="quipu-w3c-eval-") as temporary:
        database = Path(temporary) / "suite.db"
        for fixture in case.data:
            loaded = subprocess.run(
                [str(quipu), "knot", str(fixture), "--db", str(database)],
                text=True,
                capture_output=True,
            )
            if loaded.returncode:
                return {**base, "status": "error", "diagnostic": loaded.stderr.strip()}
        for fixture in case.graph_data:
            loaded = subprocess.run(
                [
                    str(quipu),
                    "knot",
                    str(fixture),
                    "--graph",
                    fixture.resolve().as_uri(),
                    "--db",
                    str(database),
                ],
                text=True,
                capture_output=True,
            )
            if loaded.returncode:
                return {**base, "status": "error", "diagnostic": loaded.stderr.strip()}
        for fixture, graph in case.protocol_graph_data:
            loaded = subprocess.run(
                [str(quipu), "knot", str(fixture), "--graph", graph, "--db", str(database)],
                text=True,
                capture_output=True,
            )
            if loaded.returncode:
                return {**base, "status": "error", "diagnostic": loaded.stderr.strip()}
        for fixture, graph in case.update_graph_data:
            loaded = subprocess.run([str(quipu), "knot", str(fixture), "--graph", graph, "--db", str(database)], text=True, capture_output=True)
            if loaded.returncode:
                return {**base, "status": "error", "diagnostic": loaded.stderr.strip()}
        try:
            if case.test_class == "protocol":
                run_protocol_case(case, server, database)
                return {**base, "status": "passed"}
            if case.test_class == "update":
                run_update_case(case, quipu, server, database, Path(temporary))
                return {**base, "status": "passed"}
            assert case.query and case.result
            query = f"BASE <{case.query.resolve().as_uri()}>\n{case.query.read_text()}"
            if case.test_class == "result-format":
                actual = run_result_format_case(case, quipu, server, database)
                expected = expected_result(case.result)
            else:
                # A goal entailment regime is ANSWERED under that regime: the
                # engine composes the graph with its materialised closure. The
                # QUERY TEXT is untouched -- rewriting it to add
                # `FROM <g> FROM <g#inferred>` would be easy, local, and would
                # make the published number describe a query the suite never
                # asked (aegis-1gp76j).
                read_argv = [str(quipu), "read", query, "--db", str(database)]
                # RDFS closure applies to the RDFS REGIME ONLY. RDF entailment
                # does not include the rdfs2/3/7/9 rules, so applying them to an
                # RDF-regime case OVER-entails: measured, it broke `owlds02`
                # (RDF bucket, 15/16 -> 14/16) while fixing six RDFS cases. The
                # regime names which closure is licensed, and a stronger one is
                # not a safer default.
                bucket = ENTAILMENT_BUCKET.get(case.identifier, "")
                regime = CLOSURE_REGIME.get(bucket)
                if (
                    case.test_class == "entailment"
                    and regime is not None
                    and ENTAILMENT_COMMITMENT.get(bucket) == "goal"
                ):
                    read_argv += ["--entailment", regime]
                observed = subprocess.run(
                    read_argv,
                    text=True,
                    capture_output=True,
                )
                if "query error:" in observed.stderr:
                    return {**base, "status": "failed", "diagnostic": observed.stderr.strip()}
                if case.result.suffix in {".ttl", ".nt"}:
                    actual = actual_graph(observed.stdout)
                    expected = expected_graph(
                        case.result, quipu, Path(temporary) / "expected.db"
                    )
                else:
                    actual = actual_result(observed.stdout)
                    expected = expected_result(case.result)
        except (ET.ParseError, ValueError, KeyError, json.JSONDecodeError) as error:
            return {**base, "status": "error", "diagnostic": str(error)}
        if isinstance(actual, Counter) and isinstance(expected, Counter):
            passed = actual == expected
        elif isinstance(actual, bool) or isinstance(expected, bool):
            passed = actual == expected
        else:
            actual_vars, actual_rows = actual
            expected_vars, expected_rows = expected
            aligned_rows = reorder_rows(actual_vars, actual_rows, expected_vars)
            if aligned_rows is not None and case.test_class == "result-format":
                aligned_rows = [
                    tuple(normalize_delimited_numeric(value) for value in row)
                    for row in aligned_rows
                ]
                expected_rows = [
                    tuple(normalize_delimited_numeric(value) for value in row)
                    for row in expected_rows
                ]
            passed = aligned_rows is not None and (
                rows_equal_with_blank_nodes(aligned_rows, expected_rows)
                if any(value.startswith("_:") for row in expected_rows for value in row)
                else Counter(aligned_rows) == Counter(expected_rows)
            )
        return {
            **base,
            "status": "passed" if passed else "failed",
            "diagnostic": "" if passed else "actual result differs from expected multiset",
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path)
    parser.add_argument("--quipu", default="quipu", type=Path)
    parser.add_argument("--server", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--class", dest="classes", action="append", choices=CLASS_MANIFESTS)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--allow-unpinned-suite", action="store_true")
    args = parser.parse_args()
    try:
        args.quipu = executable_path(args.quipu)
        args.server = executable_path(args.server or args.quipu.with_name("quipu-server"))
    except ValueError as error:
        parser.error(str(error))

    revision = git_output(args.suite, "rev-parse", "HEAD")
    dirty = git_output(args.suite, "status", "--porcelain")
    if not args.allow_unpinned_suite and (revision != PINNED_SUITE_REVISION or dirty):
        parser.error(f"suite must be clean at {PINNED_SUITE_REVISION}; got {revision}")
    cases = discover_cases(args.suite)
    if args.classes:
        cases = [case for case in cases if case.test_class in args.classes]
    if args.limit is not None:
        cases = cases[: args.limit]
    if not cases:
        parser.error("selected manifests produced zero approved cases")

    version = subprocess.run(
        [str(args.quipu), "--version"], check=True, text=True, capture_output=True
    ).stdout.strip()
    quipu_root = Path(__file__).resolve().parents[2]
    results = [
        {"class": case.test_class, **run_case(case, args.quipu, args.server)} for case in cases
    ]
    for item in results:
        for field in ("manifest", "query", "result"):
            if item[field]:
                item[field] = str(Path(str(item[field])).relative_to(args.suite))
    classes = {}
    for test_class in CLASS_MANIFESTS:
        selected = [item for item in results if item["class"] == test_class]
        if not selected:
            continue
        counts = Counter(item["status"] for item in selected)
        classes[test_class] = {"cases": len(selected), **dict(sorted(counts.items()))}
        if test_class == "entailment":
            discovered = {item["id"] for item in selected}
            expected = set(ENTAILMENT_BUCKET)
            if discovered != expected or len(selected) != 70:
                missing = sorted(expected - discovered)
                extra = sorted(discovered - expected)
                parser.error(f"entailment inventory mismatch: missing={missing}, extra={extra}, cases={len(selected)}")
            bucket_counts = Counter(item["decision_bucket"] for item in selected)
            classes[test_class]["decision_buckets"] = dict(sorted(bucket_counts.items()))
    generated_at, generated_by = provenance()
    report = {
        "benchmark": "W3C RDF Tests SPARQL 1.1 evaluation classes",
        "suite_revision": revision,
        "quipu_revision": git_output(quipu_root, "rev-parse", "HEAD"),
        "quipu_version": version,
        "generated_at": generated_at,
        "generated_by": generated_by,
        "isolation": "one temporary SQLite store per executable test",
        "reproduce": {
            "build": "cargo build --release --bin quipu --bin quipu-server --features shacl,onnx,server",
            "runner": "python3 benchmark/public/sparql11_evaluation.py",
            "environment": {
                "SUITE": "/tmp/rdf-tests/sparql/sparql11",
                "QUIPU_BIN": "$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"target_directory\"])')/release/quipu",
            },
            "per_class": {
                test_class: (
                    "python3 benchmark/public/sparql11_evaluation.py "
                    "--suite \"$SUITE\" --quipu \"$QUIPU_BIN\" "
                    f"--class {test_class} --output /tmp/sparql11-{test_class}.json"
                )
                for test_class in CLASS_MANIFESTS
            },
        },
        "classes": classes,
        "results": results,
    }
    if set(classes) == {"entailment"}:
        inventory = sorted(
            (
                item["id"], item["manifest"], sorted(item["entailment_regimes"]),
                sorted(item["entailment_profiles"]), item["decision_bucket"],
            )
            for item in results
        )
        report["inventory_digest"] = hashlib.sha256(
            json.dumps(inventory, separators=(",", ":")).encode()
        ).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(classes, sort_keys=True))
    # A failing CASE is a RESULT, not a broken run. Exit nonzero only when the
    # harness itself could not answer (`error`).
    #
    # This mattered the moment RDF/RDFS became goals (aegis-1gp76j). Before
    # that every entailment case was `unsupported`, never `failed`, so this
    # line returned 0 and the workflow step ran on. Turning the regimes into
    # goals produced 6 genuine `failed` cases — and because the workflow step
    # is `bash -e`, a nonzero exit here KILLS THE STEP BEFORE
    # `check_regression.py` runs. The ledger would still upload, the job would
    # be red forever, and the entailment regression gate — the whole point of
    # the ledger — would never execute again.
    #
    # So the exit code stopped meaning "the run broke" and started meaning
    # "quipu is not yet fully conformant", which is true, permanent, and
    # already recorded in the ledger. Regressions are `check_regression.py`'s
    # job: it compares against the baseline and names the class that moved.
    # This exit code cannot do that and should not try.
    return 1 if any(item["status"] == "error" for item in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
