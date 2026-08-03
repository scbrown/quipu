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
quipu knot data.ttl --timestamp 2026-03-15T00:00:00Z --db my.db  # Source-true valid-time
```

| Flag | Description |
|------|-------------|
| `--shapes <file>` | SHACL shapes file for write-time validation |
| `--timestamp <ISO-8601>` | `valid_from` for the facts (default: now). Supply the source event time when ingesting history |

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
quipu episode deploy.json --base-ns "https://quarterdeck.internal/ontology#" --db my.db
quipu episode deploy.json --timestamp 2026-03-15T00:00:00Z --db my.db
```

| Flag | Description |
|------|-------------|
| `--base-ns <IRI>` | Namespace to mint entity IRIs in (default: the built-in aegis namespace). Lets non-aegis deployments use the episode abstraction |
| `--timestamp <ISO-8601>` | `valid_from` for the facts (default: now) |

### `quipu retract <entity-IRI>`

Retract facts for an entity.

```bash
quipu retract "http://example.org/old-service" --db my.db
quipu retract "http://example.org/alice" --predicate "http://example.org/email" --db my.db
```

| Flag | Description |
|------|-------------|
| `--predicate <IRI>` | Only retract facts with this predicate |
| `--timestamp <ISO-8601>` | Transaction valid-time for the retraction (default: now) |

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

### `quipu reason`

Run the Datalog reasoner to derive facts from rules.

```bash
quipu reason --db my.db
quipu reason --rules custom-rules.ttl --db my.db
# --reactive needs a non-default feature (see below):
quipu reason --reactive --db my.db   # requires: cargo build --features reactive-reasoner
```

| Flag | Default | Description |
|------|---------|-------------|
| `--rules <file>` | `shapes/aegis-rules.ttl` | Turtle file containing rules |
| `--reactive` | off | Register reactive observer after evaluation. **Requires the non-default `reactive-reasoner` feature**; on a build without it, `quipu reason --reactive` errors and exits non-zero rather than silently doing nothing. |

Output shows asserted/retracted counts per rule. Derived facts are written
with `source = "reasoner:<rule-id>"` provenance.

See [Reasoner Reference](reasoner.md) for full details on rule syntax and
the evaluation model.

### `quipu repl`

Interactive SPARQL prompt.

```bash
quipu repl --db my.db
```

Type SPARQL queries at the prompt. Use `:quit` or `:q` to exit.

### `quipu audit <trace.jsonl>`

Check an enforcement trace against the constraint specification in the store —
SARC's `T ⊨ Σ`.

```bash
quipu audit ~/.local/state/hank/metrics.jsonl --db my.db
quipu audit trace.jsonl --json --db my.db
```

| Flag | Default | Description |
|------|---------|-------------|
| `--json` | off | Emit the full report as one JSON object instead of readable lines |

**Exit code `1` when the trace contradicts the spec, `0` otherwise** — so a CI
job can gate on it without parsing anything.

Four passes run over every record: **coverage** (is every constraint the trace
cites actually in Σ, and does every refusal name one), **placement** (was each
constraint evaluated at a point its class can be enforced at, and does the
record agree with Σ about its class), **outcome** (does the response taken match
the one declared, at the recorded mode), and **attribution** (does the record say
who is answerable). Every pass is a comparison between two declared values; none
of them calls a model.

Findings come in two severities and only one of them fails the gate:

- **violation** — the trace *contradicts* Σ. A soft constraint that blocked, a
  declared `deny` that only warned under `enforce`, a record whose declared
  principal chain disagrees with the process that ran.
- **incompleteness** — the trace does not say *enough* to decide. No principal
  chain, no declared class, a constraint Σ declares that this window never
  exercised.

Incompleteness never changes the exit code. A checker that failed the build over
a missing `planner` would be switched off within a week, and then the violations
would stop being caught too.

Two limits worth stating before reading a `T ⊨ Σ` result as reassurance. Coverage
is checked in the direction quipu can decide — nothing is cited that Σ does not
define — because the other direction, *was every constraint that applied
evaluated*, means re-running the selector against the file as it stood, and quipu
has neither the file nor the parser. And the report counts lines it could not
read rather than skipping them, so `N line(s) unreadable` is always part of the
summary: conformance over a window that was only partly read is not conformance.
