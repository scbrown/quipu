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
constraint evaluated at a point its class can be enforced at, does the record
agree with Σ about its class, and does the layer that actually evaluated it match
the `aegis:hostedAtLayer` the policy claims — SARC I6), **outcome** (does the
response taken match the one declared, at the recorded mode), and
**attribution** (does the record say who is answerable). Every pass is a
comparison between two declared values; none of them calls a model.

The I6 check is one-directional. A policy claiming `"tool"` while a hook in the
agent's own loop evaluated it is a violation — it reads as enforced somewhere an
agent cannot route around while being enforced somewhere an agent can. A policy
claiming `"orchestration"` while something stronger enforced it is silent:
understating your own robustness misleads nobody in a direction that costs them.

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

### `quipu audit inventory`

Check the **dispatch graph** rather than a trace — SARC I7, enforcement
completeness.

```bash
quipu audit inventory --db my.db
quipu knot shapes/dispatch-inventory.ttl --db my.db   # load the shipped seed first
```

I7 is a property of the dispatch graph, not of any one constraint: a harness
exposes N classes of tool call, and completeness is the question of whether every
class that can change state passes through a point where a constraint could stop
it. `aegis:ToolClass` declares each class, whether it is `executable`, and which
`governedAt` points it traverses.

Findings, in the same two severities:

- **violation** — an executable class that traverses no enforcement point and has
  no `aegis:ungovernedReason`. An unknown hole.
- **incompleteness** — an executable class that traverses nothing but says
  *why*: an **acknowledged bypass surface**. Reported on every run, because a
  bypass surface an operator has stopped seeing is one they have stopped
  weighing. Also: a class that does not declare `aegis:executable`, since whether
  it needs a point is then undecidable.
- **violation**, the other direction — a constraint in Σ placed at a point no
  declared executable class traverses. It reads as governance in the catalog and
  can never fire in the deployment.

An empty inventory is reported as an incompleteness, never as a pass: an unwritten
dispatch graph is not an empty one.

`shapes/dispatch-inventory.ttl` ships the seed for this stack — the edit path and
quipu's own write gate as governed, reads as non-executable, and Bash, `Task`, CI
pipelines, cron, remote shells, a sibling session's VCS index and a hostile agent
as acknowledged surfaces with where each is enforced instead. Nothing derives it
from the harness's actual tool registry, so it can drift from reality the way a
prose list does; the difference is that a drifted declaration is a wrong answer
to a question something asks rather than a paragraph nobody re-reads.

### `quipu audit replay <trace.jsonl>`

Re-check a recorded window against the **current** Σ and report what promoting
each rule from `advise` to `enforce` would do.

```bash
quipu audit replay ~/.local/state/hank/metrics.jsonl --db my.db
quipu audit replay trace.jsonl --json --db my.db
```

Exits `0` whatever it finds. Replay reports *readiness*, and readiness is a
judgement an operator makes: failing a build because a rule has not yet fired
would turn "we have not measured this" into "this is broken", which are different
states needing different responses.

Per rule, five gates — each a reason not to promote:

| gate | what it asks | why it blocks promotion |
|---|---|---|
| liveness | did it ever fire? | a rule promoted without firing has been tested by nothing |
| both outcomes | did it record `satisfied` *and* `unsatisfied`? | a one-sided check is vacuous or universal, and neither is distinguishable from broken |
| in spec | is it in Σ at all? | a rule enforcing outside the specification has nothing to be promoted *to* |
| recoverability | after a refusal, did work on that target ever succeed? | a rule nobody has got past is an outage with a reason attached |
| new blocks | how many more actions would `enforce` refuse? | not a gate — the number the operator is actually deciding about |

Nothing is re-evaluated. The predicate needed the file as it stood and that file
is gone, so this is deterministic arithmetic over records rather than a
simulation.

**Three limits, printed with every summary rather than kept in a footnote.** It
measures only traffic that happened, so a rule that would block a kind of edit
nobody attempted shows zero new blocks and is not therefore safe. It counts
false-positive *candidates* and never false positives — a block is wrong only if
the action was legitimate, and no record carries that judgement. And it bounds no
false negatives at all: actions a rule let through without firing look exactly
like actions it correctly approved.

### `quipu audit tree <trace.jsonl>`

Reassemble the dispatch forest from the principal chains a trace carries.

```bash
quipu audit tree trace.jsonl
quipu audit tree trace.jsonl --json
```

Needs no store — the tree is a property of the trace alone — and exits `0`
always. A shape is not a verdict; the findings that *are* verdicts (a laundered
chain, a partial attribution tuple) belong to `quipu audit <trace>`.

SARC §9.5's **attribution dilution** is what this addresses: an orchestrator
dispatches, a worker acts, and a flat record cannot say which link was
answerable. The trace this stack emits is a **sequence**, so the tree here is
*reconstructed* rather than structural, and the output says so in three places:

- **Unattributed records are not placed.** A record with no chain is counted and
  left out. Attaching it to whichever root happened to be first would invent an
  answer to the question the tree exists to answer.
- **Implied dispatch nodes are flagged.** A chain `[orchestrator, worker]` proves
  an orchestrator exists; it does not prove the orchestrator's own actions are in
  this window. "This agent did nothing" and "this agent's actions were not
  recorded" are different facts and only one is good news.
- **Collapsed nodes get a note.** Two separate dispatches of the same worker by
  the same caller produce the same chain and land on one node. Not an error — one
  agent legitimately does many things — but the reader must not be told the tree
  is unambiguous when it is not.
