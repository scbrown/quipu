# SPARQL from Zero

This tutorial teaches SPARQL using a concrete homelab dataset. Every query
has sample data, the query itself, and the results table — so you can follow
along by loading the data into Quipu and running the queries yourself.

## The Sample Dataset

Save this as `homelab.ttl`:

```turtle
@prefix hw:   <http://example.org/homelab/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

# --- Hosts ---
hw:koror      a hw:Host ;
    rdfs:label      "koror" ;
    hw:hostname     "koror.example" ;
    hw:cpuCores     "8"^^xsd:integer ;
    hw:memoryMB     "32768"^^xsd:integer ;
    hw:role         "hypervisor" .

hw:palau      a hw:Host ;
    rdfs:label      "palau" ;
    hw:hostname     "palau.example" ;
    hw:cpuCores     "4"^^xsd:integer ;
    hw:memoryMB     "16384"^^xsd:integer ;
    hw:role         "storage" .

hw:yap        a hw:Host ;
    rdfs:label      "yap" ;
    hw:hostname     "yap.example" ;
    hw:cpuCores     "4"^^xsd:integer ;
    hw:memoryMB     "8192"^^xsd:integer ;
    hw:role         "edge" .

# --- Services ---
hw:traefik    a hw:WebApp ;
    rdfs:label      "traefik" ;
    hw:runsOn       hw:koror ;
    hw:port         "443"^^xsd:integer ;
    hw:dependsOn    hw:pihole .

hw:pihole     a hw:Service ;
    rdfs:label      "pihole" ;
    hw:runsOn       hw:koror ;
    hw:port         "53"^^xsd:integer .

hw:grafana    a hw:WebApp ;
    rdfs:label      "grafana" ;
    hw:runsOn       hw:koror ;
    hw:port         "3000"^^xsd:integer ;
    hw:dependsOn    hw:prometheus .

hw:prometheus a hw:Service ;
    rdfs:label      "prometheus" ;
    hw:runsOn       hw:palau ;
    hw:port         "9090"^^xsd:integer .

hw:minio      a hw:Service ;
    rdfs:label      "minio" ;
    hw:runsOn       hw:palau ;
    hw:port         "9000"^^xsd:integer .

hw:nginx      a hw:WebApp ;
    rdfs:label      "nginx" ;
    hw:runsOn       hw:yap ;
    hw:port         "80"^^xsd:integer ;
    hw:dependsOn    hw:minio .

# --- Type hierarchy ---
hw:WebApp     rdfs:subClassOf hw:Service .
```

Load it:

```bash
quipu knot homelab.ttl --db homelab.db
```

## 1. Your First Query: SELECT

A SPARQL query matches patterns against the graph. The simplest pattern is
a single triple with a variable:

```sparql
SELECT ?host
WHERE {
  ?host a <http://example.org/homelab/Host> .
}
```

This says "find every `?host` that has type `hw:Host`." The `a` keyword
is shorthand for `rdf:type`.

Run it:

```bash
quipu read "SELECT ?host WHERE { ?host a <http://example.org/homelab/Host> }" \
  --db homelab.db
```

| ?host |
|-------|
| `http://example.org/homelab/koror` |
| `http://example.org/homelab/palau` |
| `http://example.org/homelab/yap` |

## 2. Multiple Patterns: JOIN

Add more patterns to narrow results. Patterns in the same `WHERE` block
are joined — every pattern must match:

```sparql
SELECT ?host ?cores
WHERE {
  ?host a <http://example.org/homelab/Host> .
  ?host <http://example.org/homelab/cpuCores> ?cores .
}
```

| ?host | ?cores |
|-------|--------|
| `hw:koror` | 8 |
| `hw:palau` | 4 |
| `hw:yap` | 4 |

## 3. Using Prefixes

Full IRIs are verbose. SPARQL supports `PREFIX` declarations (without the `@`
and trailing `.` that Turtle uses):

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?host ?cores
WHERE {
  ?host a hw:Host .
  ?host hw:cpuCores ?cores .
}
```

The results are identical. Use prefixes in every query from here on.

## 4. FILTER: Narrowing Results

`FILTER` applies conditions to bound variables:

```sparql
PREFIX hw:  <http://example.org/homelab/>
PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>

SELECT ?host ?mem
WHERE {
  ?host a hw:Host .
  ?host hw:memoryMB ?mem .
  FILTER(?mem > 10000)
}
```

| ?host | ?mem |
|-------|------|
| `hw:koror` | 32768 |
| `hw:palau` | 16384 |

### String filters

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?svc ?label
WHERE {
  ?svc a hw:Service .
  ?svc <http://www.w3.org/2000/01/rdf-schema#label> ?label .
  FILTER(CONTAINS(?label, "pi"))
}
```

| ?svc | ?label |
|------|--------|
| `hw:pihole` | "pihole" |

Available filter functions: `=`, `!=`, `<`, `>`, `<=`, `>=`, `&&`, `||`,
`!`, `BOUND()`, `CONTAINS()`, `REGEX()`, `LCASE()`, `isIRI()`.

## 5. OPTIONAL: Left Joins

Not every service has a `dependsOn` edge. `OPTIONAL` includes the match
if it exists, but doesn't exclude the row if it doesn't:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?svc ?dep
WHERE {
  ?svc a hw:Service .
  OPTIONAL { ?svc hw:dependsOn ?dep . }
}
```

| ?svc | ?dep |
|------|------|
| `hw:traefik` | `hw:pihole` |
| `hw:pihole` | |
| `hw:grafana` | `hw:prometheus` |
| `hw:prometheus` | |
| `hw:minio` | |
| `hw:nginx` | `hw:minio` |

Services without dependencies appear with an empty `?dep` column.

## 6. UNION: Combining Patterns

`UNION` matches rows from either branch:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?thing ?label
WHERE {
  {
    ?thing a hw:Host .
    ?thing <http://www.w3.org/2000/01/rdf-schema#label> ?label .
  }
  UNION
  {
    ?thing a hw:WebApp .
    ?thing <http://www.w3.org/2000/01/rdf-schema#label> ?label .
  }
}
```

Returns all hosts and web apps.

## 7. ORDER BY, LIMIT, OFFSET

Sort and paginate results:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?host ?mem
WHERE {
  ?host a hw:Host .
  ?host hw:memoryMB ?mem .
}
ORDER BY DESC(?mem)
LIMIT 2
```

| ?host | ?mem |
|-------|------|
| `hw:koror` | 32768 |
| `hw:palau` | 16384 |

`OFFSET 1 LIMIT 1` would skip koror and return only palau.

## 8. Aggregates: COUNT, SUM, AVG

Group and aggregate with `GROUP BY`:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?host (COUNT(?svc) AS ?serviceCount)
WHERE {
  ?svc hw:runsOn ?host .
}
GROUP BY ?host
ORDER BY DESC(?serviceCount)
```

| ?host | ?serviceCount |
|-------|---------------|
| `hw:koror` | 3 |
| `hw:palau` | 2 |
| `hw:yap` | 1 |

### Total resources

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT (SUM(?cores) AS ?totalCores) (SUM(?mem) AS ?totalMem)
WHERE {
  ?host a hw:Host .
  ?host hw:cpuCores ?cores .
  ?host hw:memoryMB ?mem .
}
```

| ?totalCores | ?totalMem |
|-------------|-----------|
| 16 | 57344 |

### HAVING: Filter on aggregates

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?host (COUNT(?svc) AS ?n)
WHERE {
  ?svc hw:runsOn ?host .
}
GROUP BY ?host
HAVING(?n > 1)
```

| ?host | ?n |
|-------|----|
| `hw:koror` | 3 |
| `hw:palau` | 2 |

## 9. RDFS Subclass Inference

Remember that `hw:WebApp rdfs:subClassOf hw:Service`. SPARQL type patterns are
asserted-only: a constant and a variable type form mean the same thing.

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?svc
WHERE {
  ?svc a hw:Service .
}
```

This returns the three services asserted directly as `hw:Service`. It does not
silently turn a syntactic type form into an RDFS entailment request.

### One asserted form; make inference explicit

These two forms now agree:

```sparql
# ASSERTED ONLY
SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a hw:Service }

# ASSERTED ONLY — equivalent spelling
SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ?t . FILTER(?t = hw:Service) }
```

For the inferred question, say so with the standard property path:

| Question | Form |
|---|---|
| vocabulary census, governance gating, "who emits the wrong type" | either asserted type form |
| "what depends on X", blast radius, impact | explicit `a/rdfs:subClassOf*` path |

To ask for inference explicitly:

```sparql
SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a/rdfs:subClassOf* hw:Service }
```

### Migration marker: what was withheld

Ambiguity here is expensive: both queries return HTTP 200 and both counts are
individually plausible, so a number answering the *other* question does not look
wrong. The constant form used to widen through subclasses; during the migration
window Quipu announces that withheld expansion only on queries whose answer could
have changed:

```json
{
  "count": 3,
  "inference": {
    "applied": false,
    "withheldTypes": [
      {"type": "http://example.org/homelab/Service",
       "subclasses": ["http://example.org/homelab/WebApp"]}
    ]
  }
}
```

The field is **absent** when the flip did not change the query, so its presence
is the signal. The subclasses are named so the reader can choose the explicit
path form when the inferred answer is intended. A leaf type is never reported:
the old and new answers already agree there.

#### Every result shape carries it — including ASK

The marker is not a SELECT feature. `ASK` is the shape most in need of it:

```sparql
ASK { hw:postgres a hw:Service }     # -> {"result": false, "inference": {...}}
```

`hw:postgres` may be asserted **only** as `hw:DatabaseService` — nothing in the
graph says it is a `hw:Service`, so this now answers `false`. A boolean gives you
no number to look at twice, so the marker makes the semantic change visible.
`CONSTRUCT`/`DESCRIBE` carry it too: their formerly inferred triples are likewise
withheld unless the query uses the explicit path.

**What the marker claims.** It says the old implicit expansion was withheld from
this query. It does not say the resulting answer necessarily changed: a marked
`ASK` can still be `true` about a directly asserted fact. To ask the inferred
question, use the explicit path; to inspect asserted types, ask:

```sparql
SELECT ?t WHERE { hw:postgres a ?t }    # what is it ACTUALLY typed as?
```

#### Standard result formats: the marker moves to a header

If you request a W3C shape with `Accept`
(`application/sparql-results+json`, `application/sparql-results+xml`,
`text/turtle`), the body is fixed by spec and has nowhere to put the marker.
It travels as a response header instead, naming the affected type constants:

```http
x-quipu-inference: withheld: http://example.org/homelab/Service
```

Same rule: the header is **absent** when the flip did not affect the query. The body is
untouched and stays conformant, so a standard parser is unaffected — but a
client that ignores headers gets no signal, which is a reason to prefer the
default JSON shape when the distinction matters. Full `withheldTypes` detail is
one `Accept`-free request away.

## 10. Property Paths

SPARQL 1.1 property paths let you traverse edges without binding
intermediate variables.

### Sequence (`/`)

"What hosts do web apps' dependencies run on?"

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?app ?depHost
WHERE {
  ?app a hw:WebApp .
  ?app hw:dependsOn/hw:runsOn ?depHost .
}
```

`hw:dependsOn/hw:runsOn` means: follow `dependsOn`, then follow `runsOn`.

| ?app | ?depHost |
|------|----------|
| `hw:traefik` | `hw:koror` |
| `hw:grafana` | `hw:palau` |
| `hw:nginx` | `hw:palau` |

### Transitive closure (`*` and `+`)

If you had a chain like `A dependsOn B dependsOn C`, you could traverse
the full dependency chain:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?svc ?transitiveDep
WHERE {
  ?svc hw:dependsOn+ ?transitiveDep .
}
```

`+` means "one or more hops." `*` means "zero or more" (includes the
starting node itself).

### Alternative (`|`)

Match either predicate:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?thing ?name
WHERE {
  ?thing (hw:hostname|<http://www.w3.org/2000/01/rdf-schema#label>) ?name .
}
```

### Reverse (`^`)

"What services does koror host?" using reverse traversal:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?svc
WHERE {
  hw:koror ^hw:runsOn ?svc .
}
```

`^hw:runsOn` means "follow `runsOn` edges backwards."

## 11. Temporal Queries

Every SPARQL query in Quipu can include a temporal context.

### Valid-time travel

"What did the homelab look like on March 15?"

```bash
quipu read "PREFIX hw: <http://example.org/homelab/>
SELECT ?host ?cores WHERE {
  ?host a hw:Host .
  ?host hw:cpuCores ?cores .
}" --db homelab.db --valid-at 2026-03-15
```

Via REST:

```bash
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "query": "PREFIX hw: <http://example.org/homelab/> SELECT ?host ?cores WHERE { ?host a hw:Host . ?host hw:cpuCores ?cores }",
    "valid_at": "2026-03-15"
  }'
```

### Transaction-time travel

"What did the database know after the first 5 transactions?"

```bash
quipu read "SELECT ?s ?p ?o WHERE { ?s ?p ?o }" --db homelab.db --tx 5
```

## 12. Other Query Forms

### ASK: Yes/No Questions

```sparql
PREFIX hw: <http://example.org/homelab/>

ASK { hw:koror a hw:Host }
```

Returns `true` or `false`.

### CONSTRUCT: Build New Triples

```sparql
PREFIX hw: <http://example.org/homelab/>

CONSTRUCT {
  ?svc hw:colocatedWith ?other .
}
WHERE {
  ?svc hw:runsOn ?host .
  ?other hw:runsOn ?host .
  FILTER(?svc != ?other)
}
```

Returns triples showing which services share a host.

### DESCRIBE: Entity Details

```sparql
PREFIX hw: <http://example.org/homelab/>

DESCRIBE hw:koror
```

Returns all triples where koror is the subject.

## Cheat Sheet

| Pattern | Meaning |
|---------|---------|
| `?x a hw:Host` | ?x has type Host |
| `FILTER(?n > 5)` | Numeric comparison |
| `FILTER(CONTAINS(?s, "abc"))` | Substring match |
| `OPTIONAL { ... }` | Include if available |
| `{ A } UNION { B }` | Either pattern |
| `GROUP BY ?x` | Aggregate per group |
| `ORDER BY DESC(?n)` | Sort descending |
| `LIMIT 10 OFFSET 5` | Paginate |
| `?x hw:a/hw:b ?y` | Path sequence |
| `?x hw:a+ ?y` | Transitive closure |
| `?x ^hw:a ?y` | Reverse edge |
| `?x (hw:a\|hw:b) ?y` | Either predicate |

## What's Next

- [Homelab Operator Tutorial](homelab-operator.md) — model a full infrastructure
- [Temporal Model](../concepts/temporal-model.md) — deep dive on time-travel
- [REST API Reference](../reference/rest-api.md) — every endpoint
