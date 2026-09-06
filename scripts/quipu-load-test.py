#!/usr/bin/env python3
"""Deterministic mixed HTTP load test and performance ratchet for Quipu."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import statistics
import time
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path


@dataclass
class Sample:
    operation: str
    elapsed_ms: float
    status: str
    started: float
    ended: float


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    rank = max(0, math.ceil(pct * len(ordered)) - 1)
    return ordered[rank]


def post(base_url: str, path: str, payload: dict, timeout: float, operation: str) -> Sample:
    started = time.monotonic()
    status = "ok"
    request = urllib.request.Request(
        base_url + path,
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json", "X-Quipu-Client": "load-test"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            response.read()
            if response.status != 200:
                status = f"http_{response.status}"
    except urllib.error.HTTPError as error:
        error.read()
        status = f"http_{error.code}"
    except TimeoutError:
        status = "timeout"
    except urllib.error.URLError as error:
        status = "timeout" if isinstance(error.reason, TimeoutError) else "transport"
    ended = time.monotonic()
    return Sample(operation, (ended - started) * 1000.0, status, started, ended)


def get_text(base_url: str, path: str, timeout: float) -> str:
    request = urllib.request.Request(
        base_url + path, headers={"X-Quipu-Client": "load-test"}
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read().decode()


def post_json(base_url: str, path: str, payload: dict, timeout: float) -> dict:
    request = urllib.request.Request(
        base_url + path,
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json", "X-Quipu-Client": "load-test"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read())


def seed(base_url: str, count: int, timeout: float) -> None:
    ontology = post(
        base_url,
        "/ontology",
        {
            "action": "load",
            "name": "load-test-ontology",
            "turtle": """@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://aegis.gastown.local/ontology/> .
ex:LoadFixture rdfs:subClassOf ex:Thing .
""",
        },
        timeout,
        "seed_ontology",
    )
    if ontology.status != "ok":
        raise RuntimeError(f"ontology seed failed: {ontology.status}")
    nodes = [
        {
            "name": f"load-{hashlib.sha256(str(index).encode()).hexdigest()[:20]}",
            "type": "LoadFixture",
            "description": f"deterministic load fixture {index}",
        }
        for index in range(count)
    ]
    sample = post(
        base_url,
        "/episode",
        {
            "name": "load-test-fixture",
            "source": "scripts/quipu-load-test.py",
            "nodes": nodes,
            "edges": [{
                "source": "LoadFixture",
                "target": "Thing",
                "relation": "rdfs:subClassOf",
            }],
        },
        timeout,
        "seed",
    )
    if sample.status != "ok":
        raise RuntimeError(f"fixture seed failed: {sample.status}")
    inferred = post_json(
        base_url,
        "/query",
        {"query": "SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a <http://aegis.gastown.local/ontology/Thing> }"},
        timeout,
    )
    actual = inferred.get("rows", [{}])[0].get("n")
    if actual != count or inferred.get("inference", {}).get("applied") is not True:
        raise RuntimeError(
            f"formal-default check failed: inferred Thing count={actual}, expected={count}, response={inferred}"
        )


def request_for(sequence: int) -> tuple[str, str, dict]:
    kind = sequence % 6
    if kind == 5:
        # ASK over an unbounded pattern (aegis-yzn4vp). This is the shape the
        # obvious liveness probe uses, and before the short-circuit it was the
        # MOST expensive question in the mix: 4.36 s on the 5.7 GB deployed
        # store against 4.2 ms for the equivalent SELECT ... LIMIT 1, because
        # the ASK arm materialised every solution to answer a yes/no question.
        #
        # It is measured as its own row rather than folded into query_full_scan
        # because they are no longer the same cost class and a regression here
        # is a health-probe outage, not a slow report.
        return "query_ask_unbounded", "/query", {"query": "ASK { ?s ?p ?o }"}
    if kind == 0:
        return "query_bounded", "/query", {
            "query": "SELECT ?p ?o WHERE { <http://aegis.gastown.local/ontology/load-5feceb66ffc86f38d952> ?p ?o } LIMIT 20"
        }
    if kind == 1:
        return "query_full_scan", "/query", {
            "query": "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"
        }
    if kind == 2:
        return "query_inferred_type", "/query", {
            "query": "SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a <http://aegis.gastown.local/ontology/Thing> }"
        }
    if kind == 3:
        return "search", "/search", {"embedding": [0.0] * 384, "limit": 5}
    return "episode", "/episode", {
        "name": f"load-write-{sequence}",
        "source": "scripts/quipu-load-test.py",
        "nodes": [{
            "name": f"write-{hashlib.sha256(f'write-{sequence}'.encode()).hexdigest()[:20]}",
            "type": "LoadFixture",
            "description": f"mixed-load write {sequence}",
        }],
    }


def summarize(samples: list[Sample], elapsed: float, peak_rss: int) -> dict:
    operations: dict[str, dict] = {}
    for operation in sorted({sample.operation for sample in samples}):
        selected = [sample for sample in samples if sample.operation == operation]
        latencies = [sample.elapsed_ms for sample in selected]
        statuses = Counter(sample.status for sample in selected)
        operations[operation] = {
            "requests": len(selected),
            "p50_ms": round(percentile(latencies, 0.50), 3),
            "p95_ms": round(percentile(latencies, 0.95), 3),
            "p99_ms": round(percentile(latencies, 0.99), 3),
            "statuses": dict(sorted(statuses.items())),
        }
    errors = Counter(sample.status for sample in samples if sample.status != "ok")
    return {
        "requests": len(samples),
        "elapsed_seconds": round(elapsed, 3),
        "throughput_rps": round(len(samples) / elapsed, 3),
        "peak_rss_bytes": peak_rss,
        "errors": dict(sorted(errors.items())),
        "operations": operations,
    }


def parse_peak_rss(metrics: str) -> int:
    for line in metrics.splitlines():
        if line.startswith("quipu_process_peak_rss_bytes "):
            return int(float(line.split()[1]))
    raise RuntimeError("/metrics omitted quipu_process_peak_rss_bytes")


def evaluate(report: dict, baseline: dict) -> list[str]:
    failures = []
    limits = baseline["limits"]
    total_errors = sum(report["errors"].values())
    error_rate = total_errors / max(1, report["requests"])
    if error_rate > limits["max_error_rate"]:
        failures.append(f"error rate {error_rate:.3f} > {limits['max_error_rate']:.3f}")
    if report["throughput_rps"] < limits["min_throughput_rps"]:
        failures.append(
            f"throughput {report['throughput_rps']:.3f} < {limits['min_throughput_rps']:.3f} rps"
        )
    if report["peak_rss_bytes"] > limits["max_peak_rss_bytes"]:
        failures.append(
            f"peak RSS {report['peak_rss_bytes']} > {limits['max_peak_rss_bytes']} bytes"
        )
    for operation, max_p99 in limits["max_p99_ms"].items():
        actual = report["operations"].get(operation, {}).get("p99_ms", float("inf"))
        if actual > max_p99:
            failures.append(f"{operation} p99 {actual:.3f} > {max_p99:.3f} ms")
    if report["read_progress_during_writes"] < limits["min_read_progress_during_writes"]:
        failures.append("no successful read overlapped a write; WAL read-pool progress unproven")
    return failures


def run(args: argparse.Namespace) -> dict:
    get_text(args.url, "/health", args.timeout)
    seed(args.url, args.seed_nodes, args.timeout)
    all_samples: list[Sample] = []
    levels = []
    sequence = 0
    overall_start = time.monotonic()
    for concurrency in args.concurrency:
        jobs = []
        for _ in range(args.requests_per_level):
            operation, path, payload = request_for(sequence)
            jobs.append((operation, path, payload))
            sequence += 1
        started = time.monotonic()
        with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
            futures = [pool.submit(post, args.url, path, payload, args.timeout, operation) for operation, path, payload in jobs]
            samples = [future.result() for future in futures]
        level_elapsed = time.monotonic() - started
        all_samples.extend(samples)
        levels.append({"concurrency": concurrency, **summarize(samples, level_elapsed, 0)})
    elapsed = time.monotonic() - overall_start
    peak_rss = parse_peak_rss(get_text(args.url, "/metrics", args.timeout))
    report = summarize(all_samples, elapsed, peak_rss)
    writes = [sample for sample in all_samples if sample.operation == "episode"]
    reads = [sample for sample in all_samples if sample.operation != "episode" and sample.status == "ok"]
    report["read_progress_during_writes"] = sum(
        1 for read in reads if any(read.started < write.ended and write.started < read.ended for write in writes)
    )
    report["concurrency_levels"] = levels
    report["architecture"] = {
        "reads": "WAL read pool",
        "writes": "single fair writer",
        "http_408": "slow read/query budget exhaustion; bounded read must still progress",
    }
    return report


def self_test() -> None:
    assert percentile([4, 1, 3, 2], 0.50) == 2
    assert percentile([4, 1, 3, 2], 0.99) == 4
    metrics = "# x\nquipu_process_peak_rss_bytes 12345\n"
    assert parse_peak_rss(metrics) == 12345
    base = {
        "limits": {
            "max_error_rate": 0.0,
            "min_throughput_rps": 1.0,
            "max_peak_rss_bytes": 100,
            "max_p99_ms": {"query_bounded": 10.0},
            "min_read_progress_during_writes": 1,
        }
    }
    report = {
        "requests": 2, "errors": {}, "throughput_rps": 2.0, "peak_rss_bytes": 99,
        "operations": {"query_bounded": {"p99_ms": 9.0}}, "read_progress_during_writes": 1,
    }
    assert evaluate(report, base) == []
    report["errors"] = {"http_408": 1}
    assert any("error rate" in failure for failure in evaluate(report, base))
    print("quipu-load-test self-test: PASS")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:3030")
    parser.add_argument("--concurrency", type=int, nargs="+", default=[1, 4, 8])
    parser.add_argument("--requests-per-level", type=int, default=24)
    parser.add_argument("--seed-nodes", type=int, default=200)
    parser.add_argument("--timeout", type=float, default=10.0)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    report = run(args)
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered)
    print(rendered, end="")
    if args.baseline:
        failures = evaluate(report, json.loads(args.baseline.read_text()))
        for failure in failures:
            print(f"RATCHET: {failure}")
        return 1 if failures else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
