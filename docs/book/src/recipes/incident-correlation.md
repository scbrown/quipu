# Incident Correlation

Recipes for linking incidents to infrastructure, code, and time.

## Record an Incident

Ingest an incident as an episode with edges to affected services:

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "inc-2026-04-02-dns",
    "source": "pagerduty-agent",
    "group_id": "incidents",
    "nodes": [
      {
        "name": "inc-dns-outage",
        "type": "Incident",
        "description": "DNS resolution failures across all services",
        "properties": {
          "severity": "P1",
          "started": "2026-04-02T14:30:00Z",
          "resolved": "2026-04-02T16:00:00Z",
          "root_cause": "pihole OOM after update"
        }
      }
    ],
    "edges": [
      {"source": "inc-dns-outage", "target": "pihole", "relation": "causedBy"},
      {"source": "inc-dns-outage", "target": "traefik", "relation": "affected"},
      {"source": "inc-dns-outage", "target": "grafana", "relation": "affected"}
    ]
  }'
```

## Query: What Caused This Incident?

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?incident ?cause ?description
WHERE {
  ?incident a ont:Incident .
  ?incident ont:causedBy ?cause .
  ?incident rdfs:comment ?description .
}
```

## Query: What Has Pihole Caused?

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?incident ?desc
WHERE {
  ?incident ont:causedBy <http://aegis.gastown.local/ontology/pihole> .
  ?incident rdfs:comment ?desc .
}
```

## Query: Incident History for a Service

"Show me every incident that affected grafana":

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?incident ?description ?cause
WHERE {
  ?incident ont:affected <http://aegis.gastown.local/ontology/grafana> .
  ?incident rdfs:comment ?description .
  OPTIONAL { ?incident ont:causedBy ?c . ?c rdfs:label ?cause . }
}
```

## Correlate Incidents with Deployments

If you track deployments as episodes (see [Agent Builder](../tutorials/agent-builder.md)),
you can correlate:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX prov: <http://www.w3.org/ns/prov#>

SELECT ?incident ?deploy
WHERE {
  ?incident a ont:Incident .
  ?incident ont:causedBy ?svc .
  ?deploy a ont:Deployment .
  ?deploy ont:deploys ?svc .
  ?incident rdfs:label ?incLabel .
  ?deploy rdfs:label ?deploy .
}
```

## Time-Travel: State Before the Incident

"What did the infrastructure look like before things broke?"

```bash
curl -s localhost:3030/query -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "query": "PREFIX ont: <http://aegis.gastown.local/ontology/> PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> SELECT ?svc ?host WHERE { ?svc ont:runsOn ?host }",
    "valid_at": "2026-04-02T14:00:00Z"
  }'
```

Compare this with the state at incident time to see what changed.

## Provenance: Which Agent Reported This?

```sparql
PREFIX prov: <http://www.w3.org/ns/prov#>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?entity ?episode ?source
WHERE {
  ?entity prov:wasGeneratedBy ?ep .
  ?ep rdfs:label ?episode .
  ?ep <http://aegis.gastown.local/ontology/source> ?source .
}
ORDER BY ?source
```

## Pattern: Incident Dashboard Query

A monitoring agent can run this periodically to build a dashboard:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>

SELECT ?svc ?label (COUNT(?inc) AS ?incidentCount)
WHERE {
  ?inc ont:affected ?svc .
  ?svc rdfs:label ?label .
}
GROUP BY ?svc ?label
ORDER BY DESC(?incidentCount)
```

Services with the most incidents are your reliability hotspots.
