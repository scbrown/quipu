#!/usr/bin/env python3
"""Score Quipu against the pinned W3C SHACL Core manifest inventory."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import time
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

PINNED_SUITE_REVISION = "9c863967bceaef1a87c24e4dd761eda763823120"
RUNNER_VERSION = 1
INCLUDES = re.compile(r"mf:include\s+<([^>]+)>")
ENTRIES = re.compile(r"mf:entries\s*\((.*?)\)", re.S)
PREFIX = re.compile(r"@prefix\s+([\w-]*):\s*<([^>]+)>\s*\.")
APPROVED = re.compile(r"mf:status\s+sht:approved")
EXPECTED_COUNTS = {
    "core-complex-misc": 7,
    "core-node": 32,
    "core-property": 38,
    "core-path": 13,
    "core-targets": 7,
    "core-validation-reports": 1,
    # The pinned manifest includes 22. nodeValidator-001.ttl exists in the
    # checkout but is deliberately not reachable from component/manifest.ttl;
    # manifest discovery must not inflate the denominator by enumerating files.
    "shacl-sparql": 22,
}


class HarnessError(RuntimeError):
    pass


@dataclass(frozen=True)
class Case:
    identifier: str
    fixture: Path
    category: str
    action: str
    data: Path
    shapes: Path
    expected_conforms: bool | None
    expected_results: tuple[tuple[tuple[str, str], ...], ...]


def git_output(cwd: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(cwd), *args], check=True, text=True, capture_output=True).stdout.strip()


def executable_path(value: Path) -> Path:
    if value.parent == Path(".") and not value.exists():
        located = shutil.which(str(value))
        if located:
            return Path(located).resolve()
        raise HarnessError(f"executable not found: {value}")
    return value.resolve()


def balanced(text: str, start: int, opening: str = "[", closing: str = "]") -> str:
    depth = 0
    quote = None
    escaped = False
    comment = False
    for index in range(start, len(text)):
        char = text[index]
        if comment:
            if char in "\r\n":
                comment = False
            continue
        if quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            continue
        if char == "#":
            comment = True
        elif char in "\"'":
            quote = char
        elif char == opening:
            depth += 1
        elif char == closing:
            depth -= 1
            if depth == 0:
                return text[start:index + 1]
    raise HarnessError(f"unterminated {opening}{closing} region")


def walk_manifests(root: Path) -> list[Path]:
    found: list[Path] = []
    visiting: set[Path] = set()
    visited: set[Path] = set()

    def visit(path: Path) -> None:
        path = path.resolve()
        if path in visiting:
            raise HarnessError(f"manifest include cycle at {path}")
        if path in visited:
            return
        if not path.is_file():
            raise HarnessError(f"missing manifest fixture: {path}")
        visiting.add(path)
        includes = INCLUDES.findall(path.read_text())
        if includes:
            for relative in includes:
                visit(path.parent / relative)
        else:
            found.append(path)
        visiting.remove(path)
        visited.add(path)

    visit(root / "manifest.ttl")
    return found


def category_of(relative: Path) -> str:
    parts = relative.parts
    if parts[0] == "sparql":
        return "shacl-sparql"
    if parts[:2] in (("core", "complex"), ("core", "misc")):
        return "core-complex-misc"
    return f"core-{parts[1]}"


def referenced_graph(text: str, predicate: str, fixture: Path) -> Path:
    match = re.search(rf"{re.escape(predicate)}\s+<([^>]*)>", text)
    if not match or match.group(1) == "":
        return fixture
    return (fixture.parent / match.group(1)).resolve()


def expand_term(token: str, prefixes: dict[str, str]) -> str:
    token = token.strip().rstrip(";")
    if token.startswith("<") and token.endswith(">"):
        return token[1:-1]
    if token.startswith('"'):
        match = re.match(r'"((?:[^"\\]|\\.)*)"', token, re.S)
        return bytes(match.group(1), "utf-8").decode("unicode_escape") if match else token
    if ":" in token:
        prefix, local = token.split(":", 1)
        if prefix in prefixes:
            return prefixes[prefix] + local
    return token


def expected_results(statement: str, document: str) -> tuple[tuple[tuple[str, str], ...], ...]:
    prefixes = dict(PREFIX.findall(document))
    fields = {
        "sh:focusNode": "focus_node",
        "sh:sourceConstraintComponent": "component",
        "sh:resultPath": "path",
        "sh:value": "value",
        "sh:sourceShape": "source_shape",
        "sh:resultSeverity": "severity",
        "sh:resultMessage": "message",
    }
    results = []
    cursor = 0
    while (match := re.search(r"sh:result\s*\[", statement[cursor:])):
        start = cursor + match.end() - 1
        block = balanced(statement, start)
        cursor = start + len(block)
        values = {}
        for predicate, name in fields.items():
            found = re.search(rf"{re.escape(predicate)}\s+([^;\]\n]+)", block)
            if found:
                values[name] = expand_term(found.group(1), prefixes)
        if values:
            results.append(tuple(sorted(values.items())))
    return tuple(results)


def discover_cases(root: Path) -> list[Case]:
    cases: list[Case] = []
    seen: set[str] = set()
    for fixture in walk_manifests(root):
        text = fixture.read_text()
        entries = ENTRIES.search(text)
        if not entries:
            raise HarnessError(f"leaf fixture has no mf:entries: {fixture}")
        identifiers = re.findall(r"<([^>]+)>|(?<![\w-])(:[\w-]+)", entries.group(1))
        for iri, qname in identifiers:
            local = iri or qname
            token = f"<{local}>" if iri else local
            markers = list(re.finditer(rf"(?m)^\s*{re.escape(token)}\s+", text))
            if not markers:
                raise HarnessError(f"entry {token} has no definition in {fixture}")
            marker = markers[-1]
            end = text.find("\n.", marker.start())
            statement = text[marker.start(): end + 2 if end >= 0 else len(text)]
            if not APPROVED.search(statement):
                continue
            action = "Validate" if "rdf:type sht:Validate" in statement else "unknown"
            key = f"{fixture.relative_to(root).with_suffix('')}#{Path(local).name.lstrip(':')}"
            if key in seen:
                raise HarnessError(f"duplicate SHACL case key: {key}")
            seen.add(key)
            conforms = re.search(r'sh:conforms\s+"(true|false)"\^\^xsd:boolean', statement)
            cases.append(Case(
                key, fixture, category_of(fixture.relative_to(root)), action,
                referenced_graph(statement, "sht:dataGraph", fixture),
                referenced_graph(statement, "sht:shapesGraph", fixture),
                conforms.group(1) == "true" if conforms else None,
                expected_results(statement, text),
            ))
    counts = Counter(case.category for case in cases)
    if dict(counts) != EXPECTED_COUNTS or len(cases) != 120:
        raise HarnessError(f"SHACL inventory mismatch: expected={EXPECTED_COUNTS}, observed={dict(counts)}")
    return cases


def run_case(case: Case, adapter: Path, root: Path) -> dict:
    base = {
        "id": case.identifier,
        "fixture": str(case.fixture.relative_to(root)),
        "manifest": str(case.fixture.relative_to(root)),
        "class": case.category,
        "category": case.category,
        "w3c_action": case.action,
        "expected_outcome": "failure" if case.expected_conforms is None else "conforms" if case.expected_conforms else "violates",
        "quipu_code_path": "validate_shapes -> cached_validator -> Validator::from_turtle -> Validator::validate -> rudof Native",
    }
    if case.category == "shacl-sparql":
        return {**base, "status": "unsupported", "reason": "SHACL-SPARQL constraints are not implemented", "duration_ms": 0}
    started = time.monotonic()
    try:
        observed = subprocess.run(
            [str(adapter), "--shapes", str(case.shapes), "--data", str(case.data)],
            text=True, capture_output=True, timeout=20,
        )
    except subprocess.TimeoutExpired:
        return {**base, "status": "error", "reason": "case timed out", "duration_ms": 20_000}
    duration = round((time.monotonic() - started) * 1000, 3)
    if observed.returncode:
        status = "passed" if case.expected_conforms is None else "error"
        diagnostic = re.sub(r"thread 'main' \(\d+\)", "thread 'main'", observed.stderr.strip())
        diagnostic = re.sub(r"/[^\s:]+/\.cargo/registry/src/[^\s:]+/", "$CARGO_REGISTRY/", diagnostic)
        return {**base, "status": status, "reason": diagnostic, "duration_ms": duration}
    try:
        report = json.loads(observed.stdout)
    except json.JSONDecodeError as error:
        return {**base, "status": "error", "reason": f"invalid adapter JSON: {error}", "duration_ms": duration}
    passed = case.expected_conforms is not None and report.get("conforms") is case.expected_conforms
    actual_rows = []
    for issue in report.get("results", []):
        component = issue.get("component")
        if component and component.startswith("http://www.w3.org/ns/shacl#") and not component.endswith("ConstraintComponent"):
            local = component.rsplit("#", 1)[-1]
            component = "http://www.w3.org/ns/shacl#" + local[:1].upper() + local[1:] + "ConstraintComponent"
        normalized = {**issue, "component": component}
        severity = normalized.get("severity")
        if severity and not severity.startswith("http"):
            normalized["severity"] = "http://www.w3.org/ns/shacl#" + severity
        actual_rows.append(normalized)
    if case.expected_results:
        remaining = actual_rows.copy()
        for expected_row in case.expected_results:
            wanted = dict(expected_row)
            matched = next((index for index, actual in enumerate(remaining)
                            if all(str(actual.get(name) or "") == value for name, value in wanted.items())), None)
            if matched is None:
                passed = False
                break
            remaining.pop(matched)
        passed = passed and not remaining
    return {
        **base,
        "status": "passed" if passed else "failed",
        "reason": "" if passed else "validation report differs from expected normative terms",
        "duration_ms": duration,
        "observed": {"conforms": report.get("conforms"), "violations": report.get("violations"), "warnings": report.get("warnings")},
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path)
    parser.add_argument("--suite-revision", default=PINNED_SUITE_REVISION)
    parser.add_argument("--quipu", required=True, type=Path, help="quipu-shacl-conformance binary")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--allow-unpinned-suite", action="store_true")
    args = parser.parse_args(argv)
    try:
        adapter = executable_path(args.quipu)
        checkout = args.suite.resolve()
        while checkout != checkout.parent and not (checkout / ".git").exists():
            checkout = checkout.parent
        revision = git_output(checkout, "rev-parse", "HEAD")
        dirty = git_output(checkout, "status", "--porcelain")
        if not args.allow_unpinned_suite and (revision != args.suite_revision or dirty):
            raise HarnessError(f"suite must be clean at {args.suite_revision}; got {revision}, dirty={bool(dirty)}")
        cases = discover_cases(args.suite)
    except (HarnessError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    rows = [run_case(case, adapter, args.suite) for case in cases]
    inventory = sorted((row["id"], row["fixture"], row["category"]) for row in rows)
    counts = {category: dict(Counter(row["status"] for row in rows if row["category"] == category)) for category in EXPECTED_COUNTS}
    report = {
        "benchmark": "W3C SHACL Core conformance",
        "suite_revision": revision,
        "quipu_revision": git_output(Path(__file__).resolve().parents[2], "rev-parse", "HEAD"),
        "runner_version": RUNNER_VERSION,
        "inventory_digest": hashlib.sha256(json.dumps(inventory, separators=(",", ":")).encode()).hexdigest(),
        "classes": counts,
        "results": rows,
        "reproduce": {"runner": "python3 benchmark/public/shacl_core.py", "suite": "/tmp/data-shapes/data-shapes-test-suite/tests"},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(counts, sort_keys=True))
    if args.baseline:
        compared = subprocess.run(["python3", str(Path(__file__).with_name("check_regression.py")), "--baseline", str(args.baseline), "--candidate", str(args.output)])
        return compared.returncode
    return 1 if any(row["status"] in {"failed", "error"} for row in rows) else 0


if __name__ == "__main__":
    raise SystemExit(main())
