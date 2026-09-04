#!/usr/bin/env python3
"""Score the approved W3C SPARQL 1.1 SERVICE tests against Quipu."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("sparql11_evaluation", HERE / "sparql11_evaluation.py")
EVAL = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = EVAL
SPEC.loader.exec_module(EVAL)
PINNED_SUITE_REVISION = EVAL.PINNED_SUITE_REVISION
SERVICE_DATA = re.compile(r"qt:serviceData\s*\[\s*qt:endpoint\s*<([^>]+)>\s*;\s*qt:data\s*<([^>]+)>\s*\]", re.S)


def discover(manifest: Path) -> list[tuple[object, tuple[tuple[str, Path], ...]]]:
    cases = {case.identifier: case for case in EVAL.parse_manifest("federated-query", manifest)}
    found = []
    for statement in EVAL.turtle_statements(manifest.read_text()):
        subject = statement.lstrip().split(None, 1)[0]
        if subject in cases:
            data = tuple((endpoint, manifest.parent / path) for endpoint, path in SERVICE_DATA.findall(statement))
            found.append((cases[subject], data))
    return found


def start_server(server: Path, cwd: Path, database: Path, url: str) -> subprocess.Popen:
    process = subprocess.Popen([str(server), "--db", str(database), "--bind", url.removeprefix("http://")], cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    for _ in range(150):
        try:
            with urllib.request.urlopen(f"{url}/health", timeout=0.2):
                return process
        except (urllib.error.URLError, TimeoutError):
            if process.poll() is not None:
                raise ValueError(f"quipu-server exited before health: {process.stderr.read().strip()}")
            time.sleep(0.02)
    process.terminate()
    raise ValueError("quipu-server did not become healthy")


def run_case(case: object, service_data: tuple[tuple[str, Path], ...], quipu: Path, server: Path) -> dict:
    base = {"class": "federated-query", "id": case.identifier, "name": case.name, "manifest": "service/manifest.ttl", "query": f"service/{case.query.name}", "result": f"service/{case.result.name}"}
    if re.search(r"\bSERVICE\s+(?:SILENT\s+)?\?", case.query.read_text(), re.I):
        return {**base, "status": "unsupported", "reason": "variable SERVICE endpoints are deliberately refused; endpoints must be operator-configured", "commitment": "deliberate-policy-deviation"}
    with tempfile.TemporaryDirectory(prefix="quipu-w3c-service-") as raw:
        temporary = Path(raw)
        endpoint_map = {endpoint: f"http://127.0.0.1:{EVAL.reserve_port()}/query" for endpoint, _ in service_data}
        config_dir = temporary / ".bobbin"
        config_dir.mkdir()
        lines: list[str] = []
        for index, (endpoint, _) in enumerate(service_data):
            lines += ["[[quipu.federation.remotes]]", f'name = "w3c-{index}"', f'url = "{endpoint_map[endpoint].removesuffix("/query")}"', ""]
        (config_dir / "config.toml").write_text("\n".join(lines))
        processes = []
        try:
            for index, (endpoint, fixture) in enumerate(service_data):
                database = temporary / f"remote-{index}.db"
                loaded = subprocess.run([str(quipu), "knot", str(fixture), "--db", str(database)], text=True, capture_output=True)
                if loaded.returncode:
                    raise ValueError(loaded.stderr.strip())
                processes.append(start_server(server, temporary, database, endpoint_map[endpoint].removesuffix("/query")))
            local_db = temporary / "local.db"
            remote_fixtures = {fixture for _, fixture in service_data}
            # The general manifest parser sees nested qt:data values too;
            # serviceData belongs only in its endpoint store, never locally.
            for fixture in case.data:
                if fixture in remote_fixtures:
                    continue
                text = fixture.read_text()
                for declared, local in endpoint_map.items():
                    text = text.replace(declared, local)
                mapped = temporary / fixture.name
                mapped.write_text(text)
                loaded = subprocess.run([str(quipu), "knot", str(mapped), "--db", str(local_db)], text=True, capture_output=True)
                if loaded.returncode:
                    raise ValueError(loaded.stderr.strip())
            coordinator_url = f"http://127.0.0.1:{EVAL.reserve_port()}"
            processes.append(start_server(server, temporary, local_db, coordinator_url))
            query = case.query.read_text()
            for declared, local in endpoint_map.items():
                query = query.replace(declared, local)
            request = urllib.request.Request(f"{coordinator_url}/query", data=query.encode(), method="POST", headers={"Content-Type": "application/sparql-query", "Accept": "application/sparql-results+json"})
            try:
                response = urllib.request.urlopen(request, timeout=10)
            except urllib.error.HTTPError as error:
                raise ValueError(f"HTTP {error.code}: {error.read().decode(errors='replace')}") from error
            with response:
                observed_path = temporary / "observed.srj"
                observed_path.write_bytes(response.read())
            actual_vars, actual_rows = EVAL.expected_json(observed_path)
            expected_vars, expected_rows = EVAL.expected_result(case.result)
            # Quipu exposes provider/trust/freshness as extension metadata. They
            # are deliberately outside the W3C query's projected variables and
            # therefore outside standards-result comparison.
            keep = [index for index, name in enumerate(actual_vars) if name not in {"_provider", "_trust", "_freshness"}]
            actual_vars = [actual_vars[index] for index in keep]
            actual_rows = [tuple(row[index] for index in keep) for row in actual_rows]
            aligned = EVAL.reorder_rows(actual_vars, actual_rows, expected_vars)
            passed = aligned is not None and Counter(aligned) == Counter(expected_rows)
            diagnostic = "" if passed else f"actual vars/rows {actual_vars!r} {actual_rows!r}; expected {expected_vars!r} {expected_rows!r}"
            return {**base, "status": "passed" if passed else "failed", "diagnostic": diagnostic}
        except (ValueError, urllib.error.URLError, KeyError, json.JSONDecodeError) as error:
            return {**base, "status": "error", "diagnostic": str(error)}
        finally:
            for process in reversed(processes):
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path)
    parser.add_argument("--quipu", default="quipu", type=Path)
    parser.add_argument("--server", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--allow-unpinned-suite", action="store_true")
    args = parser.parse_args()
    args.quipu = EVAL.executable_path(args.quipu)
    args.server = EVAL.executable_path(args.server or args.quipu.with_name("quipu-server"))
    revision = EVAL.git_output(args.suite, "rev-parse", "HEAD")
    dirty = EVAL.git_output(args.suite, "status", "--porcelain")
    if not args.allow_unpinned_suite and (revision != PINNED_SUITE_REVISION or dirty):
        parser.error(f"suite must be clean at {PINNED_SUITE_REVISION}; got {revision}")
    cases = discover(args.suite / "service" / "manifest.ttl")
    if len(cases) != 7:
        parser.error(f"expected exactly 7 approved SERVICE cases, found {len(cases)}")
    results = [run_case(case, data, args.quipu, args.server) for case, data in cases]
    counts = Counter(row["status"] for row in results)
    report = {"benchmark": "W3C RDF Tests SPARQL 1.1 federated query", "suite_revision": revision, "quipu_revision": EVAL.git_output(Path(__file__).resolve().parents[2], "rev-parse", "HEAD"), "quipu_version": subprocess.run([str(args.quipu), "--version"], check=True, text=True, capture_output=True).stdout.strip(), "scope": "all seven Working Group-approved BasicFederatedQuery SERVICE cases", "policy": "SERVICE IRIs must match operator-configured remotes; variable endpoints are a deliberate policy deviation", "classes": {"federated-query": {"cases": len(results), **dict(sorted(counts.items()))}}, "results": results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["classes"], sort_keys=True))
    return 0 if counts["failed"] == 0 and counts["error"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
