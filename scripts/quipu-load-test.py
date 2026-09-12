#!/usr/bin/env python3
"""Deterministic mixed HTTP load test and performance ratchet for Quipu."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
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


# Write endpoints require a bearer on the deployed fleet while reads stay open
# (aegis-z10). Without one this harness cannot seed its own fixture against a
# production server, so the acceptance bar "run the checked-in harness on the
# deployed corpus" was unreachable with the checked-in harness. Never printed.
_AUTH_TOKEN: str | None = None

# Last error body per operation, so a seed failure can say WHY.
_LAST_ERROR_BODY: dict[str, str] = {}


def _headers(json_body: bool) -> dict[str, str]:
    headers = {"X-Quipu-Client": "load-test"}
    if json_body:
        headers["Content-Type"] = "application/json"
    if _AUTH_TOKEN:
        headers["Authorization"] = f"Bearer {_AUTH_TOKEN}"
    return headers


def post(base_url: str, path: str, payload: dict, timeout: float, operation: str) -> Sample:
    started = time.monotonic()
    status = "ok"
    request = urllib.request.Request(
        base_url + path,
        data=json.dumps(payload).encode(),
        headers=_headers(json_body=True),
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            response.read()
            if response.status != 200:
                status = f"http_{response.status}"
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors="replace")
        # Keep the server's reason. A bare "http_400" on the seed sent a reader
        # hunting their own payload when the refusal was a pre-existing OWL
        # violation elsewhere in the store (aegis-svtdyn, 2026-09-12).
        _LAST_ERROR_BODY[operation] = body[:2000]
        status = f"http_{error.code}"
    except TimeoutError:
        status = "timeout"
    except urllib.error.URLError as error:
        status = "timeout" if isinstance(error.reason, TimeoutError) else "transport"
    ended = time.monotonic()
    return Sample(operation, (ended - started) * 1000.0, status, started, ended)


def get_text(base_url: str, path: str, timeout: float) -> str:
    request = urllib.request.Request(base_url + path, headers=_headers(json_body=False))
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read().decode()


def post_json(base_url: str, path: str, payload: dict, timeout: float) -> dict:
    request = urllib.request.Request(
        base_url + path,
        data=json.dumps(payload).encode(),
        headers=_headers(json_body=True),
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
        raise RuntimeError(
            f"ontology seed failed: {ontology.status}: "
            f"{_LAST_ERROR_BODY.get('seed_ontology', '<no body>')}"
        )
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
        raise RuntimeError(
            f"fixture seed failed: {sample.status}: "
            f"{_LAST_ERROR_BODY.get('seed', '<no body>')}"
        )
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


def evaluate(report: dict, baseline: dict) -> tuple[list[str], list[str]]:
    """Grade a report. Returns (failures, unmeasured).

    `unmeasured` exists because a two-state verdict is FORCED to render "this
    instrument could not see the bound" as "the bound was not exceeded" or as
    "the bound was exceeded", and both readings are wrong. The RSS bound is the
    case that needs it: `quipu_process_peak_rss_bytes` is a high-water mark
    since process start, so on a long-lived server the post-run reading is the
    max of what this run did and everything the process had already done. On
    the deployed fleet that reading was 2.42 GB against a 512 MiB bound before
    a single harness request was sent (aegis-svtdyn, 2026-09-12) — a number no
    change to the code under test could have moved.
    """
    failures = []
    unmeasured = []
    limits = baseline["limits"]
    total_errors = sum(report["errors"].values())
    error_rate = total_errors / max(1, report["requests"])
    if error_rate > limits["max_error_rate"]:
        failures.append(f"error rate {error_rate:.3f} > {limits['max_error_rate']:.3f}")
    if report["throughput_rps"] < limits["min_throughput_rps"]:
        failures.append(
            f"throughput {report['throughput_rps']:.3f} < {limits['min_throughput_rps']:.3f} rps"
        )
    max_rss = limits["max_peak_rss_bytes"]
    baseline_peak = report.get("baseline_peak_rss_bytes", 0)
    growth = report.get("peak_rss_growth_bytes", 0)
    # Growth IS attributable to this run on a warm process as well as a cold
    # one, so it is always graded. It is a LOWER bound, never a certificate:
    # zero growth means the run did not push past a mark the process had
    # already set, not that the run was cheap.
    if growth > max_rss:
        failures.append(f"peak RSS grew {growth} > {max_rss} bytes during this run")
    if baseline_peak > max_rss:
        unmeasured.append(
            f"peak RSS bound {max_rss} bytes: NOT MEASURED. The server had already "
            f"reached {baseline_peak} bytes before this run began, so the post-run "
            f"reading of {report['peak_rss_bytes']} is not attributable to this load. "
            f"Restart the server and re-run to measure this bound."
        )
    elif report["peak_rss_bytes"] > max_rss:
        failures.append(f"peak RSS {report['peak_rss_bytes']} > {max_rss} bytes")
    for operation, max_p99 in limits["max_p99_ms"].items():
        actual = report["operations"].get(operation, {}).get("p99_ms", float("inf"))
        if actual > max_p99:
            failures.append(f"{operation} p99 {actual:.3f} > {max_p99:.3f} ms")
    if report["read_progress_during_writes"] < limits["min_read_progress_during_writes"]:
        failures.append("no successful read overlapped a write; WAL read-pool progress unproven")
    return failures, unmeasured


def run(args: argparse.Namespace) -> dict:
    get_text(args.url, "/health", args.timeout)
    # Read the high-water mark BEFORE seeding. Everything after this point is
    # ours; everything before it belongs to whatever the process did earlier.
    baseline_peak_rss = parse_peak_rss(get_text(args.url, "/metrics", args.timeout))
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
    report["baseline_peak_rss_bytes"] = baseline_peak_rss
    report["peak_rss_growth_bytes"] = max(0, peak_rss - baseline_peak_rss)
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

    def clean(**overrides):
        report = {
            "requests": 2, "errors": {}, "throughput_rps": 2.0, "peak_rss_bytes": 99,
            "operations": {"query_bounded": {"p99_ms": 9.0}},
            "read_progress_during_writes": 1,
            "baseline_peak_rss_bytes": 10, "peak_rss_growth_bytes": 89,
        }
        report.update(overrides)
        return report

    assert evaluate(clean(), base) == ([], [])
    assert any("error rate" in f for f in evaluate(clean(errors={"http_408": 1}), base)[0])

    # RSS verdict, all four arms. A cold process grades the absolute peak; a
    # process already over the bound cannot grade it at all and must say so
    # rather than emit a failure nothing in the code under test could clear.
    cold_over = clean(peak_rss_bytes=101, baseline_peak_rss_bytes=10, peak_rss_growth_bytes=91)
    failures, unmeasured = evaluate(cold_over, base)
    assert any("peak RSS 101" in f for f in failures), failures
    assert unmeasured == [], unmeasured

    warm_over = clean(peak_rss_bytes=5000, baseline_peak_rss_bytes=5000, peak_rss_growth_bytes=0)
    failures, unmeasured = evaluate(warm_over, base)
    assert failures == [], failures
    assert any("NOT MEASURED" in u for u in unmeasured), unmeasured

    warm_grew = clean(peak_rss_bytes=5200, baseline_peak_rss_bytes=5000, peak_rss_growth_bytes=200)
    failures, unmeasured = evaluate(warm_grew, base)
    assert any("grew 200" in f for f in failures), failures
    assert any("NOT MEASURED" in u for u in unmeasured), unmeasured

    # The boundary itself: baseline exactly AT the bound is still measurable.
    at_bound = clean(peak_rss_bytes=100, baseline_peak_rss_bytes=100, peak_rss_growth_bytes=0)
    assert evaluate(at_bound, base) == ([], []), evaluate(at_bound, base)

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
    parser.add_argument(
        "--auth-token-file",
        type=Path,
        help="file holding the bearer for write endpoints; QUIPU_AUTH_TOKEN overrides it",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    global _AUTH_TOKEN
    _AUTH_TOKEN = os.environ.get("QUIPU_AUTH_TOKEN") or (
        args.auth_token_file.read_text().strip() if args.auth_token_file else None
    )
    report = run(args)
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered)
    print(rendered, end="")
    if args.baseline:
        failures, unmeasured = evaluate(report, json.loads(args.baseline.read_text()))
        for item in unmeasured:
            print(f"UNMEASURED: {item}")
        for failure in failures:
            print(f"RATCHET: {failure}")
        if failures:
            return 1
        # Exit 2, not 0: a bound the instrument could not see is not a bound
        # the code under test satisfied, and a green run that silently dropped
        # one of its criteria is the failure this distinction exists to stop.
        return 2 if unmeasured else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
