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

Remember that `hw:WebApp rdfs:subClassOf hw:Service`. Quipu automatically
expands type queries through the subclass hierarchy:

```sparql
PREFIX hw: <http://example.org/homelab/>

SELECT ?svc
WHERE {
  ?svc a hw:Service .
}
```

This returns **all six** services — including traefik, grafana, and nginx
(which are typed as `hw:WebApp`). Quipu follows `rdfs:subClassOf` edges
automatically, so you query at the level you care about.

### The asymmetry: two forms, two numbers, both correct

Expansion applies when the type is a **constant IRI**. It does **not** apply
when the type is a **variable** — that form is matched literally:

```sparql
# INFERRED — includes hw:WebApp instances
SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a hw:Service }

# ASSERTED ONLY — only things literally typed hw:Service
SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a ?t . FILTER(?t = hw:Service) }
```

Those two queries express what looks like the same constraint and return
different counts. **Neither is wrong**, and you need both:

| Question | Form |
|---|---|
| "what depends on X", blast radius, impact | constant IRI (inferred) |
| vocabulary census, governance gating, "who emits the wrong type" | variable + `FILTER` (asserted) |

To ask for inference explicitly — recommended when the distinction matters to
your reader — use the path form, which is equivalent to the constant form:

```sparql
SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s a/rdfs:subClassOf* hw:Service }
```

### How to tell which one you got

Ambiguity here is expensive: both queries return HTTP 200 and both counts are
individually plausible, so a number answering the *other* question does not look
wrong. Quipu therefore **announces expansion in the response** whenever it
widened a type constant:

```json
{
  "count": 6,
  "inference": {
    "applied": true,
    "expandedTypes": [
      {"type": "http://example.org/homelab/Service",
       "subclasses": ["http://example.org/homelab/WebApp"]}
    ]
  }
}
```

The field is **absent** when nothing was inferred, so its presence is the
signal. The subclasses are named because "inference happened" is not actionable
on its own — you need to see *which* types were folded in to know whether the
count is the one you wanted. A type with no subclasses is never reported: both
forms agree there, which also makes a leaf type a useful control when you are
checking this behaviour yourself.

#### Every result shape carries it — including ASK

The marker is not a SELECT feature. `ASK` is the shape most in need of it:

```sparql
ASK { hw:postgres a hw:Service }     # -> {"result": true, "inference": {...}}
```

`hw:postgres` may be asserted **only** as `hw:DatabaseService` — nothing in the
graph says it is a `hw:Service` — and this still answers `true`, because
expansion widened the question. A boolean gives you no number to look at twice,
so without the marker an inferred `true` is byte-for-byte identical to an
asserted one. `CONSTRUCT`/`DESCRIBE` carry it too, and there it matters for a
further reason: the triples you get back are inference-widened, and a
materialised graph is easy to write down somewhere and re-read later as fact.

**What the marker claims.** It says expansion was applied *to the query*. It does
**not** say the answer depended on it — a marked `ASK` can be `true` about a
directly-asserted fact that needed no inference at all. Read a marked `true` as
"this was the wide question", not as "this true is inferred". To settle whether
the inference mattered, ask the asserted form:

```sparql
SELECT ?t WHERE { hw:postgres a ?t }    # what is it ACTUALLY typed as?
```

#### Standard result formats: the marker moves to a header

If you request a W3C shape with `Accept`
(`application/sparql-results+json`, `application/sparql-results+xml`,
`text/turtle`), the body is fixed by spec and has nowhere to put the marker.
It travels as a response header instead, naming the widened type constants:

```http
x-quipu-inference: http://example.org/homelab/Service
```

Same rule: the header is **absent** when nothing was inferred. The body is
untouched and stays conformant, so a standard parser is unaffected — but a
client that ignores headers gets no signal, which is a reason to prefer the
default JSON shape when the distinction matters. Full `expandedTypes` detail is
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
