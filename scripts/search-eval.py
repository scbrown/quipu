#!/usr/bin/env python3
"""Evaluate judged retrieval against an isolated loopback Quipu server.

No external dependencies. Operational corpora/judgments stay outside this repo.
"""
from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import math
from pathlib import Path
import statistics
import time
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("evaluation must not follow redirects")


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical(entity, prefixes):
    for prefix, base in prefixes.items():
        if entity.startswith(prefix + ":"):
            return base + entity[len(prefix) + 1:]
    return entity


def validate_suite(suite):
    if suite.get("schema_version") != 1:
        raise ValueError("expected schema_version=1")
    queries = suite["queries"]
    if not queries or len({q["id"] for q in queries}) != len(queries):
        raise ValueError("queries must be nonempty with unique IDs")
    prefixes = suite.get("prefixes", {})
    for query in queries:
        for key in ("id", "query", "class", "source", "judgment_note"):
            if not isinstance(query[key], str) or not query[key].strip():
                raise ValueError(f"missing {key}")
        judgments = query["relevance"]
        if not judgments or any(type(v) is not int or not 0 <= v <= 3 for v in judgments.values()):
            raise ValueError("relevance must contain integer grades 0..3")
        if not any(judgments.values()):
            raise ValueError("each query needs a positive judgment")
        keys = [canonical(k, prefixes) for k in judgments]
        if len(keys) != len(set(keys)):
            raise ValueError("duplicate canonical judgment")
        if set(query.get("params", {})) & {"query", "embedding", "limit"}:
            raise ValueError("params cannot override query, embedding or limit")
    return queries


def metrics(results, relevance, prefixes=None):
    prefixes = prefixes or {}
    judged = {canonical(k, prefixes): v for k, v in relevance.items()}
    entities = [canonical(r["entity"], prefixes) for r in results]
    if len(set(entities)) != len(entities):
        raise ValueError("duplicate entity in search response")
    if any(not isinstance(r["score"], (int, float)) or not math.isfinite(r["score"]) for r in results):
        raise ValueError("non-finite or missing search score")
    grades = [judged.get(e, 0) for e in entities]
    dcg = lambda xs: sum((2 ** g - 1) / math.log2(i + 2) for i, g in enumerate(xs[:10]))
    ideal = dcg(sorted(judged.values(), reverse=True))
    return {
        "ndcg@10": dcg(grades) / ideal,
        "recall@20": sum(g > 0 for g in grades[:20]) / sum(g > 0 for g in judged.values()),
        "mrr@20": next((1 / (i + 1) for i, g in enumerate(grades[:20]) if g > 0), 0),
        "judged_fraction@20": sum(e in judged for e in entities[:20]) / max(1, len(entities[:20])),
    }


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def summarize(rows):
    if not rows:
        raise ValueError("no evaluated queries")
    latencies = [ms for r in rows for ms in r["latency_ms"]]
    return {
        "queries": len(rows),
        **{k: statistics.mean(r["metrics"][k] for r in rows) for k in rows[0]["metrics"]},
        "latency_p50_ms": percentile(latencies, .5),
        "latency_p95_ms": percentile(latencies, .95),
    }


def local_endpoint(endpoint):
    parsed = urlsplit(endpoint)
    if (parsed.scheme != "http" or parsed.username or parsed.password
            or parsed.path not in ("", "/") or parsed.query or parsed.fragment
            or not parsed.hostname or not ipaddress.ip_address(parsed.hostname).is_loopback):
        raise ValueError("endpoint must be a literal loopback HTTP origin")
    return endpoint.rstrip("/")


def request(endpoint, path, payload=None):
    body = None if payload is None else json.dumps(payload).encode()
    req = Request(endpoint + path, data=body, headers={
        "Content-Type": "application/json", "X-Quipu-Client": "agent-adhoc",
    })
    start = time.perf_counter()
    with build_opener(NoRedirect).open(req, timeout=60) as response:
        value = json.load(response)
    elapsed = (time.perf_counter() - start) * 1000
    if not isinstance(value, dict) or "error" in value:
        raise ValueError(f"invalid response from {path}")
    return value, elapsed


def evaluate(suite, endpoint, repeats):
    queries = validate_suite(suite)
    prefixes = suite.get("prefixes", {})
    rows = []
    for query in queries:
        payload = {**query.get("params", {}), "query": query["query"], "limit": 20}
        # One unmeasured warmup per query; all repetitions retained.
        request(endpoint, "/search", payload)
        samples = [request(endpoint, "/search", payload) for _ in range(repeats)]
        results = samples[0][0]["results"]
        signature = lambda hits: [(canonical(r["entity"], prefixes), r["score"]) for r in hits]
        if any(signature(s[0]["results"]) != signature(results) for s in samples):
            raise ValueError(f"unstable rankings for {query['id']}; freeze corpus/config")
        rows.append({"id": query["id"], "class": query["class"], "request": payload,
                     "results": results, "latency_ms": [s[1] for s in samples],
                     "metrics": metrics(results, query["relevance"], prefixes)})
    return rows


def compare(reference, candidate):
    """Exact semantic regression gate; query identity, corpus and judgments must match."""
    for key in ("suite_sha256", "corpus_sha256"):
        if reference[key] != candidate[key]:
            raise ValueError(f"cannot compare different {key}")
    left, right = reference["rows"], candidate["rows"]
    if len(left) != len(right):
        raise ValueError("query count changed")
    for old, new in zip(left, right):
        if old["id"] != new["id"] or old["request"] != new["request"]:
            raise ValueError("query identity or request changed")
        sig = lambda row: [(r["entity"], r["score"]) for r in row["results"]]
        if sig(old) != sig(new):
            raise ValueError(f"semantic ranking changed: {old['id']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", type=Path, required=True)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--corpus-sha256", required=True)
    parser.add_argument("--snapshot-at", required=True)
    parser.add_argument("--expected-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--compare", type=Path)
    args = parser.parse_args()
    if args.repeats < 1 or len(args.corpus_sha256) != 64:
        parser.error("positive repeats and SHA256 corpus digest required")
    endpoint = local_endpoint(args.endpoint)
    suite = json.loads(args.suite.read_text())
    version, _ = request(endpoint, "/version")
    if version.get("git_sha") != args.expected_sha or version.get("git_dirty") is not False:
        raise ValueError("server is not the expected clean build")
    rows = evaluate(suite, endpoint, args.repeats)
    after, _ = request(endpoint, "/version")
    if after != version:
        raise ValueError("server version changed during evaluation")
    report = {"schema_version": 1, "suite_sha256": digest(args.suite),
              "corpus_sha256": args.corpus_sha256, "snapshot_at": args.snapshot_at,
              "version": version, "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "warmup_per_query": 1, "repeats": args.repeats, "rows": rows,
              "overall": summarize(rows),
              "by_class": {c: summarize([r for r in rows if r["class"] == c])
                           for c in sorted({r["class"] for r in rows})}}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    if args.compare:
        compare(json.loads(args.compare.read_text()), report)
    print(json.dumps(report["overall"], indent=2))


if __name__ == "__main__":
    main()
