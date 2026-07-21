# Example: a homelab knowledge graph

A small, **fully synthetic** homelab graph that shows the kinds of questions a Quipu
knowledge graph answers. No real hosts, IPs, credentials, or people — safe to read and run.

Three hosts run six services, owned by three people, under a deploy policy that changed
over time. From that you can ask about **ownership**, **topology**, **dependency/impact**,
**time-aware policy supersession**, **inventory**, and **provenance**.

## Load it

```bash
quipu episode examples/homelab-graph/episode.json \
  --base-ns http://example.org/homelab/
```

`--base-ns` matters: without it, Quipu mints IRIs in its **own** default namespace, so the
facts you load from this example land under someone else's domain in *your* store. The
queries below assume `http://example.org/homelab/`.

Over HTTP the same thing:

```bash
curl -s http://localhost:PORT/episode -X POST -H "Content-Type: application/json" \
  --data @examples/homelab-graph/episode.json
```

This ingests into `group_id: example-homelab`. Then query with SPARQL via `POST /query`
(`{"query": "..."}`). All queries below use:

```sparql
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX ex:   <http://example.org/homelab/>
```

## What you can ask

### 1. Ownership — "who owns / who do I ask about X?"

```sparql
SELECT ?owner ?thing WHERE {
  ?o ?p ?t . ?o rdfs:label ?owner . ?t rdfs:label ?thing .
  FILTER(regex(str(?p), "owns"))
}
```

→ 6 rows: `evan` owns photo-app, media-hub, ledger-db; `dana` owns edge-proxy, archive-fs;
`fern` owns policy-gitops-deploys.

Note `fern` owns a **policy**, not a service — ownership here spans both, which is the point:
one question, one answer, no special-casing by kind.

### 2. Topology — "what runs on this host?"

```sparql
SELECT DISTINCT ?svc WHERE {
  ?s ?p ?o . ?s rdfs:label ?svc . ?o rdfs:label ?h .
  FILTER(regex(str(?p), "runs_on")) FILTER(regex(?h, "^aurora$"))
}
```

→ `photo-app`, `media-hub`.

### 3. Dependency / impact — "what breaks if X goes down?"

```sparql
SELECT ?dependent WHERE {
  ?s ?p ?o . ?s rdfs:label ?dependent . ?o rdfs:label ?target .
  FILTER(regex(str(?p), "depends_on")) FILTER(regex(?target, "^ledger-db$"))
}
```

→ `photo-app` depends on `ledger-db` (so a ledger-db outage takes photo-app with it).

### 4. Temporal — "what's the *current* policy?" (newest-wins)

```sparql
SELECT ?current ?retired WHERE {
  ?a ?p ?b . ?a rdfs:label ?current . ?b rdfs:label ?retired .
  FILTER(regex(str(?p), "supersedes"))
}
```

→ `policy-gitops-deploys` **supersedes** `policy-manual-deploys`. The graph, not a human,
resolves which policy is in force. This is the pattern most graph demos skip.

**And you can resolve it without the `supersedes` edge at all** — each policy carries the
date it was issued, so newest-wins falls out of the data:

```sparql
SELECT ?policy ?issued WHERE {
  ?p ex:issued_on ?issued . ?p rdfs:label ?policy .
} ORDER BY DESC(?issued)
```

→ 2 rows, newest first: `policy-gitops-deploys` (2025-09-01), then
`policy-manual-deploys` (2025-02-01). Add `LIMIT 1` for "just the current one".

That matters because the two answers are independent: `supersedes` is an assertion someone
made, `issued_on` is a fact about each policy. When they disagree, you have found a bug in
your graph rather than a wrong answer.

### 5. Inventory / classification — "list everything of a kind"

```sparql
SELECT DISTINCT ?policy WHERE { ?s ?p ?o . ?s rdfs:label ?policy . FILTER(regex(?policy, "^policy-")) }
```

→ 2 rows: `policy-manual-deploys`, `policy-gitops-deploys`.

`DISTINCT` is load-bearing: the pattern matches once per triple on each policy node, so
without it this returns **13 rows** — the same two policies, repeated. A count you did not
ask for is not an inventory.

### 6. Provenance — "where did this come from?"

Every fact is ingested inside a named **episode** with a `source`, so you can always trace
a claim back to what asserted it and when.

## Why a graph (vs. grep / a wiki)

Ownership, topology, and dependencies are *relationships*. A graph answers "who owns MCP",
"what runs on aurora", and "what supersedes the old policy" directly — instead of hoping a
doc is current and greppable. Add facts as episodes; the answers stay live.
