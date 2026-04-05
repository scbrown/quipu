# SHACL Validation

SHACL (Shapes Constraint Language) lets you define what valid data looks like
and enforce it at write time. When an agent or user tries to add facts that
violate a shape, Quipu rejects the write and returns structured feedback
explaining exactly what's wrong.

## Why Validate?

Without validation, agents can write anything:

```turtle
hw:koror hw:cpuCores "lots" .   # Should be an integer
hw:koror a hw:Host .            # Missing required hostname
```

With SHACL shapes loaded, Quipu catches these problems before they enter
the fact log.

## Defining a Shape

A shape declares constraints for a class of entities. Here's a shape that
says "every Host must have exactly one hostname (a string) and at least one
cpuCores (an integer)":

```turtle
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix hw:  <http://example.org/homelab/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

hw:HostShape
    a sh:NodeShape ;
    sh:targetClass hw:Host ;
    sh:property [
        sh:path hw:hostname ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
    ] ;
    sh:property [
        sh:path hw:cpuCores ;
        sh:datatype xsd:integer ;
        sh:minCount 1 ;
    ] .
```

## Loading Shapes

### CLI

```bash
quipu shapes load --name homelab --file shapes/homelab.shapes.ttl --db homelab.db
```

### REST API

```bash
curl -s localhost:3030/shapes -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "action": "load",
    "name": "homelab",
    "turtle": "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix hw: <http://example.org/homelab/> .\nhw:HostShape a sh:NodeShape ; sh:targetClass hw:Host ; sh:property [ sh:path hw:hostname ; sh:minCount 1 ] ."
  }'
```

### Listing loaded shapes

```bash
quipu shapes list --db homelab.db
```

## Validation in Action

Try to add a Host without a hostname:

```bash
quipu knot - --db homelab.db --shapes shapes/homelab.shapes.ttl <<'EOF'
@prefix hw: <http://example.org/homelab/> .
hw:badhost a hw:Host .
EOF
```

Quipu rejects it with structured feedback:

```json
{
  "conforms": false,
  "violations": 1,
  "issues": [
    {
      "severity": "Violation",
      "focus_node": "http://example.org/homelab/badhost",
      "path": "http://example.org/homelab/hostname",
      "component": "MinCountConstraintComponent",
      "message": "Less than 1 values for hw:hostname",
      "source_shape": "http://example.org/homelab/HostShape"
    }
  ]
}
```

This feedback is designed for agents — structured JSON with enough detail
to fix the problem automatically.

## Supported Constraints

| Constraint | What it checks |
|-----------|---------------|
| `sh:minCount` / `sh:maxCount` | Cardinality (how many values) |
| `sh:datatype` | Value type (xsd:string, xsd:integer, etc.) |
| `sh:minInclusive` / `sh:maxInclusive` | Numeric ranges |
| `sh:minLength` / `sh:maxLength` | String length |
| `sh:pattern` | Regex match |
| `sh:in` | Allowed values (enumeration) |
| `sh:class` | Referenced entity must have rdf:type |
| `sh:node` | Nested shape reference |
| `sh:or` / `sh:and` / `sh:not` | Logical constraints |
| `sh:equals` / `sh:disjoint` | Property pair constraints |

Quipu supports the full SHACL Core specification via the rudof library.

## Dry-Run Validation

Validate data without writing it to the store:

```bash
quipu validate --shapes shapes/homelab.shapes.ttl --data data.ttl
```

```bash
curl -s localhost:3030/validate -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "shapes": "@prefix sh: ... shapes turtle ...",
    "data": "@prefix hw: ... data turtle ..."
  }'
```

## Pre-Built Shapes

Quipu ships with shapes for the Aegis infrastructure ontology in the
`shapes/` directory. These cover:

- `LXCContainer`, `ProxmoxNode`, `BareMetalHost` — compute resources
- `SystemdService`, `WebApplication`, `Database` — services
- Common properties: hostname, ipAddress, memoryMB, cpuCores, dependsOn

Load them with:

```bash
quipu shapes load --name aegis --file shapes/aegis-ontology.shapes.ttl --db homelab.db
```

## Best Practices

- **Load shapes before data** — shapes must be present to validate incoming writes
- **One shape set per domain** — group related constraints (e.g., "homelab", "code")
- **Start permissive, tighten later** — begin with minCount constraints,
  add datatype checks as your ontology stabilizes
- **Use validation feedback** — the structured JSON is designed for automated
  remediation by agents
