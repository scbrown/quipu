# CLI Commands

The `quipu` binary provides a command-line interface for all operations.

## Global Flags

| Flag | Description |
|------|-------------|
| `--db <path>` | Store database path (default: `.bobbin/quipu/quipu.db`) |

## Commands

### `quipu knot <file.ttl>`

Load RDF facts from a Turtle file.

```bash
quipu knot data.ttl --db my.db
quipu knot data.ttl --shapes schema.ttl --db my.db  # With SHACL validation
```

| Flag | Description |
|------|-------------|
| `--shapes <file>` | SHACL shapes file for write-time validation |

Alias: `load`

### `quipu read "<sparql>"`

Execute a SPARQL query.

```bash
quipu read "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10" --db my.db
quipu read "SELECT ?s WHERE { ?s a <http://ex.org/Person> }" --valid-at "2026-03-01"
```

| Flag | Description |
|------|-------------|
| `--valid-at <date>` | Time-travel: query as of this ISO-8601 timestamp |
| `--tx <N>` | Time-travel: query as of this transaction ID |

Alias: `query`

### `quipu cord`

List entities, optionally filtered by type.

```bash
quipu cord --db my.db
quipu cord --type "http://example.org/Person" --limit 50 --db my.db
```

| Flag | Description |
|------|-------------|
| `--type <IRI>` | Filter by rdf:type |
| `--limit <N>` | Maximum results (default: 100) |

### `quipu unravel`

Time-travel query: view facts at a past point.

```bash
quipu unravel --tx 5 --db my.db
quipu unravel --valid-at "2026-03-15T00:00:00Z" --db my.db
```

Requires at least one of `--tx` or `--valid-at`.

### `quipu episode <file.json>`

Ingest a structured episode from a JSON file.

```bash
quipu episode deploy.json --db my.db
echo '{"name": "test", "nodes": []}' | quipu episode - --db my.db  # stdin
```

### `quipu retract <entity-IRI>`

Retract facts for an entity.

```bash
quipu retract "http://example.org/old-service" --db my.db
quipu retract "http://example.org/alice" --predicate "http://example.org/email" --db my.db
```

| Flag | Description |
|------|-------------|
| `--predicate <IRI>` | Only retract facts with this predicate |

### `quipu shapes`

Manage persistent SHACL shapes.

```bash
quipu shapes load person-shape schema/person.ttl --db my.db
quipu shapes list --db my.db
quipu shapes remove person-shape --db my.db
```

Loaded shapes automatically validate all future writes.

### `quipu validate`

Dry-run SHACL validation without writing.

```bash
quipu validate --shapes schema.ttl --data test-data.ttl
```

### `quipu export`

Export all current facts in an RDF format.

```bash
quipu export --db my.db                        # N-Triples (default)
quipu export --format turtle --db my.db        # Turtle
```

| Flag | Description |
|------|-------------|
| `--format <fmt>` | Output format: `ntriples` (default) or `turtle` |

### `quipu stats`

Show store statistics.

```bash
quipu stats --db my.db
```

Output: fact count, entity count, predicate count.

### `quipu repl`

Interactive SPARQL prompt.

```bash
quipu repl --db my.db
```

Type SPARQL queries at the prompt. Use `:quit` or `:q` to exit.
