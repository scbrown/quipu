#!/usr/bin/env python3
"""Run pinned W3C SPARQL 1.1 classes against dedicated Quipu stores."""

from __future__ import annotations

import argparse
import csv
import json
import re
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

PINNED_SUITE_REVISION = "369a90d1a60c021b746df2e411da0ff36258a758"
APPROVED = "dawgt:approval dawgt:Approved"
TYPE = re.compile(r"rdf:type\s+mf:(QueryEvaluationTest|UpdateEvaluationTest|ProtocolTest)")
NAME = re.compile(r'mf:name\s+"((?:[^"\\]|\\.)*)"', re.S)
QUERY = re.compile(r"qt:query\s+<([^>]+)>")
DATA = re.compile(r"qt:data\s+<([^>]+)>")
GRAPH_DATA = re.compile(r"qt:graphData\s+<([^>]+)>")
RESULT = re.compile(r"mf:result\s+<([^>]+)>")

CLASS_MANIFESTS = {
    "query-evaluation": "manifest-sparql11-query.ttl",
    "protocol": "protocol/manifest.ttl",
    "update": "manifest-sparql11-update.ttl",
    "entailment": "entailment/manifest.ttl",
    "result-format": "manifest-sparql11-results.ttl",
}


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


def parse_manifest(test_class: str, manifest: Path) -> list[Case]:
    cases: list[Case] = []
    for statement in turtle_statements(manifest.read_text()):
        kind = TYPE.search(statement)
        if not kind or APPROVED not in statement:
            continue
        subject = statement.lstrip().split(None, 1)[0]
        name = NAME.search(statement)
        graph_data = tuple(manifest.parent / item for item in GRAPH_DATA.findall(statement))
        cases.append(
            Case(
                test_class=test_class,
                manifest=manifest,
                identifier=subject,
                name=bytes(name.group(1), "utf-8").decode("unicode_escape") if name else subject,
                kind=kind.group(1),
                query=resolve(manifest.parent, QUERY.search(statement)),
                data=tuple(manifest.parent / item for item in DATA.findall(statement)),
                graph_data=graph_data,
                result=resolve(manifest.parent, RESULT.search(statement)),
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
        return f"<{value}>"
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
    variables = lines[0].split("\t")
    rows = []
    for line in lines[2:]:
        if not line or re.fullmatch(r"\d+ results", line):
            continue
        rows.append(tuple(line.split("\t")))
    return variables, rows


def unsupported_reason(case: Case) -> str | None:
    if case.test_class == "protocol":
        return "W3C HTTP request-sequence executor is not implemented"
    if case.test_class == "update":
        return "Quipu exposes no SPARQL Update execution surface"
    if case.test_class == "entailment":
        return "manifest entailment-regime setup is not implemented"
    if case.kind != "QueryEvaluationTest":
        return f"test kind {case.kind} is not executable by this runner"
    if case.graph_data:
        return "named-graph fixture loading is not implemented"
    if not case.query:
        return "manifest action has no qt:query"
    if not case.result:
        return "manifest case has no expected mf:result"
    if case.result.suffix not in {".srj", ".srx", ".csv", ".tsv"}:
        return f"expected result format {case.result.suffix or '<none>'} is not comparable"
    return None


def run_case(case: Case, quipu: Path) -> dict[str, object]:
    base = {
        "id": case.identifier,
        "name": case.name,
        "manifest": str(case.manifest),
        "query": str(case.query) if case.query else None,
        "result": str(case.result) if case.result else None,
    }
    if reason := unsupported_reason(case):
        return {**base, "status": "unsupported", "reason": reason}
    assert case.query and case.result
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
        query = case.query.read_text()
        observed = subprocess.run(
            [str(quipu), "read", query, "--db", str(database)],
            text=True,
            capture_output=True,
        )
        if "query error:" in observed.stderr:
            return {**base, "status": "failed", "diagnostic": observed.stderr.strip()}
        try:
            actual = actual_result(observed.stdout)
            expected = expected_result(case.result)
        except (ET.ParseError, ValueError, KeyError, json.JSONDecodeError) as error:
            return {**base, "status": "error", "diagnostic": str(error)}
        if isinstance(actual, bool) or isinstance(expected, bool):
            passed = actual == expected
        else:
            actual_vars, actual_rows = actual
            expected_vars, expected_rows = expected
            if any(value.startswith("_:") for row in expected_rows for value in row):
                return {**base, "status": "unsupported", "reason": "blank-node isomorphism is not implemented"}
            passed = actual_vars == expected_vars and Counter(actual_rows) == Counter(expected_rows)
        return {
            **base,
            "status": "passed" if passed else "failed",
            "diagnostic": "" if passed else "actual result differs from expected multiset",
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path)
    parser.add_argument("--quipu", default="quipu", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--class", dest="classes", action="append", choices=CLASS_MANIFESTS)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--allow-unpinned-suite", action="store_true")
    args = parser.parse_args()
    try:
        args.quipu = executable_path(args.quipu)
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
    results = [{"class": case.test_class, **run_case(case, args.quipu)} for case in cases]
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
    report = {
        "benchmark": "W3C RDF Tests SPARQL 1.1 evaluation classes",
        "suite_revision": revision,
        "quipu_revision": git_output(quipu_root, "rev-parse", "HEAD"),
        "quipu_version": version,
        "isolation": "one temporary SQLite store per executable test",
        "reproduce": {
            "build": "cargo build --release --bin quipu",
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
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(classes, sort_keys=True))
    return 1 if any(item["status"] in {"failed", "error"} for item in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
