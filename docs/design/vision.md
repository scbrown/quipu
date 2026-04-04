# Quipu: AI-Native Knowledge Graph — Vision

> Created: 2026-04-04, crew/braino session (aegis-6ct6)
> Status: DRAFT — exploration bead, not implementation

## One-Line

An embeddable, agent-enforced knowledge graph that combines strict RDF/OWL
ontology with native vector search — "SQLite for knowledge graphs."

## Why "Quipu"?

A quipu (khipu) is the Incan knotted-string recording system — a
pre-Columbian knowledge graph encoded in textile. The metaphor is exact:

- **Cords** = entities (each cord represents a thing)
- **Knots** = facts (type, position, and grouping encode meaning)
- **Colors** = types/classes (different cord colors = different categories)
- **Pendant cords** = relationships (branching from main cord)
- **Positional encoding** = temporal ordering (events recorded in sequence)
- **Khipukamayuq** (readers) = agents that interpret the structure

Quipu is a Bobbin subsystem: Bobbin holds the thread (raw data, code,
context), Quipu ties knots of structured meaning into it. Same textile
family, complementary roles.

`quipu knot` (assert a fact), `quipu read` (query), `quipu unravel`
(time-travel), `quipu cord` (list entities).

## The Problem

We've lived with two failure modes:

**1. Traditional RDF stores demand too much ceremony.**
Jena, Stardog, GraphDB — they assume human ontology engineers carefully
designing schemas in Protégé. The tooling is powerful but the cost is
prohibitive: who writes the SHACL shapes? Who maintains the OWL axioms?
Result: strict stores stay empty, or get populated with junk that passes
schema validation but lacks semantic coherence.

**2. AI-native stores have no structure at all.**
Graphiti, Mem0, LangChain graph extractors — they let LLMs dump free-form
triples into property graphs. Entity names collide ("dolt" the service vs
"dolt" the tool), relationships are inconsistent strings, there's no
validation. The graph grows but doesn't compound — it's a pile of facts,
not a knowledge base.

**The insight: agents can do ontology engineering.** The work that was too
burdensome for humans (schema inference, entity resolution, relationship
typing, SHACL validation, dedup) is exactly what LLM agents are good at.
An AI-native store shouldn't relax structure — it should enforce it strictly,
using agents as first-class participants in the schema lifecycle.

## Core Thesis

> Start strict. Use agents to bear the cost of strictness.

This is NOT "start sloppy, tighten later" (the TypeScript approach). Every
triple in Quipu conforms to a schema. Every entity has a canonical name and
type. Every relationship is from the declared vocabulary. The difference from
traditional RDF is that **agents** — not humans — do the work of:

1. **Schema inference** — proposing new classes and properties based on data
2. **Entity resolution** — matching incoming mentions to canonical entities
3. **Relationship typing** — mapping natural-language relations to ontology predicates
4. **SHACL validation** — running shapes against proposed writes, rejecting violations
5. **Ontology evolution** — proposing schema changes when data doesn't fit

The store rejects invalid writes. The agent's job is to make writes valid.

## What "AI-Native" Means for Quipu

Agents are not just API consumers — they are first-class participants:

| Traditional Store | Quipu |
|------------------|-------|
| Human writes schema in Protégé | Agent proposes schema from data patterns |
| Human maps data to ontology | Agent resolves entities + types at ingest |
| Validation rejects bad writes silently | Validation returns structured feedback: "entity X matches existing Y with 0.87 similarity — merge?" |
| Schema evolution is a migration | Schema evolution is a PR-like proposal agents can review |
| Search is SPARQL or nothing | Search is SPARQL + vector + hybrid, all first-class |

### The Proposer-Critic Pattern

Every write goes through an agent-driven pipeline:

```text
Data arrives → Proposer agent extracts triples
             → Critic agent validates against schema (SHACL)
             → If invalid: structured feedback → Proposer retries
             → If valid: write committed with provenance
             → If schema gap: schema evolution proposal created
```

In Gas Town, crew agents ARE both proposer and critic. The store provides
the validation machinery; the agent provides the intelligence.

## Killer Features

What makes Quipu different from every existing knowledge graph tool:

### 1. Agents Enforce Strict Schema (Not Humans)

Traditional RDF stores demand human ontology engineers. AI-native stores
have no schema at all. Quipu enforces strict OWL/SHACL schemas, but
**agents bear the cost** of schema compliance — entity resolution,
relationship typing, validation, and schema evolution are all agent-driven.
The store rejects invalid writes; the agent's job is to make writes valid.

### 2. Immutable Bitemporal Fact Log

Every fact is an immutable entry with two time axes: **transaction time**
(when asserted) and **valid time** (when true in the world). Nothing is
ever deleted, only superseded. This enables:

- **Time-travel queries**: "What did we believe about X on March 15?"
- **Speculative transactions**: Fork in memory, apply hypothetical writes,
  query without persisting
- **Contradiction detection**: Automatic — overlapping valid-time intervals
  on the same entity+attribute
- **Complete audit trail**: Every fact traces to who asserted it and when

### 3. Incremental Materialization with Provenance

OWL/Datalog reasoning produces derived triples. When base facts change,
only affected derivations are recomputed (not full re-reasoning). Every
derived fact tracks its provenance: which rule and which input facts
created it. "Why do we believe X?" is always answerable.

### 4. Native Hybrid Search (SPARQL + Vector)

SPARQL 1.1 for structured queries + LanceDB for semantic similarity,
unified in a single query engine. Temporal filters are native to both:
`WHERE valid_to IS NULL` is the default. No bolt-on vector search —
it's a first-class query primitive.

### 5. Agent-Friendly Validation Feedback

Validation doesn't just reject. It returns:

- Which SHACL shape was violated and why
- The closest matching existing entity (with similarity score)
- Suggested corrections ("did you mean entity X?")
- Whether a schema evolution proposal is appropriate
- Structured JSON that agents can act on programmatically

### 6. Embeddable with "SQLite Energy"

Single process, no server required. Link it into your Rust binary.
SQLite for the fact log, LanceDB for vectors, both in-process. Inspect
your knowledge graph with `sqlite3 quipu.db`. Back it up with `cp`.

### 7. Episode Provenance

Every fact traces back to an episode (a document, observation, extraction
session). Episodes are first-class graph citizens. The chain from raw
observation → extracted fact → derived inference is a graph traversal.

### 8. Type-Hierarchy-Aware Queries

Query at an abstract level (`?x a :Resource`), results span all subtypes.
Combined with an explanation engine: "why does this entity match?" tracks
which triple patterns matched which bindings. Agents get introspectable
reasoning, not opaque result sets.

### 9. Graph Projection for Algorithms

Materialize subgraphs into contiguous CSR structures for graph algorithms
(PageRank, community detection, shortest path, link prediction) without
a graph database server. Results write back as triples.

### 10. Virtual Graph Federation

Query external data sources (databases, APIs, other stores) as virtual
RDF graphs via SPARQL. Quipu becomes the federation layer that unifies
heterogeneous data under one query language.

---

## Design Principles

### 1. Schema-First, Always

Every triple conforms to a declared ontology. No schemaless mode.
RDFS/OWL for class hierarchy, SHACL for shape constraints.

### 2. Temporal-Native

Every fact has `valid_from` and `valid_until`. No fact is permanent.
History is queryable. RDF-star reification stores metadata on triples
(who asserted it, when, from what episode, confidence).

### 3. Episode Provenance

Every fact traces back to an episode (a document, observation, extraction).
Episodes are first-class graph citizens with their own metadata.
"Why do we believe X?" is always answerable.

### 4. Embeddable

Single process, no server required. Link it into your Rust binary.
Optional server mode for multi-process access. "SQLite energy."

### 5. Standard Query Interface

SPARQL 1.1 for structured queries. Not a property-graph query language.
SPARQL because: W3C standard, massive tool ecosystem, federated query
support, decades of optimization research.

### 6. Native Vector Search

Embeddings stored alongside triples. Hybrid search (SPARQL + vector
similarity) in a single query. Not a bolt-on.

### 7. Agent-Friendly Feedback

Validation doesn't just say "rejected." It returns:

- Which SHACL shape was violated
- The closest matching entity (with similarity score)
- Suggested corrections
- Whether a schema evolution might be appropriate

## Architecture (Conceptual)

```text
┌──────────────────────────────────────────────────────────────┐
│                     Bobbin + Quipu                            │
│                                                              │
│  ┌─────────────────────┐  ┌────────────────────────────┐    │
│  │   Bobbin (existing) │  │      Quipu (new module)    │    │
│  │                     │  │                            │    │
│  │  Code search        │  │  Knowledge graph           │    │
│  │  Context assembly   │  │  Ontology enforcement      │    │
│  │  Tag effects        │  │  Temporal facts            │    │
│  │  Feedback loops     │  │  Episode provenance        │    │
│  └─────────┬───────────┘  └──────────┬─────────────────┘    │
│            │                         │                       │
│  ┌─────────┴─────────────────────────┴─────────────────┐    │
│  │              Shared Infrastructure                   │    │
│  │                                                      │    │
│  │  ONNX Embeddings (all-MiniLM-L6-v2, 384-dim)       │    │
│  │  LanceDB (vectors + filtered search)                │    │
│  │  SQLite (metadata, triples, FTS)                    │    │
│  │  MCP Server + REST API + CLI                        │    │
│  └──────────────────────────────────────────────────────┘    │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                 Quipu Internals                       │   │
│  │                                                      │   │
│  │  Triple Store ─── SQLite (SPO/POS/OSP indexes)      │   │
│  │  RDF Model ────── oxrdf + oxttl (RDF-star support)  │   │
│  │  SPARQL ────────── spargebra parser + evaluator      │   │
│  │  Validation ───── rudof (SHACL/ShEx)                │   │
│  │  Ontology ──────── horned-owl (OWL parsing + RDFS)  │   │
│  │  Temporal ──────── RDF-star reification on facts     │   │
│  │  Provenance ───── Episode nodes in graph             │   │
│  └──────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

## Landscape Position

### Why Not Existing Tools?

| Tool | Why Not |
|------|---------|
| **Jena/Fuseki** | Java, heavy, no vector search, no agent integration, server-only |
| **Oxigraph** | Closest fit — Rust, embeddable, SPARQL — but no OWL reasoning, no SHACL, no vector search. Building blocks, not a complete solution. |
| **Stardog** | Proprietary, expensive, server-only |
| **GraphDB** | Proprietary, Java, server-only |
| **RDFox** | Proprietary, expensive, in-memory only |
| **Blazegraph** | Dead (archived 2023) |
| **TypeDB** | Strongest type system, but no RDF/SPARQL, no vector search, MPL license |
| **Graphiti** | AI-native but no schema enforcement, name collisions, Cypher not SPARQL, vendor-locked to FalkorDB/Neo4j |
| **Cognee** | OWL validation but advisory (tags invalid, doesn't reject), Python |
| **TerminusDB** | Schema-first, RDF heritage, but no vector search, no agent integration, WOQL not SPARQL |
| **Kuzu** | Embeddable, vector search, MIT — but property graph, RDF support paused |

### What Quipu Would Be

The tool that doesn't exist yet: **Oxigraph's embeddability + RDFox's reasoning +
Stardog's AI features + TypeDB's strictness** — open source, MIT, lightweight.

## Rust Ecosystem Building Blocks

If built in Rust, significant infrastructure already exists:

| Component | Crate | Maturity |
|-----------|-------|----------|
| RDF data model | `oxrdf` | Stable, well-maintained |
| RDF parsing | `oxttl`, `oxrdfio` | Stable, RDF-star support |
| SPARQL parsing | `spargebra` | Stable |
| SPARQL evaluation | Custom (over SQLite fact log) | To build — spargebra AST → SQL |
| OWL parsing | `horned-owl` | Active, 20-40x faster than Java OWL API |
| OWL reasoning | `whelk-rs` (via horned-owl) | Early but functional |
| SHACL validation | `rudof` | Active, MIT, presented at ISWC 2024 |
| Vector search | `lancedb` | Production-proven in Bobbin, Apache 2.0 |
| Fact store | `rusqlite` (EAVT log) | Battle-tested, 40M+ downloads |
| RDF-star | `oxrdf` + `oxttl` | Supported |
| Embeddings | ONNX Runtime | all-MiniLM-L6-v2, 384-dim, proven in Bobbin |
| Graph algorithms | `petgraph` | Mature, on-demand materialization |

The **open-ontologies** project (61 stars) is attempting something similar:
Oxigraph + OWL2-DL tableaux + SHACL in a single Rust binary with MCP
integration. Worth watching, but very early (in-memory only, no temporal
facts, brute-force vector search, single contributor).

---

## Key Decisions

These are the decisions that shape everything else. Ordered by dependency —
each decision constrains the ones below it.

### Decision 1: Build vs. Extend ✅ RESOLVED → Compose + Bobbin integration

**Compose** Rust crates (oxrdf, spargebra, rudof, horned-owl, LanceDB,
rusqlite) orchestrated by a Quipu layer, integrated as a Bobbin subsystem.

Rationale: Each component does what it's best at, pieces are independently
swappable, and Bobbin provides the deployment skeleton (MCP server, REST API,
CLI, ONNX pipeline, LanceDB integration, config patterns).

### Decision 2: Implementation Language ✅ RESOLVED → Rust

The ecosystem alignment is overwhelming — every major building block exists
as a Rust crate, and Bobbin is already Rust. No other viable option.

### Decision 3: Storage Backend ✅ RESOLVED → EAVT Fact Log (SQLite) + LanceDB

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| **Fact store** | SQLite (rusqlite), EAVT append-only log | Immutable, bitemporal, inspectable, Bobbin uses it |
| **Vector search** | LanceDB | Bobbin already has it, SQL-like filtered search, temporal queries built-in |
| **Embeddings** | ONNX Runtime, all-MiniLM-L6-v2 | Bobbin already has it, 384-dim, local, no API calls |

**Storage schema: EAVT fact log (not a mutable triple table)**

```sql
CREATE TABLE facts (
    e INTEGER,          -- entity (dictionary-encoded IRI)
    a INTEGER,          -- attribute (dictionary-encoded IRI)
    v BLOB,             -- value
    tx INTEGER,         -- transaction ID (monotonic)
    valid_from TEXT,     -- when fact became true in the world
    valid_to TEXT,       -- when fact stopped being true (NULL = current)
    op INTEGER,          -- 1 = assert, 0 = retract
    PRIMARY KEY (e, a, v, tx)
);
CREATE INDEX idx_eavt ON facts(e, a, v, valid_from);
CREATE INDEX idx_aevt ON facts(a, e, v, valid_from);
CREATE INDEX idx_vaet ON facts(v, a, e, valid_from);
CREATE INDEX idx_tx   ON facts(tx);
```

Current state is a view: `WHERE op = 1 AND valid_to IS NULL`.
Time-travel: `WHERE tx <= ? AND valid_from <= ? AND (valid_to IS NULL OR valid_to > ?)`.

This design gives us contradiction detection, time-travel queries, speculative
transactions, and complete audit trails — all from the storage schema itself.

**Why not a graph database (FalkorDB, Neo4j)?** SQLite with indexed fact log
handles 1-3 hop traversals at <1M facts with negligible performance difference
vs. native graph DBs. We gain embeddability, inspectability, zero operational
overhead, and bitemporal capabilities that no graph DB offers natively. We
lose index-free adjacency (matters at 10M+ with 5+ hop queries) and built-in
graph algorithms (materialized into petgraph when needed).

**Why not RocksDB?** Stays as an upgrade path if we outgrow SQLite's ~50M
fact ceiling. Pluggable storage backend makes this a future option.

**Why LanceDB over arroy?** Bobbin already pays the dependency cost. LanceDB
has built-in SQL-like filtered search — temporal queries like
`WHERE valid_to IS NULL AND entity_type = 'Service'` work natively. Every
LanceDB vector query includes temporal filters by default.

**LanceDB temporal awareness**: Vector entries carry `valid_to` so superseded
entity embeddings are filtered out during search. Default filter:
`WHERE valid_to IS NULL`.

### Decision 4: OWL Reasoning Depth ✅ RESOLVED → RDFS + SHACL, OWL 2 RL later

Start with RDFS (class hierarchy, domain/range) + SHACL validation (shape
constraints on write). Add OWL 2 RL rules incrementally when use cases
demand it. Full OWL 2 DL reasoning is rarely needed and adds enormous
complexity. SHACL shapes are more practical than OWL axioms for validation.

### Decision 5: Query Interface ✅ RESOLVED → SPARQL 1.1 + extensions

SPARQL 1.1 as the standard interface (spargebra already parses it). Add
extension functions for vector similarity (`cairn:similar()`) and temporal
queries (`cairn:validAt()`). SPARQL is more expressive than Cypher for RDF
data and has decades of ecosystem behind it.

### Decision 6: Agent Interface Protocol ✅ RESOLVED → Extend Bobbin's

Bobbin already exposes MCP + REST + CLI. Quipu adds knowledge graph tools
to the same surfaces. Agents interact with one MCP server (Bobbin) that
serves both code context and knowledge graph context.

### Decision 7: License ✅ RESOLVED → MIT

Non-negotiable per the bead description. Aligns with all key dependencies:
Oxigraph (MIT/Apache-2.0), rudof (MIT/Apache-2.0), horned-owl (MIT),
LanceDB (Apache 2.0), rusqlite (MIT).

### Remaining Open Decisions

**A. SPARQL evaluation strategy** ✅ RESOLVED → Custom evaluator over SQLite

The bitemporal fact log decision (see Architectural Requirements below)
forces this: Oxigraph's engine expects a mutable triple store with INSERT/
DELETE semantics. It has no concept of temporal columns, transaction IDs,
or log-structured storage. Building a custom SPARQL evaluator on top of
`spargebra` (parser) + SQLite fact log gives us:

- Native `AS OF` time-travel queries
- Bitemporal filters pushed into SQLite's query optimizer
- No impedance mismatch between storage and query engine
- Fewer dependencies (oxrdf + spargebra only, not full Oxigraph)

The Oxigraph store/engine is no longer needed. We keep `oxrdf` (RDF data
model) and `spargebra` (SPARQL parser) only.

**B. Schema evolution model**: How do agents propose ontology changes?
Options: Terraform-style plan/apply (like open-ontologies), PR-like review
workflow, or immediate-with-audit-trail.

**C. Graph algorithm strategy** ✅ RESOLVED → Hybrid materialization

Day-to-day queries: SPARQL over SQLite (1-3 hop lookups, fast enough).
Discovery/analysis: materialize relevant subgraph into `petgraph` (in-memory
Rust graph library), run algorithms (shortest path, community detection,
connected components), write results back as triples.

```text
SQLite (durable, inspectable, SPARQL)
  ↕ materialize on demand
petgraph (in-memory, graph algorithms)
```

For <1M triples, full graph materialization takes milliseconds. Periodic
community detection can run as a batch job (e.g., cron), results stored as
`cairn:memberOfCommunity` triples in SQLite. This avoids a graph DB server
while preserving graph algorithm capabilities.

**D. Reactive notifications**: SQLite `update_hook` for change-data-capture,
or a simple in-process event bus, or defer until there's a real use case?

---

## What Quipu is NOT

- **Not a Graphiti replacement** (yet) — Graphiti serves Gas Town today; Quipu
  is a longer-term vision for what comes next
- **Not a general-purpose graph database** — it's specifically for knowledge
  graphs with ontological structure
- **Not an LLM framework** — it doesn't call LLMs; agents call it
- **Not a reasoning engine** — it validates and stores; agents reason

## Relationship to Gas Town

If Quipu existed today, it would replace FalkorDB + Graphiti in the ontology
stack. The crew-driven extraction pattern (bead → crew extracts → write to
graph) would remain identical — Quipu would just be a better graph backend
with actual schema enforcement.

The existing `crew-ontology-triple-store.md` design doc describes the
architecture that works with Graphiti today. Quipu is the vision for what
that backend should eventually become.

---

## Current Ontology State (as of 2026-04-04)

The aegis homelab ontology already exists in JSON-LD. This is what Quipu
would formalize with strict schema enforcement.

### Existing Infrastructure

- **251 JSON-LD entity files** across 22 crew member directories
- **Shared `@context`**: `ontology/aegis-context.jsonld` (defines all types + prefixes)
- **Graphiti + FalkorDB** backend: ~55 entity nodes, ~42 edges, 6 episodes
- **15-minute cron ingestion** via `ontology-ingest-trigger.sh`
- **Entity harvest scripts** convert JSON-LD → Graphiti REST calls

### Entity Types Defined

Infrastructure: `LXCContainer`, `ProxmoxNode`, `BareMetalHost`, `SystemdService`,
`WebApplication`, `DatabaseService`, `ReverseProxyRoute`, `ZFSDataset`,
`NetworkSegment`

Agents: `CrewMember`, `Polecat`, `Rig`

Tools: `CLI`, `MCPServer`, `Plugin`, `Skill`, `Formula`

People: `Person`, `FamilyMember`, `GoogleAccount`

Media: `MediaLibrary`, `Movie`, `TVSeries`

Governance: `Directive`, `Observation`, `DecisionRecord`, `DesignDoc`

### Relationship Vocabulary

`runs_on`, `depends_on`, `connects_to`, `managed_by`, `monitors`,
`routes_to`, `owns`, `deployed_on`, `member_of`, `applies_to`,
`reports_to`, `manages`, `was_derived_from`, `was_generated_by`

### Standard Entity Files (per crew member)

| File | Contents |
|------|----------|
| `crew.jsonld` | 14+ crew members with roles, tiers, domains, shield scores |
| `hosts.jsonld` | Proxmox nodes and bare metal hosts |
| `containers.jsonld` | LXC containers with IPs, CT IDs, services |
| `services.jsonld` | Systemd services, web apps, databases, routes |
| `networks.jsonld` | Network segments (container, baremetal, VPN) |
| `tools.jsonld` | CLIs, MCP servers, plugins, skills |
| `rigs.jsonld` | Gas Town rigs with purposes |
| `people.jsonld` | Stiwi, family, accounts, interests |
| `media.jsonld` | Libraries, theme park, watchlists |

### Known Issues

1. **Duplicate nodes** — Graphiti creates per-episode copies (koror appears 4x)
2. **No formal OWL** — uses JSON-LD `@context` but no `.owl` or `.ttl` schema files
3. **No SHACL shapes** — no write-time validation, any triple accepted
4. **No temporal metadata** — facts have no `valid_from`/`valid_until`

Quipu would address all four: SHACL shapes enforce structure, OWL defines
the class hierarchy, entity resolution prevents duplicates, and temporal
metadata makes facts versioned.

### Migration Path: JSON-LD → Quipu

The existing `aegis-context.jsonld` maps cleanly to OWL + SHACL:

```text
JSON-LD @type "LXCContainer" → OWL class :LXCContainer rdfs:subClassOf :Infrastructure
JSON-LD property "runsService" → OWL objectProperty :runsService (domain: :LXCContainer, range: :Service)
Required fields → SHACL sh:minCount 1
Allowed values → SHACL sh:in (...)
```

The entity files themselves are already valid RDF (JSON-LD is an RDF
serialization). Quipu ingests them directly — no format conversion needed,
just schema enforcement added on top.

## Differentiation from open-ontologies

The **open-ontologies** project (MIT, Rust, 61 stars, ~25 days old) is the
closest existing project. It composes Oxigraph + OWL2-DL tableaux + SHACL
in a single binary with MCP integration. Here's where Quipu diverges:

| Dimension | open-ontologies | Quipu |
|-----------|----------------|-------|
| **Storage** | In-memory only (Oxigraph `Store::new()`). Reloads from files each session. | Persistent (SQLite/RocksDB). Knowledge survives restarts. |
| **Temporal facts** | None. Whole-graph snapshots only. | Native. Every fact has `valid_from`/`valid_until`. RDF-star reification for provenance. |
| **Vector search** | Brute-force HashMap, won't scale past ~100K entities. | LanceDB ANN index with filtered search, scales to millions. |
| **Episode provenance** | Lineage events in SQLite (operation audit trail). | First-class episode nodes in the graph. "Why do we believe X?" is a graph traversal. |
| **Agent write path** | 43 MCP tools, but no structured feedback on validation failure. | Validation returns similarity scores, suggested corrections, schema evolution proposals. |
| **Multi-tenancy** | Single default graph. | Named graphs for isolation (per-rig, per-domain). |
| **Integration** | Standalone MCP server. | Embeddable library + MCP + REST. Designed to compose with Bobbin, beads, Gas Town. |
| **Schema evolution** | Terraform-style plan/apply/drift. | Agent-driven: propose → review → merge (like a PR for ontology changes). |

**What we'd learn from them**: Their tableaux reasoner and SHACL validator
are well-built. Their MCP tool surface (43 tools) shows what agents actually
need. Their lifecycle tools (plan/apply/drift/lock) are smart patterns.

**What they lack that we need**: Persistence, temporal facts, scalable vector
search, episode provenance, and — critically — integration with an existing
agent ecosystem (Gas Town, beads, Bobbin).

---

## The Bobbin Connection

Bobbin is a Rust semantic code search engine (also Stiwi's project). It shares
remarkable architectural DNA with what Quipu needs:

### Shared Infrastructure

| Component | Bobbin Today | Quipu Needs | Shared? |
|-----------|-------------|-------------|---------|
| Language | Rust | Rust | ✅ Same |
| Embeddings | ONNX Runtime, all-MiniLM-L6-v2, 384-dim | ONNX Runtime, same or similar model | ✅ Share embedding engine |
| Vector store | LanceDB (LMDB-backed) | arroy (LMDB-backed) or LanceDB | ✅ Could share LanceDB |
| Metadata store | SQLite (rusqlite) | SQLite (rusqlite) | ✅ Same |
| API surface | MCP + REST + CLI | MCP + REST + embedded lib | ✅ Same patterns |
| Tags/classification | Namespace:name tags with effects | RDF classes, SHACL shapes | 🔄 Different model, similar purpose |
| Feedback loop | Rating → lineage → improvement | Validation feedback → schema evolution | 🔄 Similar pattern |
| Config format | TOML | TOML | ✅ Same |

### Integration Possibilities

**Option A: Quipu as a Bobbin subsystem** — Bobbin already indexes code; Quipu
adds knowledge graph capabilities. `bobbin` becomes the unified context engine
for both code and domain knowledge. Quipu is a crate that Bobbin depends on.

```text
bobbin (context engine)
├── code search (existing)
│   ├── LanceDB vectors
│   ├── SQLite metadata
│   └── Tree-sitter parsing
├── knowledge graph (cairn crate)  ← NEW
│   ├── RDF triple store (Oxigraph or custom)
│   ├── SHACL validation (rudof)
│   ├── OWL reasoning (horned-owl)
│   └── Temporal facts (RDF-star)
└── shared infrastructure
    ├── ONNX embeddings
    ├── MCP server
    └── SQLite
```

**Option B: Quipu as a sibling** — separate binary, shared crates. Bobbin
and Quipu run independently but share the embedding engine and can cross-query.
`bobbin search` finds code; `cairn query` finds knowledge. Hooks inject both.

**Option C: Shared core, different faces** — extract common infrastructure
(embeddings, storage, MCP, config) into a shared crate. Both Bobbin and Quipu
are thin layers on top.

**Recommendation**: Start with **Option A** (Quipu as a Bobbin subsystem).
Reasons:

- Bobbin already has the Rust skeleton, ONNX pipeline, MCP server, CLI
- One binary to deploy, one MCP server to configure
- Context injection hooks can blend code context + knowledge graph context
- Agents already trust Bobbin — adding knowledge doesn't change the interface
- If it outgrows Bobbin, extraction to a sibling is straightforward

### What This Changes in the Key Decisions

| Decision | Before (standalone Quipu) | After (Bobbin integration) |
|----------|--------------------------|---------------------------|
| **Language** | Rust (recommended) | Rust (confirmed — Bobbin is Rust) |
| **Storage** | SQLite default | SQLite + LanceDB (inherit Bobbin's stack) |
| **Vector search** | arroy (LMDB) | LanceDB (already proven in Bobbin) |
| **Embeddings** | Choose model | all-MiniLM-L6-v2 via ONNX (inherit Bobbin's) |
| **API surface** | Design from scratch | Extend Bobbin's MCP/REST/CLI pattern |
| **Deployment** | New binary | New capability in existing binary |
| **Config** | New config format | Extend `.bobbin/config.toml` |

---

## Gas Town Integration

Quipu doesn't exist in isolation — it's the knowledge backbone for a
multi-agent system. Here's how it connects to Gas Town's moving parts.

### Beads Integration

Beads (Dolt-backed issue tracker) are the primary source of operational
knowledge. Integration points:

| Event | What Happens | Direction |
|-------|-------------|-----------|
| **Bead created** | Extract entities from title + description, store as episode. Suggest related beads via graph traversal. | Bead → Quipu |
| **Bead hooked** | Query graph for entity context: what services are involved, what past incidents relate, what constraints apply. Inject into agent's context. | Quipu → Agent |
| **Bead closed** | Extract learnings/outcomes as episode. Update entity states (e.g., service now healthy). Auto-invalidate stale facts. | Bead → Quipu |
| **Bead labeled** | Labels map to ontology classes. `ontology-ingested` = episode extracted. Track lifecycle. | Bead ↔ Quipu |
| **Dependency added** | Store as graph edge. Enable impact queries: "if I close this bead, what else is unblocked?" | Bead → Quipu |

### Agent Lifecycle Integration

| Event | What Happens |
|-------|-------------|
| **`gt hook`** | Inject relevant graph context alongside Bobbin code context |
| **`gt prime`** | Load agent's domain knowledge from graph (their past work, expertise areas) |
| **`gt handoff`** | Store handoff context as episode. Next session inherits institutional memory. |
| **`gt mail`** | Parse entity mentions in mail, inject graph context about those entities |
| **Patrol cycles** | Dearing's ontology patrol feeds observations → extraction beads → graph |
| **Polecat dispatch** | Query graph for polecat skills (demonstrated via past bead completions) to route work |

### The Unified Context Pipeline

Today, Bobbin injects code context into agent prompts via hooks. With Quipu
integrated, the same hook pipeline injects **code + knowledge**:

```text
Agent prompt arrives (UserPromptSubmit hook)
  │
  ├─ Bobbin: semantic search over code
  │   └─ "Here are relevant code files..."
  │
  ├─ Quipu: SPARQL + vector search over knowledge graph
  │   └─ "Here are relevant entities, facts, past decisions..."
  │
  └─ Merged context injection (budget-constrained)
      └─ Agent sees unified context: code + knowledge
```

The agent doesn't care where context comes from. They just get richer,
more relevant context — and the knowledge graph makes the code context
better (because it knows which services the code touches) and vice versa.

### Attribution & Provenance

Gas Town already has universal actor attribution (`BD_ACTOR` format:
`rig/role/name`). Quipu inherits this:

- Every episode records which agent extracted it
- Every fact traces to its source episode
- Every schema change records who proposed it
- Graph queries can answer: "what does braino know that dearing doesn't?"

This closes the loop: agents build the knowledge graph, the knowledge graph
makes agents more effective, and everything is auditable.

---

## Architectural Requirements from Competitor Analysis

Features worth stealing from competitors, organized by when they must be
designed into the architecture.

### Must Architect From Day One

These features are load-bearing structural decisions. Retrofitting them
later is a near-rewrite.

**1. Immutable Bitemporal Fact Log** (stolen from: Datomic, TerminusDB)

Every fact is an immutable entry in an append-only log with two time axes:

- **Transaction time** (`tx_time`): when the fact was asserted
- **Valid time** (`valid_from`, `valid_to`): when the fact was true in the world

SQLite schema: `(entity, attribute, value, tx_id, valid_from, valid_to, op)`

This enables:

- **Time-travel queries**: "What did we believe about koror on March 15?"
- **Speculative transactions**: Fork in memory, apply hypothetical writes,
  query without persisting — "What breaks if we decommission this host?"
- **Contradiction detection** (Graphiti's killer feature): Query for
  overlapping valid-time intervals on the same entity+attribute. Falls out
  for free from bitemporal design.
- **Complete audit trail**: Nothing is ever deleted, only superseded.
- **Git-for-data** (TerminusDB): Branch, merge, diff on the graph itself
  using delta encoding over the fact log.

**The critical decision**: Is the log the primary storage (Datomic-style,
EAVT indexes over the log) or a changelog on mutable tables? Log-as-source
shapes every index, every query path, every write operation. Must decide
before writing a single line of storage code.

**2. Incremental Materialization with Provenance** (stolen from: RDFox)

OWL/Datalog reasoning produces derived triples. When base facts change,
only affected derivations are recomputed (RDFox achieves 2-3M inferences/sec).
Without this, every new triple triggers full re-reasoning — unusable at scale.

Requires a **dependency graph** between derived facts and the base facts +
rules that produced them:

- Each derived triple stores provenance: which rule + which inputs created it
- On deletion: walk dependency graph, remove unsupported derivations
- On addition: run only rules whose body patterns match new facts

This must be in the storage layer from the start. The `derived_facts` table
needs a `provenance` column linking to `(rule_id, input_fact_ids[])`.

**3. Graph Projection Interface** (stolen from: Neo4j GDS)

A `ProjectionBuilder` trait that materializes a subgraph into a contiguous
CSR (compressed sparse row) structure suitable for graph algorithms:

```rust
trait ProjectionBuilder {
    fn project(&self, query: &str) -> Projection;  // SPARQL → in-memory graph
}

trait Projection {
    fn page_rank(&self, config: PageRankConfig) -> HashMap<NodeId, f64>;
    fn louvain(&self, config: LouvainConfig) -> HashMap<NodeId, CommunityId>;
    fn shortest_path(&self, from: NodeId, to: NodeId) -> Vec<NodeId>;
    fn link_predict(&self, from: NodeId, to: NodeId) -> f64;
    // ...
}
```

Design the trait now; implement algorithms incrementally via `petgraph` or
custom CSR. Community detection results write back as triples:
`?entity cairn:memberOfCommunity ?community`.

**4. Type-Hierarchy-Aware Query Planner** (stolen from: TypeDB)

Query at an abstract type level, results span all subtypes:
`?x a :Resource` → returns documents, services, containers — all subtypes.

The query planner expands abstract types using the OWL class hierarchy and
optimizes across them. Combined with an **explanation engine**: tracking
which triple patterns matched which bindings during execution, so agents
can ask "why does this entity match this query?"

### Architect the Interface Now, Implement Later

**5. Virtual Graphs / Query Federation** (stolen from: Stardog)

A `VirtualGraphProvider` trait that maps external data sources (Dolt beads,
Bobbin indexes, REST APIs) as virtual RDF graphs queryable via SPARQL:

```rust
trait VirtualGraphProvider {
    fn resolve(&self, pattern: &TriplePattern) -> Vec<Triple>;
    fn cost_estimate(&self, pattern: &TriplePattern) -> Cost;
}
```

The query planner needs to know about federated sources from the start
(cost model, filter pushdown). Actual connectors (beads, Bobbin, etc.)
are additive.

**6. Reactive Subscriptions** (stolen from: SurrealDB)

Subscribe to changes matching a graph pattern, get push notifications:
`LIVE SELECT ?s WHERE { ?s :hasState "down" }` → notified when any entity
goes down. An event bus on the write path. Interface design now, implementation
after the write path stabilizes.

### Can Add Later (no architectural impact)

| Feature | Stolen From | Notes |
|---------|-------------|-------|
| Graph algorithms (PageRank, Louvain, etc.) | Neo4j GDS | Pure functions over Projection API |
| Speculative transactions | Datomic | In-memory overlay on fact log (~200 lines if bitemporal is in place) |
| Poincaré embeddings for hierarchy | open-ontologies | Just another distance metric in LanceDB |
| Natural language → SPARQL | Stardog Voicebox | Agent-side concern, not storage |
| Columnar property storage | Kuzu | LanceDB is already columnar/Arrow-based |
| Design pattern enforcement | open-ontologies | Application-layer rules over SHACL |

### Summary: The Four Early Bets

| Bet | Difficulty | AI Value | Bolt-on Later? |
|-----|-----------|----------|----------------|
| Immutable bitemporal fact log | Medium | Critical | **No** |
| Incremental materialization + provenance | Hard | Critical | **No** |
| Graph projection interface (CSR) | Medium | High | Partially |
| Type-hierarchy query planner | Medium | High | Partially |

Get the first two right and Quipu has a foundation that no competitor
fully combines in one embeddable package.

---

## Next Steps

1. **Validate decisions** — review this doc, challenge assumptions
2. **Design the fact log schema** — the most consequential early decision.
   Immutable append-only EAVT with bitemporal columns in SQLite. Prototype
   the schema, write path, and time-travel query pattern.
3. **Prototype in Bobbin** — add a `cairn` module with basic triple store
   - SHACL validation (rudof). Prove they compose.
4. **Design incremental materialization** — provenance-tracked derived
   triples with dependency graph for efficient re-reasoning.
5. **Design the write path** — schema-validated ingest with agent feedback
   (SHACL violations return structured suggestions, not just rejections).
6. **Design schema evolution** — how agents propose and review ontology changes
7. **Define MCP tool surface** — what tools do agents need? Learn from
   open-ontologies' 43 tools and Bobbin's existing patterns.
8. **Port existing ontology** — migrate the `aegis-context.jsonld` taxonomy
   to OWL + SHACL as the first real Quipu ontology.
9. **Benchmark** — compare against Graphiti+FalkorDB on Gas Town workloads
