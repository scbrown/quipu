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
| `--fork <name>` | Scope the default graph to a named fork (see `quipu fork`); unknown or dropped forks are refused |

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

Export deterministic RDF from ROOT or one explicit scope.

```bash
quipu export --db my.db                        # N-Triples (default)
quipu export --format turtle --db my.db        # Turtle
quipu export --group-id project-a --db my.db   # provenance group
quipu export --construct 'CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }' --db my.db
```

| Flag | Description |
|------|-------------|
| `--format <fmt>` | Output format: `ntriples` (default) or `turtle` |
| `--graph <iri>` | Export one named graph |
| `--group-id <id>` | Export entities attributed to one episode group |
| `--construct <query>` | Export a SPARQL CONSTRUCT or DESCRIBE graph |

The three scope flags are mutually exclusive. Omit all three for ROOT.

### `quipu share`

Write a deterministic directory intended for git storage and interchange.

```bash
quipu share --output knowledge-share --db my.db
quipu share --output project-share --group-id project-a --shapes project-shapes --turtle
quipu share --output next-share --parent-share sha256:abc123 --db my.db
```

| Flag | Description |
|------|-------------|
| `--output <dir>` | New destination directory (required; an existing path is refused) |
| `--graph <iri>` | Share one named graph |
| `--group-id <id>` | Share entities attributed to one episode group |
| `--construct <query>` | Share a SPARQL CONSTRUCT or DESCRIBE result |
| `--shapes <name>` | Include one loaded shape set; repeatable |
| `--parent-share <id>` | Record the prior `share_id` in this lineage |
| `--turtle` | Add the derived, human-readable `export.ttl` view |

The three scope flags are mutually exclusive and default to ROOT. Required
payloads are `export.nt`, `shapes.ttl`, and `manifest.json`. The graph payload
is sorted and duplicate-free; the manifest hashes the exact payload bytes and
uses the anchored transaction timestamp, so unchanged state produces
byte-identical output.

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

### `quipu impact <entity-IRI>`

Bounded BFS over entity edges: what is downstream of this entity? With
`--remove`, speculatively retracts the entity (SQLite savepoint, no mutation),
re-runs the reasoner inside the fork, and walks the result — "what would break
if I removed this?".

```bash
quipu impact http://example.org/traefik --hops 3 --db my.db
quipu impact http://example.org/traefik --remove --db my.db
```

| Flag | Description |
|------|-------------|
| `--remove` | Counterfactual: impact of removing the entity |
| `--hops <N>` | Walk depth (default from `DEFAULT_HOPS`) |
| `--predicate <IRI>` | Restrict to these predicates (repeatable) |

### `quipu project`

Run graph algorithms over the projected knowledge graph: `stats`, `in_degree`,
`pagerank`/`ppr`, `components`, `louvain`, `shortest_path`.

```bash
quipu project --algorithm pagerank --limit 10 --db my.db
quipu project --algorithm pagerank --seed http://example.org/alice --db my.db  # PPR
quipu project --algorithm shortest_path --from <IRI> --to <IRI> --db my.db
```

| Flag | Description |
|------|-------------|
| `--algorithm <name>` | Algorithm to run (default: `stats`) |
| `--type <IRI>` / `--predicate <IRI>` | Restrict the projection |
| `--graph <IRI>` | Project one named graph's own facts instead of ROOT |
| `--seed <IRI>` | PPR seed (repeatable; switches pagerank to personalized) |
| `--damping` / `--max-iters` / `--tolerance` | PageRank parameters |
| `--limit <N>` | Max results (default: 20) |
| `--from` / `--to` | Endpoints for `shortest_path` |

### `quipu report`

Graph health report: hub entities (god-nodes), surprising connections, and
suggested competency questions.

```bash
quipu report --hubs 10 --surprises 5 --db my.db
```

| Flag | Description |
|------|-------------|
| `--hubs` / `--surprises` / `--questions` | How many of each to return |
| `--type <IRI>` / `--predicate <IRI>` | Restrict the underlying projection |

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
- **incompleteness**, the zero-trust boundary — a class declaring
  `aegis:importsUntrustedState` brings content into the agent's context that has
  not been through this deployment's constraints (a sub-agent's response, an MCP
  server's output, retrieved documents). Reported whether or not the class is
  governed: `governedAt` says its own *actions* traverse a point and says nothing
  about what it *returned*. No trust predicate evaluates imported content today,
  so this is an open boundary reported on every run rather than a closed gap. A
  class that imports and declares no `aegis:untrustedOrigin` is a **violation** —
  an import channel nobody can describe is one nobody can weigh.

An empty inventory is reported as an incompleteness, never as a pass: an unwritten
dispatch graph is not an empty one.

`shapes/dispatch-inventory.ttl` ships the seed for this stack — the edit path and
quipu's own write gate as governed, reads as non-executable, and Bash, `Task`, CI
pipelines, cron, remote shells, a sibling session's VCS index and a hostile agent
as acknowledged surfaces with where each is enforced instead. Nothing derives it
from the harness's actual tool registry, so it can drift from reality the way a
prose list does; the difference is that a drifted declaration is a wrong answer
to a question something asks rather than a paragraph nobody re-reads.

### `quipu audit namespace`

List the base-namespace predicates **episode ingest minted** that no loaded shape
mentions — namespace drift, in the same shape as `quipu audit inventory`.

```bash
quipu audit namespace --db my.db
quipu audit namespace --graph urn:example:tenant --json --db my.db
```

Exits `0` whatever it finds, and refuses nothing. Every key in an episode node's
`properties` map becomes a predicate in the base namespace via
`sanitize_iri_local`, with no shape governing which keys are admissible, so
agents writing free-form properties mint predicates indefinitely and nothing
reported the drift. A *gate* here would reject writes every deployment is already
making — the ontology in the store today was grown by exactly this path — so it
would be switched off within a day and the drift would go back to being
invisible. A report an operator reads beats a gate nobody leaves on.

Per ungoverned predicate: the IRI, how many current facts use it, how many
distinct episode-written subjects carry it, and the window it has been in use.

```text
namespace: 2 ungoverned predicate(s), 1 governed, minted by episode ingest over
2 episode-written subject(s) in urn:quipu:graph:root against 1 loaded shape(s)

UNGOVERNED http://aegis.gastown.local/ontology/rackUnit: 1 fact(s) on 1 subject(s),
in use 2026-01-01T00:00:00Z .. 2026-01-01T00:00:00Z
```

**What counts as minted here.** A predicate is reported when its subject carries
`prov:wasGeneratedBy` pointing at a `{base}episode_…` activity, the predicate is
in the configured base namespace, and the object is a **literal**. That last
condition is what separates the `properties` map from the edge path: edge
relations resolve to node references and already pass through
`resolve_edge_predicate`, which is a fence. The two predicates episode ingest
emits structurally — `aegis:groupId` and `aegis:contentHash` — are excluded by
name, because the writer's own vocabulary reported as agent drift would put a
permanent floor under every report.

**What "no shape mentions it" means.** A predicate is treated as governed if its
IRI appears *anywhere* in any loaded shape's graph — as an `sh:path`, a target,
or any other position. That is the widest reading of "mentions", chosen
deliberately: this is a report an operator acts on, and a false alarm costs more
here than a missed one.

**What the seen window honestly is.** `first_seen` / `last_seen` are the earliest
and latest `valid_from` among the facts using the predicate. The store keeps no
separate mint timestamp, so this answers "since when has this predicate been in
use", not "when was this IRI first interned" — and a fact re-asserted with an
older valid time genuinely moves `first_seen` backwards.

Scans the ROOT graph by default; `--graph <iri>` scans one named graph instead. A
graph IRI that names no graph is an error, not an empty result — "no drift in the
graph you named" and "there is no such graph" are different answers and only one
should let an operator stop looking.

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

### `quipu audit inheritance <trace.jsonl>`

Check whether a constraint declared `aegis:inheritedByDelegates` stopped applying
under delegation — SARC §9.5's **constraint laundering**.

```bash
quipu audit inheritance trace.jsonl --db my.db
```

Exits `1` on a violation, like `quipu audit <trace>`.

An orchestrator is bound by a constraint, dispatches a sub-agent, and the
constraint is not re-applied at the deeper layer. Nobody decided to drop it; it
simply was not carried, and the record looks exactly like a constraint that
legitimately did not apply.

Two findings at deliberately different severities, because one has evidence and
the other has a question:

- **Violation.** Constraint *C* decided on target *T* under chain
  `[orchestrator]`, and a deeper action on the **same target** under
  `[orchestrator, worker]` did not evaluate it. *C* proved it could decide for
  *T*, so its absence deeper is a drop.
- **Incompleteness.** *C* was evaluated at a dispatch node and never anywhere in
  its subtree. That might be laundering and might be a selector that legitimately
  matched nothing deeper; deciding needs the files as they stood, which quipu
  does not have.

Reporting both as violations would make the strong one unfindable.

`aegis:onUndecidable` admits only `"escalate"` — the same shape as
`aegis:onTimeout` admitting only `"deny"`. That is the **decidability rescue**:
evaluate at the deepest layer where the constraint still decides, or hand it to a
human. A constraint that silently stops applying where it cannot be checked is
one an agent escapes by dispatching into a context where the evidence is absent.

A Σ where no constraint declares `inheritedByDelegates` reports an
incompleteness, never a clean pass: "nothing is inherited" and "nothing has said
whether anything is inherited" are different states.

### `quipu propose`

Schema-change proposal queue: submit a shapes/ontology change for review
rather than applying it directly.

```bash
quipu propose list --status pending --db my.db
quipu propose submit shape ex:PersonShape new-shape.ttl --proposer agent-1 --rationale "tighten cardinality"
quipu propose accept 3 --note "LGTM" --db my.db
quipu propose reject 4 --note "breaks existing data" --db my.db
```

| Subcommand | Description |
|------------|-------------|
| `list [--status pending]` | List proposals |
| `submit <kind> <target> <file.ttl> --proposer <id> [--rationale <text>] [--trigger <ref>]` | Queue a change |
| `accept <id> [--note <text>]` | Apply and record the decision |
| `reject <id> --note <reason>` | Reject with a reason |

### `quipu policy`

Policy by example: draft a placement-aimed advisory policy from an exemplar,
then replay it over recorded history *before* anything is created. The ordering
is the point — draft, backtest, read the hit list, and only then `quipu knot`
the file, at which point the definition-time placement check still runs and can
still refuse.

```bash
quipu policy draft --exemplar http://example.org/verdict/17 --name no-bare-secrets \
  --label "never commit a bare secret again" \
  --targets http://example.org/CodeEdit \
  --claim 'ASK { FILTER NOT EXISTS { $target ex:containsSecret true } }' \
  --out draft.ttl
quipu policy backtest draft.ttl --last-txs 500 --db my.db
quipu knot draft.ttl --db my.db
```

| Subcommand | Description |
|------------|-------------|
| `draft --exemplar <iri> --name <slug> --label <sentence> --targets <type-iri> --claim <ask>` | Emit advisory Turtle for one policy. Never writes to the store |
| `backtest <candidate.ttl>` | Replay the candidate over the store's transaction log |

`draft` flags:

| Flag | Description |
|------|-------------|
| `--exemplar <iri>` | The Verdict / `DecisionRequest` / edit record that motivated the rule (required) |
| `--name <slug>` | Local name for the policy IRI; sanitised to `[A-Za-z0-9_-]` (required) |
| `--label <sentence>` | The intent sentence, kept verbatim as `rdfs:label` (required) |
| `--targets <type-iri>` | Target entity type, `aegis:targets` (required) |
| `--claim <ask>` | The compliant condition: a SPARQL ASK over `$target` (required) |
| `--class soft\|hard` | `aegis:constraintClass` (default: `soft`) |
| `--point <point>` | `aegis:verificationPoint` (default: derived from the class — soft→PAA, hard→PAG) |
| `--layer <layer>` | `aegis:hostedAtLayer` (default: `tool`) |
| `--authority <who>` | `aegis:authority` on the parent Directive |
| `--out <file.ttl>` | Write the Turtle to a file instead of stdout |

A drafted policy is **born advisory** — `aegis:effect "warn"` is a constant, not
a flag. Promotion to enforcement goes through the existing advisory→enforcing
gates over recorded traffic.

`backtest` flags:

| Flag | Description |
|------|-------------|
| `--last-txs <N>` | Window the replay to the last N transactions (default: the whole log) |
| `--from-tx <A> --to-tx <B>` | Explicit transaction window; both must be given together |

Output is one line per hit (`tx <id> (<timestamp>): would have fired on
<target>`) followed by a summary. The summary distinguishes "0 hits" from
"cannot evaluate", and the command **exits 1 when nothing could be measured**
so a script that knots on success cannot read an unevaluable candidate as
clean.

### `quipu path`

Golden-path analysis over recorded trajectories: the provenance cone, the
backtest, and a grammar draft. All three are reads; `draft` prints Turtle for a
human to review and load. See the
[golden paths design](https://github.com/scbrown/quipu/blob/main/docs/design/golden-paths-blessing.md).

```bash
quipu path cone http://example.org/traj/42 --via http://example.org/derivedFrom --hops 6 --db my.db
quipu path backtest http://example.org/traj/42 --omit http://example.org/step/3 --json --db my.db
quipu path draft http://example.org/traj/42 --name fast-review --label "the short path" \
  --via http://example.org/derivedFrom \
  --omit http://example.org/step/3 --by http://example.org/decision/9 --db my.db
```

The trajectory IRI is the first positional argument to every subcommand.

| Subcommand | Description |
|------------|-------------|
| `cone <trajectory-IRI>` | Which steps did the falsifier-gated verified result depend on? |
| `backtest <trajectory-IRI>` | Replay a pruned candidate over past trajectories sharing a work-item topic |
| `draft <trajectory-IRI>` | Emit `gp-grammar/1` Turtle for the blessed path |

| Flag | Subcommands | Description |
|------|-------------|-------------|
| `--via <predicate-IRI>` | `cone`, `draft` | Derivation predicate to walk, in addition to `verifiedBy` (always followed). Repeatable |
| `--hops <N>` | `cone` | Depth bound for the derivation walk (default: 8) |
| `--omit <step-IRI>` | `backtest`, `draft` | Step the candidate omits. Repeatable |
| `--by <decision-IRI>` | `draft` | The human Decision authorising the paired `--omit`. Repeatable |
| `--dead-end <step-IRI>` | `draft` | Mark a step a dead end in the drafted grammar. Repeatable |
| `--name <local-name>` | `draft` | Local name for the drafted grammar (required) |
| `--label <text>` | `draft` | Human label for the drafted grammar (required) |
| `--json` | `cone`, `backtest` | Emit the report as JSON instead of the text table |

`cone` verdicts are `IN-CONE` (load-bearing; pruning needs a human Decision),
`OUT-OF-CONE` (mechanically prunable) or `CANNOT-EVALUATE` (no derivation edges
recorded — never silently prunable). `draft` refuses when the count of `--omit`
flags does not match the count of `--by` flags: a human cut without its Decision
is a silent edit of history.

### `quipu ontology`

Manage stored OWL ontologies (versioned: re-loading a name closes the prior
version). Requires the `owl` feature.

```bash
quipu ontology load my-domain domain.ttl --db my.db
quipu ontology list --db my.db
quipu ontology remove my-domain --db my.db
```

### `quipu doctor labels`

Diagnose graph-label state: which graphs carry freshness/trust/policy labels
and which are undeclared.

```bash
quipu doctor labels --db my.db
```

### `quipu pack` / `quipu unpack`

Knowledge packs: export one named graph as a self-describing, attachable
`.qpack.db` artifact (facts, manifest, shapes, stored queries, optionally
vectors), verify one, or import one into a local graph.

```bash
quipu pack urn:example:graph --out domain.qpack.db --name "domain" --version 1.0.0
quipu pack urn:example:graph --out domain.qpack.db --shapes s.ttl --queries q.json --with-vectors
quipu pack urn:example:graph --out domain.qpack.db --space 7
quipu pack --verify domain.qpack.db
quipu unpack domain.qpack.db --into urn:local:domain --db my.db
```

| Flag | Description |
|------|-------------|
| `--out <file>` | Output pack path (required for pack) |
| `--name` / `--version` | Manifest metadata |
| `--space <N>` | Ship the pack in term space N so it attaches to a consumer without id collisions (same machinery as `quipu db respace`; the content hash is unchanged — a space moves ids, not content). Not applicable to `--format turtle` |
| `--shapes <S>` / `--queries <Q>` | Ship shape sets / stored queries (repeatable) |
| `--with-vectors` | Include embeddings (refused unless the SQLite vector backend is active) |
| `--format turtle` | Also embed a Turtle serialization |
| `--verify <file>` | Recompute and check the pack's content hash |
| `--into <graph-iri>` | Unpack target graph (default: the pack's own graph IRI) |

### `quipu graph`

The graph-registry commands: offline import, and the deep-freeze lifecycle
(see [Graph Kinds & Deep Freeze](../concepts/graph-kinds.md)).

```bash
quipu graph import other.db --as urn:app:imported --db my.db
quipu graph freeze urn:app:runs/2026-07 --out /var/quipu/archive --db my.db
quipu graph thaw urn:app:runs/2026-07 --db my.db
quipu graph list --kind operational --db my.db
quipu graph list --frozen --db my.db
```

`freeze` exports the graph's full history to a `.qpack.db` archive, verifies
it by content hash, deletes the local rows and re-attaches the pack
read-only; the graph stays queryable at the same IRI and refuses writes
until `thaw`. `list` prints `iri  class  kind  lifecycle  source` per graph.

### `quipu fork`

Persistent named forks (quipu-gp5): fork ROOT as of any transaction into an
independent committed-class named graph (`urn:quipu:fork:<name>`), read it
exactly like the main line, diff it, then drop it or promote it. Promotion
re-enters through the SHACL + policy write gates — a refused promotion writes
nothing and the fork stays open. Fork ergonomics are never a gate bypass.

```bash
quipu fork 42 --name experiment --db my.db     # fork ROOT as of tx 42
quipu fork list --db my.db
quipu read "SELECT ?s ?p ?o WHERE { ?s ?p ?o }" --fork experiment --db my.db
quipu fork diff main experiment --db my.db     # each side: a fork name, or 'main'
quipu fork promote experiment --db my.db       # delta re-enters via the gates
quipu fork drop experiment --db my.db          # terminal; the name is not reusable
```

| Subcommand | Description |
|------------|-------------|
| `<tx> [--name <n>]` | Create: materialize ROOT-as-of-`<tx>` into a new fork (default name `fork-<tx>`) |
| `list` | Name, fork-tx, status, created-at for every fork |
| `diff <a> <b>` | Present-state triple diff between two forks (or `main`) |
| `promote <name>` | Apply the fork's delta to ROOT through the write gates; SHACL refusal leaves ROOT untouched |
| `drop <name>` | Close the fork; its facts remain as history, the name is not reusable |

Reads: `--fork <name>` on `quipu read`, or the `fork` field on
`POST /query` / `quipu_query`. Unknown and dropped forks are refused
loudly — never a silent fall-through to ROOT.

### `quipu db attach --list`

List the databases mounted alongside this store — the `[[quipu.attachments]]`
layers (see [Configuration](../getting-started/configuration.md#attachments))
and deep freeze's archives, which no config declares.

```bash
quipu db attach --list --db my.db
```

Output is `alias`, `path`, and mount mode (always `ro`), tab-separated. A
declared layer that could not be mounted refuses the open instead of appearing
here, so everything listed is genuinely composed.

### `quipu db respace`

Move a store into a fresh term space so it can be attached to another store
without id collisions. Reads the source read-only; writes a new file.

```bash
quipu db respace --into 7 --out respace.db --db my.db
```

### `quipu events refusals`

Count refused writes by gate (`shacl | policy | authority | owl |
placement`) — the incident-rate denominator. Reads the `write.refused` events
the write gates record; the raw events are served by
`GET /events?types=write.refused`. See the REST API reference for what a
refusal event records (metadata only, never the refused bodies) and the
speculate exclusion.

```bash
quipu events refusals --db my.db
```

### `quipu graph import <db>`

Import another quipu database's ROOT graph as a named graph in this store.

```bash
quipu graph import other.db --as urn:import:other --db my.db
```

### `quipu migrate-vectors`

Migrate stored embeddings between vector backends (requires the `lancedb`
feature).

```bash
quipu migrate-vectors --from sqlite --to lancedb --dry-run --db my.db
```
