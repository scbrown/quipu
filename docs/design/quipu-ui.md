# Quipu UI: Knowledge Graph Visualization & Exploration

> Quipu owns its visual identity. Bobbin enables it — doesn't reimagine it.

## Status

- **Author**: crew/ellie (Dearing)
- **Date**: 2026-04-05
- **Status**: Draft — design exploration
- **Beads**: qp-w2p (Quipu standalone UI), bobbin-fal (Bobbin integration)

## The Problem with micro-ui.md

The current Bobbin micro-ui.md design has Bobbin reimplementing Quipu's visual
surface: entity cards, edge lists, graph widgets, SPARQL workbench — all owned
by Bobbin through a `KnowledgeViewSet` trait that renders HTML fragments. This
is wrong for three reasons:

1. **Bobbin becomes a knowledge graph UI toolkit.** That's not its job. Bobbin
   indexes code and assembles context. It shouldn't know what a SHACL validation
   badge looks like or how to render a temporal sparkline.

2. **Quipu loses its visual identity.** If every UI element is a Bobbin Askama
   template, Quipu can't evolve its visualization independently. New entity
   types, new temporal views, new schema inspectors — all require Bobbin changes.

3. **Tight coupling at the wrong layer.** `ViewEntity`, `ViewEdge`,
   `render_entity_card()` — this is CRUD-level integration. Bobbin is reaching
   into Quipu's data model to render it. That's not delegation, it's absorption.

## The Right Separation

```text
┌─────────────────────────────────────────────────────┐
│                    User's Browser                    │
│                                                     │
│  ┌──────────────┐         ┌──────────────────────┐  │
│  │ Bobbin UI    │         │ Quipu UI             │  │
│  │              │  embed  │                      │  │
│  │  code search ├────────►│  graph explorer      │  │
│  │  bundles     │  link   │  SPARQL workbench    │  │
│  │  context     ├────────►│  schema browser      │  │
│  │              │         │  temporal navigator   │  │
│  │  [knowledge] │ iframe/ │  episode timeline    │  │
│  │  [  panel  ] │ widget  │                      │  │
│  └──────┬───────┘         └──────────┬───────────┘  │
│         │                            │              │
└─────────┼────────────────────────────┼──────────────┘
          │ REST/MCP                   │ REST/SPARQL
          ▼                            ▼
    ┌───────────┐              ┌──────────────┐
    │ Bobbin    │   crate dep  │ Quipu        │
    │ Server    │◄────────────►│ Server       │
    │ (code)    │              │ (knowledge)  │
    └───────────┘              └──────────────┘
```

**Bobbin's role**: "I have a knowledge panel. Quipu, fill it."

**Quipu's role**: "I own graph visualization, SPARQL, schema, temporal views.
I render them myself — as a standalone app, or as embeddable widgets."

## Quipu UI Architecture

### Why Rust WASM

Quipu is a Rust library. Its UI should be too:

- **Single language** — Entity types, SPARQL AST, SHACL shapes are all Rust
  structs. No serialization boundary to a TypeScript frontend.
- **Shared logic** — SPARQL parsing, validation, temporal queries run in the
  browser via WASM. The UI can validate queries client-side, syntax-highlight
  SPARQL with the real parser, preview SHACL shapes without a server round-trip.
- **Embeddable** — A WASM module can be loaded anywhere: standalone page,
  Bobbin iframe, VS Code webview, Tauri desktop app.

### Framework: Leptos

Leptos over Dioxus because:

- Fine-grained reactivity (signals, not virtual DOM diffing)
- First-class SSR for the standalone app (progressive enhancement)
- Mature ecosystem, stable API
- `leptos_router` for deep-linkable views
- Islands architecture for partial hydration (critical for embedding)

Leptos provides the application shell. Graph rendering uses JS interop with
a purpose-built graph library (see below).

### Graph Rendering: Sigma.js + Graphology

Not Cytoscape.js. Here's why:

| Criterion           | Sigma.js + Graphology      | Cytoscape.js              |
|---------------------|----------------------------|---------------------------|
| Rendering           | WebGL (fast)               | Canvas (adequate)         |
| Large graphs        | 50K+ nodes                 | ~5K nodes                 |
| Data model          | Graphology (rich, separate) | Built-in (coupled)       |
| Algorithms          | ForceAtlas2, Louvain, etc. | BFS, Dijkstra, etc.      |
| Ecosystem           | Gephi Lite built on it     | Widely used               |
| Customization       | Reducers (functional)      | Stylesheets (CSS-like)   |
| License             | MIT                        | MIT                       |

Sigma handles rendering. Graphology handles the data model and algorithms.
Quipu's Leptos app communicates with them via `wasm-bindgen` interop — Rust
signals drive graph state, JS renders it.

### The Interop Pattern

```rust
// Rust (Leptos) owns the data
let (graph_data, set_graph_data) = create_signal(GraphState::default());

// When data changes, push to JS graph renderer
create_effect(move |_| {
    let data = graph_data.get();
    // Call into JS: sigma_instance.updateGraph(data)
    update_sigma_graph(&data);
});

// JS events (node click, hover) call back into Rust
#[wasm_bindgen]
pub fn on_node_click(node_id: &str) {
    // Navigate to entity detail, update signals, etc.
}
```

This keeps Rust in control of state and navigation while JS handles WebGL
rendering. The boundary is narrow: graph data flows down, user events flow up.

## Standalone UI: Views

Quipu serves its own web UI at its REST API port. Five core views, each a
Leptos component that can also be embedded as an island.

Every view is built on the semantic web patterns from the previous section:

- **Every entity URL supports content negotiation** — the standalone UI serves
  the HTML representation; the same URL serves JSON-LD or Turtle for machines.
  The UI IS the linked data browser.
- **Every page emits JSON-LD** — even the HTML view includes a
  `<script type="application/ld+json">` block, so crawlers and embedding
  hosts (Bobbin) can extract structured data from any Quipu page.
- **Statement groups** structure all entity detail views — not flat property
  tables, but grouped-by-predicate with every value a navigable link.
- **The spotlight API** powers the search bar across all views — type text,
  get entity annotations with confidence scores, navigate directly.
- **TPF endpoint** backs the graph explorer — Sigma.js fetches triple patterns
  incrementally as the user expands neighborhoods, rather than loading the
  entire graph upfront.

### 1. Graph Explorer

The primary view. A force-directed graph of entities and relationships.

**Layout**: Full-viewport graph with a collapsible sidebar.

**Interactions**:

- **Search to focus**: Type a name or IRI → graph centers on that node,
  highlights its neighborhood
- **Click node**: Sidebar shows entity detail as **statement groups** (Wikidata
  pattern): relationships grouped by predicate, each value a clickable link.
  The sidebar URL is the entity's content-negotiated URL — copy it, and
  machines get JSON-LD, browsers get this same view.
- **Double-click node**: Expand its 1-hop neighborhood (fetch more edges)
- **Right-click node**: Radial context menu (Reagraph pattern):
  - "Show in SPARQL" → pre-fills workbench with a query for this entity
  - "Validate" → run SHACL validation, show inline results
  - "History" → open temporal view for this entity
  - "Hide" → remove from current view (not from data)
- **Semantic zoom** (Ogma pattern): Zoom out → nodes become type-colored dots
  with labels. Zoom in → nodes expand to show property summaries.
- **Lasso select**: Select multiple nodes → bulk operations (hide, export,
  run aggregation query)

**Filtering**:

- Entity type checkboxes (color-coded)
- Predicate type checkboxes (show/hide edge types)
- Date range slider (valid-time filter — bitemporal!)
- Text filter (match node labels)

**Graph data source**: Triple Pattern Fragments endpoint for incremental
loading. The initial view fetches a "home" subgraph (all hosts + services +
top-level dependencies) via a seed SPARQL query. Neighborhood expansion
(double-click) fetches additional triples via TPF — one pattern per hop,
client-side join via Graphology. This keeps the graph explorer responsive
even against a large knowledge base: only visible neighborhoods are loaded.

For complex cross-cutting queries (e.g., "everything that depends on koror
transitively"), the SPARQL workbench is the right tool — the graph explorer
is for browsing, not querying.

### 2. SPARQL Workbench

Split-pane: editor on top, results on bottom.

**Editor**:

- Monaco editor (via `monaco-wasm` or CDN) with SPARQL syntax highlighting
- **Schema-aware autocomplete**: Quipu knows its loaded SHACL shapes. When you
  type `?x a`, suggest entity types from the shapes. When you type `?x aegis:`,
  suggest predicates. This is the killer feature — no other SPARQL workbench
  does schema-driven completion from SHACL.
- **Inline validation**: Parse SPARQL client-side using Quipu's own `spargebra`
  parser compiled to WASM. Red squiggles on syntax errors before you hit Run.
- **Query templates**: Pre-loaded examples from the mdbook tutorials, organized
  by persona (operator, agent builder, archaeologist, gardener).
- **Natural language bar** (future): "What depends on koror?" → generates SPARQL.

**Results**:

- **Table view**: Default. Sortable columns, clickable IRIs that navigate to
  entity detail.
- **Graph view**: If the result contains triples (CONSTRUCT or projected
  subject/predicate/object), render as an inline graph widget.
- **JSON view**: Raw result bindings for debugging.
- **Timeline view**: If results contain temporal data (`valid_from`, `valid_to`),
  offer a timeline rendering.

**History**: Query history stored in localStorage. Named saves. Shareable
query URLs (query encoded in URL hash).

### 3. Temporal Navigator

The bitemporal differentiator. No other tool does this.

**Dual-axis time control**:

```text
Transaction Time (what did we know when?)
  ◄━━━━━━━━━━━━━━━━━━━━━━━●━━━━━━━━━━━━►
  2026-01        2026-03       NOW

Valid Time (when was this true?)
  ◄━━━━━━━━━━━━━━━━●━━━━━━━━━━━━━━━━━━━━►
  2026-01      2026-02          2026-04
```

Scrubbing either slider re-queries Quipu with `AS OF` (transaction time) or
`validAt` (valid time) and updates the graph explorer.

**Entity History Panel**:
For a selected entity, show every assertion and retraction:

```text
koror (ProxmoxNode)
──────────────────────────────────────────────────
 2026-03-27  ✗ status: "online" retracted
 2026-03-27  ✓ status: "down" asserted
             ↳ episode: koror-p0-recovery
 2026-03-29  ✗ status: "down" retracted
 2026-03-29  ✓ status: "online" asserted
             ↳ episode: koror-recovery-complete
 2026-04-01  ✓ depends_on: dns.lan asserted
             ↳ episode: dns-migration
```

Each row links to the episode that caused the change. Both time dimensions
are shown (when was this true? when did we learn it?).

**Graph Diff**:
Pick two time points → see what changed:

- Green nodes/edges: Added
- Red nodes/edges: Removed
- Yellow nodes: Properties changed (expandable to see what changed)

This is invaluable for incident review: "What changed in the graph between
2pm and 3pm on the day of the outage?"

### 4. Schema Browser

Visual browser for loaded SHACL shapes and the entity type hierarchy.

**Tree View** (Protege-inspired):

```text
aegis:Thing
├── aegis:InfrastructureEntity
│   ├── aegis:ProxmoxNode (3 instances)
│   ├── aegis:LXCContainer (12 instances)
│   └── aegis:SystemdService (18 instances)
│       ├── aegis:WebApplication (7)
│       └── aegis:DatabaseService (4)
├── aegis:AgentEntity
│   ├── aegis:CrewMember (6)
│   └── aegis:Rig (3)
└── aegis:KnowledgeEntity
    ├── aegis:Directive (15)
    └── aegis:DesignDoc (8)
```

Click a type → see its SHACL shape as a card:

```text
┌─ aegis:LXCContainer ───────────────────────────┐
│                                                 │
│  Properties:                                    │
│    ctId        xsd:integer   required  (1..1)   │
│    hostname    xsd:string    required  (1..1)   │
│    ipAddress   xsd:string    optional  (0..1)   │
│    runsService →Service      optional  (0..*)   │
│    runningOn   →ProxmoxNode  required  (1..1)   │
│                                                 │
│  Instances: 12    Last validated: 2h ago   ✓    │
│  [View instances]  [Run validation]             │
└─────────────────────────────────────────────────┘
```

**Visual Schema View** (WebVOWL-inspired):
The VOWL notation — circles for classes, rectangles for datatypes, arrows for
properties — rendered as a Sigma.js graph. This gives a bird's-eye view of the
ontology structure. Not the instance data — the schema itself.

**Validation Report**:
Run SHACL validation across all entities. Results grouped by shape, severity:

```text
aegis:LXCContainer — 12 instances — 2 warnings
  ⚠ ct-205 missing ipAddress (optional but recommended)
  ⚠ ct-310 runsService links to non-existent entity

aegis:SystemdService — 18 instances — 1 error
  ✗ graphiti-server: healthScore 150 exceeds max (100)
```

Click any violation → navigate to the entity in the graph explorer.

### 5. Episode Timeline

Chronological view of ingested episodes — the "how did we learn this?" view.

```text
2026-04-04
  ┣━ 14:30  Incident: koror NFS timeout cascade
  ┃         extracted: 3 entities, 5 edges
  ┃         source: hq-3wmc (handoff mail)
  ┃
  ┣━ 16:00  Directive: media ownership → grant
  ┃         extracted: 2 entities, 1 edge
  ┃         source: IRC #aegis
  ┃
  ┗━ 22:00  Observation: traefik migration partial
             extracted: 4 entities, 3 edges
             source: crew/dearing patrol

2026-04-03
  ┣━ ...
```

Click an episode → expand to see:

- Raw episode text (the input)
- Extracted entities and edges (the output)
- Mini-graph of what was added to the knowledge graph
- Links to the source (bead, mail, IRC message)

Filter by source type, date range, entity type affected.

## Bobbin Integration: The Embed Contract

Bobbin doesn't reimplement Quipu's UI. It embeds it. Three integration levels,
from simplest to richest:

### Level 1: Links (zero effort)

Bobbin search results that match knowledge entities get a badge:

```text
search_result.rs:42  — HybridSearch trait definition
  📊 koror (ProxmoxNode)  →  http://quipu.svc:3030/entity/aegis:koror
```

The badge is a hyperlink. Clicking it opens Quipu's standalone UI. Bobbin
doesn't render anything — it just knows "this code relates to this entity"
and provides a link.

**Implementation**: Bobbin already has entity IRI data from the Quipu crate
dependency. Generating a link is trivial.

### Level 2: Embedded Widgets (iframe/web component)

Bobbin's UI includes a "Knowledge" panel that loads Quipu's UI in an iframe
or web component. Quipu renders itself — Bobbin just provides the viewport.

```text
┌─ Bobbin ──────────────────────────────────────┐
│ Search: "dns"                                 │
│                                               │
│ ┌─ Code Results ────────────────────────────┐ │
│ │ dns.tf:12 — resource "incus_instance"     │ │
│ │ traefik.toml:45 — [entrypoints.dns]       │ │
│ └───────────────────────────────────────────┘ │
│                                               │
│ ┌─ Knowledge (powered by Quipu) ────────────┐ │
│ │ ┌─────────────────────────────────────────┐│ │
│ │ │  << Quipu renders this entire area >>   ││ │
│ │ │  Graph: dns.lan → AdGuard → koror       ││ │
│ │ │  Entities: 3 | Edges: 5 | Valid ✓       ││ │
│ │ └─────────────────────────────────────────┘│ │
│ └───────────────────────────────────────────┘ │
└───────────────────────────────────────────────┘
```

**Communication**: `postMessage` between Bobbin (host) and Quipu (iframe).
Bobbin sends: `{ action: "show", query: "dns", context: "search" }`.
Quipu renders the appropriate view and sends back: `{ entities: [...] }` if
Bobbin needs to annotate its own results.

**Web Component alternative**: Quipu exports `<quipu-graph>`, `<quipu-entity>`,
`<quipu-search>` custom elements. Bobbin drops them into its HTML:

```html
<quipu-graph
  endpoint="http://quipu.svc:3030"
  query="SELECT ?s ?p ?o WHERE { ?s ?p ?o . ?s a aegis:SystemdService }"
  height="400px">
</quipu-graph>
```

The web component loads Quipu's WASM module and Sigma.js, renders itself.
Bobbin's only job is placing the element and passing attributes.

### Level 3: Shared Context Assembly (the deep integration)

When Bobbin assembles context for an AI agent, it can include Quipu knowledge.
This isn't a UI concern — it's the crate-level integration that already exists.
But the UI can *visualize* it:

Bobbin shows "Context Preview" → includes a section "Knowledge context from
Quipu" → rendered by Quipu's embedded widget showing which entities and
relationships were injected into the agent's context window.

This is read-only. Bobbin asks Quipu "what's relevant to this query?" via the
crate API. Quipu returns structured data. Bobbin says "here's what Quipu
contributed" and renders it with a Quipu widget.

### What Bobbin Does NOT Do

- Does NOT define `ViewEntity`, `ViewEdge`, or any knowledge data types
- Does NOT implement `render_entity_card()` or any rendering trait
- Does NOT serve `/kb/*` routes with its own templates
- Does NOT import Askama templates for knowledge views
- Does NOT own the graph visualization library choice

Bobbin's knowledge integration is:

1. **Data**: `quipu::Store` crate API for context assembly (already exists)
2. **UI**: `<quipu-*>` web components or iframe for visual embedding
3. **Navigation**: Hyperlinks from code results to Quipu's standalone UI

## Tech Stack Summary

### Quipu Standalone UI

| Layer           | Choice                     | Rationale                              |
|-----------------|----------------------------|----------------------------------------|
| App framework   | Leptos (Rust WASM)         | Shared types, client-side SPARQL parse |
| Graph rendering | Sigma.js + Graphology      | WebGL perf, Gephi Lite lineage         |
| SPARQL editor   | Monaco (CDN) or CodeMirror | Syntax highlighting, autocomplete      |
| Styling         | Tailwind (CDN) or vanilla  | Dark mode, utility-first               |
| Embedding       | Web Components API         | Framework-agnostic embedding           |
| Build           | Trunk (Rust WASM bundler)  | `trunk serve` for dev, `trunk build`   |
| Interop         | wasm-bindgen               | Rust ↔ JS bridge for Sigma.js          |

### Bobbin Integration

| Concern         | Approach                   | Rationale                              |
|-----------------|----------------------------|----------------------------------------|
| Knowledge panel | `<quipu-graph>` web comp   | Quipu owns rendering                   |
| Entity links    | Hyperlinks to quipu.svc    | Zero coupling                          |
| Context viz     | `<quipu-context>` web comp | Quipu shows what it contributed         |
| Data flow       | quipu crate API            | Already exists, compile-time checked    |

## Embeddable Widget Catalog

Quipu exports these web components (each self-contained WASM + JS):

### `<quipu-graph>`

Interactive graph explorer. Attributes:

- `endpoint` — Quipu REST API URL
- `query` — SPARQL query to populate initial graph (optional)
- `focus` — IRI to center on (optional)
- `depth` — Hop depth from focus node (default: 1)
- `height` — Widget height (default: 400px)
- `types` — Comma-separated entity types to show (filter)

### `<quipu-entity>`

Entity detail card with edges and history. Attributes:

- `endpoint` — Quipu REST API URL
- `iri` — Entity IRI to display
- `show-edges` — Show edge list (default: true)
- `show-history` — Show temporal history (default: false)

### `<quipu-sparql>`

SPARQL workbench. Attributes:

- `endpoint` — Quipu REST API URL
- `query` — Pre-filled query (optional)
- `height` — Widget height

### `<quipu-timeline>`

Episode timeline. Attributes:

- `endpoint` — Quipu REST API URL
- `from` / `to` — Date range filter
- `source-type` — Filter by episode source type

### `<quipu-schema>`

Schema browser showing loaded SHACL shapes. Attributes:

- `endpoint` — Quipu REST API URL
- `shape` — Focus on a specific shape (optional)

## Lessons from the Semantic Web

The semantic web has 25 years of prior art on exactly this problem: how does a
knowledge capability nest inside a larger application? The patterns below are
ranked by relevance to Quipu-in-Bobbin.

### Pattern 1: JSON-LD Annotation (Schema.org / Drupal)

Every page Bobbin renders could emit a `<script type="application/ld+json">`
block with Quipu knowledge about the code being displayed. Zero visual impact,
machine-readable, incrementally adoptable.

```html
<!-- Bobbin search result for parseConfig -->
<div class="search-result">
  <h3>parseConfig</h3>
  <p>go/pkg/config/parse.go:42</p>
</div>

<!-- Knowledge annotation — invisible to humans, consumed by machines -->
<script type="application/ld+json">
{
  "@context": {
    "@vocab": "https://schema.org/",
    "quipu": "https://quipu.dev/ontology#"
  },
  "@type": "SoftwareSourceCode",
  "@id": "https://quipu.svc/entity/parseConfig",
  "name": "parseConfig",
  "programmingLanguage": "Go",
  "quipu:dependsOn": [
    {"@id": "https://quipu.svc/entity/yamlParser"}
  ],
  "quipu:ownedBy": {"@id": "https://quipu.svc/entity/team-platform"}
}
</script>
```

**Why this matters**: Bobbin doesn't need to render knowledge — it just emits
structured data. Browser extensions, CI tools, documentation generators, and
Quipu's own UI can all consume the JSON-LD independently. Drupal has done this
since version 7 with a declarative mapping layer between content types and RDF
vocabularies. We should steal that mapping approach.

**Integration cost**: Near zero. Bobbin already has entity IRI data from the
Quipu crate. Emitting a JSON-LD block is a template addition, not an
architecture change.

### Pattern 2: LDP-Style Entity URLs (Content Negotiation)

Every knowledge entity in Quipu gets its own dereferenceable URL. The same
URL returns different formats based on the `Accept` header:

```text
GET /entity/koror
Accept: text/html           → Quipu's standalone UI page for koror
Accept: application/ld+json → JSON-LD document
Accept: text/turtle         → Turtle RDF
```

With format-specific sub-paths for debugging:

```text
/entity/koror/html
/entity/koror/json
/entity/koror/ttl
```

**Why this matters**: Bobbin links to `quipu.svc/entity/koror`. Browsers get a
rich HTML page. API clients get JSON-LD. SPARQL engines get Turtle. Zero extra
integration code — just HTTP content negotiation doing its job.

This is the Linked Data Platform (LDP) pattern: resource-oriented, not
query-oriented. Instead of Bobbin constructing SPARQL queries, it just fetches
entity URLs. Standard HTTP caching works. Every entity is bookmarkable. The
entire graph is browsable by following links.

**How DBpedia does it**: `curl -H "Accept: text/turtle" http://dbpedia.org/resource/Linux`
returns Turtle RDF. Same URL in a browser returns a human-readable page with
a table of triples, every URI hyperlinked to its own page. Browsable graph.

### Pattern 3: Entity Spotlight (DBpedia Spotlight / OpenRefine)

A Quipu API endpoint that takes unstructured text and returns matched entities:

```text
POST /quipu/spotlight
{"text": "koror NFS mount timeout cascaded to dolt-server", "confidence": 0.5}

→ {
    "annotations": [
      {"surface": "koror", "iri": "aegis:koror", "type": "ProxmoxNode",
       "confidence": 0.98, "offset": 0},
      {"surface": "dolt-server", "iri": "aegis:dolt-server", "type": "SystemdService",
       "confidence": 0.95, "offset": 42}
    ]
  }
```

**Why this matters**: Bobbin passes code comments, commit messages, bead
descriptions, or search queries through this endpoint. Matching terms become
hyperlinks to Quipu entity pages. The knowledge graph surfaces contextually
without Bobbin understanding the ontology.

**OpenRefine's reconciliation API** adds batch matching and autocomplete:

- Reconcile: batch of strings → candidate entity matches with scores
- Preview: HTML card for an entity (embeddable in any host)
- Suggest: typeahead autocomplete for entity names

This is the lightest possible "knowledge in Bobbin" integration: Bobbin sends
text, Quipu returns annotations, Bobbin renders them as links.

### Pattern 4: Statement Groups (Wikidata)

When knowledge appears in Bobbin, group it by relationship type — not as a
flat property table:

```text
parseConfig
  Dependencies ─────────────────────
    yamlParser (Go Module)
    configSchema (SHACL Shape)

  Owned By ─────────────────────────
    team-platform (CrewMember)

  Runs On ──────────────────────────
    config-service → kota (ProxmoxNode)

  Recent Episodes ──────────────────
    2026-04-03: "config parser refactored for YAML 1.2"
```

Every value is a link to another entity. The graph becomes browsable without
ever rendering a force-directed layout. This is how Wikidata presents Q42
(Douglas Adams) — statement groups with qualifiers and provenance.

### Pattern 5: Triple Pattern Fragments (Client-Side Queries)

Instead of Bobbin proxying SPARQL queries to Quipu, Quipu exposes a minimal
Triple Pattern Fragments endpoint:

```text
GET /fragments?predicate=aegis:dependsOn&object=aegis:koror
→ All triples where something depends on koror
  + total count
  + hypermedia links to next page
```

Bobbin's frontend includes Comunica (a JavaScript query engine) that
decomposes complex queries into triple pattern fetches and joins client-side.
The server stays stateless and cacheable. The client does the work.

**Why this matters**: No SPARQL proxy needed. No query planning on the server.
Trivially cacheable at the HTTP layer. The client can do ad-hoc exploration
without Bobbin's backend being involved at all. This is the lightest possible
server-side commitment with full query power on the client.

### Pattern 6: Declarative Knowledge Mapping (Drupal RDF Module)

A config file maps Bobbin's data model to Quipu's ontology:

```toml
# bobbin-quipu-mapping.toml

[mappings.code_symbol]
bobbin_type = "CodeSymbol"
quipu_type = "aegis:SoftwareComponent"
match_by = "name"  # How to reconcile Bobbin symbols with Quipu entities

[mappings.code_module]
bobbin_type = "CodeModule"
quipu_type = "aegis:CodeRepository"
match_by = "path"

[annotations]
# Which Quipu relationships to show on Bobbin search results
show_predicates = ["aegis:dependsOn", "aegis:ownedBy", "aegis:runsOn"]
max_depth = 1
```

Changes to either model only require updating the mapping — not the code.
Drupal has done this since 2011 with `rdf_mapping` YAML configs.

### What This Means for the Architecture

The semantic web patterns reshape the integration levels:

| Level | micro-ui.md (old) | This design (new) |
|-------|-------------------|-------------------|
| L0 | — | JSON-LD blocks in Bobbin pages |
| L1 | Entity badges | Entity spotlight annotations |
| L2 | KnowledgeViewSet trait | `<quipu-*>` web components |
| L3 | /kb/* routes in Bobbin | Content-negotiated entity URLs |
| L4 | — | TPF + Comunica client-side |

The key shift: **Bobbin annotates, Quipu renders.** Bobbin's job is to tell
machines "this code relates to these knowledge entities" (JSON-LD). Quipu's
job is to render those entities beautifully (standalone UI, web components).
The bridge is hyperlinks and web standards, not Rust traits.

## Testing Strategy

### The Problem

UI testing for WASM apps is different from testing a React SPA. The rendering
happens partly in Rust (Leptos signals, state management), partly in JS
(Sigma.js graph, CodeMirror editor), and partly in WebGL (graph rendering).
Polecats (our AI agent workers) need to test this too — they can't eyeball
a graph layout.

### Layer 1: Rust Unit Tests (No Browser)

Test Quipu's UI logic without a browser. Leptos components can be tested
as pure functions that produce HTML strings.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_card_renders_type_badge() {
        let entity = ViewEntity {
            iri: "aegis:koror".into(),
            label: "koror".into(),
            entity_type: "ProxmoxNode".into(),
            ..Default::default()
        };
        let html = render_entity_card(&entity);
        assert!(html.contains("ProxmoxNode"));
        assert!(html.contains("koror"));
    }

    #[test]
    fn sparql_editor_validates_query() {
        let result = validate_sparql("SELECT ?s WHERE { ?s ?p ?o }");
        assert!(result.is_ok());

        let result = validate_sparql("SLECT ?s WHERE");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected SELECT"));
    }

    #[test]
    fn temporal_controls_clamp_to_valid_range() {
        let state = TemporalState::new(/* valid range: 2026-01 to 2026-04 */);
        state.set_valid_at("2025-01-01"); // Before range
        assert_eq!(state.valid_at(), "2026-01-01"); // Clamped
    }
}
```

**What this covers**: State management, data transformations, HTML generation,
query validation, temporal logic. Everything that doesn't need a real browser.

**Polecat-friendly**: Yes — these run with `cargo test`. No browser needed.

### Layer 2: WASM Integration Tests (Headless Browser)

Test the compiled WASM module in a headless browser. Use `wasm-pack test` with
a headless Chrome/Firefox driver.

```rust
// tests/web.rs
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn graph_explorer_loads_entities() {
    // Mount the Leptos app in a test container
    let container = document().create_element("div").unwrap();
    mount_to(container.clone(), || view! { <GraphExplorer /> });

    // Wait for data fetch
    sleep(Duration::from_millis(500)).await;

    // Verify Sigma.js graph was created
    let canvas = container.query_selector("canvas").unwrap();
    assert!(canvas.is_some(), "Sigma.js canvas should be rendered");
}

#[wasm_bindgen_test]
async fn web_component_responds_to_attributes() {
    let el = document().create_element("quipu-entity").unwrap();
    el.set_attribute("iri", "aegis:koror").unwrap();
    el.set_attribute("endpoint", "http://localhost:3030").unwrap();
    document().body().unwrap().append_child(&el).unwrap();

    sleep(Duration::from_millis(500)).await;

    let shadow = el.shadow_root().unwrap();
    let label = shadow.query_selector(".entity-label").unwrap();
    assert!(label.is_some());
}
```

**What this covers**: WASM ↔ JS interop, web component lifecycle, DOM
rendering, Sigma.js initialization.

**Polecat-friendly**: Partially — requires `wasm-pack` and a headless browser.
Polecats can run this if the CI environment has Chrome/Firefox installed.

### Layer 3: Playwright E2E Tests (Full Browser Automation)

End-to-end tests that exercise the complete UI flow. Playwright because it
supports Chrome, Firefox, and WebKit, and has good WASM/WebGL support.

```typescript
// tests/e2e/graph-explorer.spec.ts
import { test, expect } from '@playwright/test';

test('graph explorer shows entities from SPARQL query', async ({ page }) => {
  await page.goto('http://localhost:3030/');

  // Wait for graph to render
  await page.waitForSelector('canvas', { timeout: 5000 });

  // Verify entity count in sidebar
  const count = await page.textContent('.entity-count');
  expect(parseInt(count!)).toBeGreaterThan(0);
});

test('clicking entity opens detail panel', async ({ page }) => {
  await page.goto('http://localhost:3030/entity/aegis:koror');

  await expect(page.locator('.entity-label')).toContainText('koror');
  await expect(page.locator('.entity-type')).toContainText('ProxmoxNode');
  await expect(page.locator('.edge-list')).toBeVisible();
});

test('SPARQL workbench executes query and shows results', async ({ page }) => {
  await page.goto('http://localhost:3030/sparql');

  // Type a query
  await page.fill('.cm-content', 'SELECT ?s WHERE { ?s a aegis:ProxmoxNode }');
  await page.click('button.run-query');

  // Wait for results
  await page.waitForSelector('.results-table');
  const rows = await page.locator('.results-table tbody tr').count();
  expect(rows).toBeGreaterThan(0);
});

test('temporal slider filters graph by valid-time', async ({ page }) => {
  await page.goto('http://localhost:3030/');

  // Get initial entity count
  const before = await page.textContent('.entity-count');

  // Drag valid-time slider to a past date
  await page.locator('.valid-time-slider').fill('2026-01-01');
  await page.waitForTimeout(500); // Wait for re-query

  // Entity count should change (some entities didn't exist in January)
  const after = await page.textContent('.entity-count');
  expect(after).not.toEqual(before);
});

test('web component embedded in external page', async ({ page }) => {
  // Serve a minimal HTML page that embeds <quipu-graph>
  await page.goto('http://localhost:8080/test-embed.html');

  // The web component should render its shadow DOM
  const component = page.locator('quipu-graph');
  await expect(component).toBeVisible();

  // Sigma.js canvas should appear inside the shadow root
  const canvas = component.locator('canvas');
  await expect(canvas).toBeVisible();
});
```

**What this covers**: Full user flows, visual rendering, interactivity,
cross-browser compatibility, web component embedding.

**Polecat-friendly**: Yes — Playwright runs headless. Polecats can execute
the full suite. We already have Playwright MCP tools available.

### Layer 4: Visual Regression Tests

Capture screenshots of known-good graph states and compare against future
renders. Catches layout drift, style changes, and rendering bugs.

```typescript
test('graph explorer visual regression', async ({ page }) => {
  await page.goto('http://localhost:3030/?seed=42'); // Deterministic layout
  await page.waitForSelector('canvas');
  await page.waitForTimeout(1000); // Wait for force layout to settle

  await expect(page).toHaveScreenshot('graph-explorer-default.png', {
    maxDiffPixelRatio: 0.05, // Allow 5% pixel difference
  });
});
```

**Note**: Graph layouts are non-deterministic (force-directed). Use a fixed
random seed and `fullArea: false` to snapshot only the sidebar/controls,
not the graph canvas itself. Test the graph via structural assertions
(node count, edge count, selected node info) rather than pixel comparison.

### Layer 5: API Contract Tests

Test that Quipu's REST API returns valid responses for every endpoint the
UI depends on. These run without a browser — pure HTTP.

```rust
#[tokio::test]
async fn entity_endpoint_returns_json_ld() {
    let client = reqwest::Client::new();
    let resp = client.get("http://localhost:3030/entity/aegis:koror")
        .header("Accept", "application/ld+json")
        .send().await.unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "application/ld+json");

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["@type"], "aegis:ProxmoxNode");
    assert!(body["@id"].as_str().unwrap().contains("koror"));
}

#[tokio::test]
async fn content_negotiation_html_vs_json() {
    let client = reqwest::Client::new();

    // HTML request
    let html = client.get("http://localhost:3030/entity/aegis:koror")
        .header("Accept", "text/html")
        .send().await.unwrap();
    assert!(html.headers()["content-type"].to_str().unwrap()
        .contains("text/html"));

    // JSON-LD request
    let json = client.get("http://localhost:3030/entity/aegis:koror")
        .header("Accept", "application/ld+json")
        .send().await.unwrap();
    assert!(json.headers()["content-type"].to_str().unwrap()
        .contains("application/ld+json"));
}

#[tokio::test]
async fn spotlight_endpoint_annotates_text() {
    let client = reqwest::Client::new();
    let resp = client.post("http://localhost:3030/spotlight")
        .json(&serde_json::json!({
            "text": "koror NFS mount timeout",
            "confidence": 0.5
        }))
        .send().await.unwrap();

    let body: serde_json::Value = resp.json().await.unwrap();
    let annotations = body["annotations"].as_array().unwrap();
    assert!(!annotations.is_empty());
    assert_eq!(annotations[0]["surface"], "koror");
}
```

**Polecat-friendly**: Fully — these are `cargo test` with a running Quipu
server. Polecats can spin up a test server, run the suite, tear it down.

### Polecat Testing Workflow

Polecats can't look at a screen, but they can test everything that matters:

```text
1. cargo test                      # Layer 1: Rust unit tests
2. wasm-pack test --headless       # Layer 2: WASM integration
3. trunk serve &                   # Start the UI server
4. npx playwright test             # Layer 3: E2E + Layer 4: visual
5. cargo test --test api_contract  # Layer 5: API contracts
```

**Test fixtures**: Ship a `test-fixtures/` directory with:

- `test-store.db` — A pre-populated Quipu SQLite database with known entities
- `test-shapes.ttl` — SHACL shapes for the test ontology
- `test-episodes.json` — Sample episodes for timeline testing
- `test-embed.html` — Minimal page embedding `<quipu-*>` web components

Polecats load the test store, run the server against it, execute all test
layers. Deterministic, reproducible, no external dependencies.

### CI Pipeline

```yaml
# .github/workflows/ui-tests.yml
jobs:
  ui-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: wasm32-unknown-unknown }
      - uses: aspect-build/rules_ts/.github/actions/setup-node@v1
      - run: cargo install trunk wasm-pack
      - run: npx playwright install chromium
      - run: cargo test                          # Unit tests
      - run: wasm-pack test --headless --chrome  # WASM tests
      - run: trunk build                         # Build UI
      - run: |
          # Start server with test fixtures
          cargo run -- serve --db test-fixtures/test-store.db &
          sleep 2
          npx playwright test                    # E2E tests
```

## Open Questions

### 1. WASM Bundle Size

Leptos + spargebra + SHACL types compiled to WASM could be 2-5MB. Acceptable
for a standalone app, but heavy for a web component embedded in Bobbin's UI.
Mitigation options:

- Lazy-load the WASM module (show skeleton UI immediately)
- Split into core (graph rendering, ~500KB) and full (SPARQL workbench, ~3MB)
- Use Leptos islands to hydrate only the active component

### 2. Monaco vs CodeMirror for SPARQL Editor

Monaco is VS Code's editor — excellent but heavy (~2MB). CodeMirror 6 is
lighter (~200KB) and has a SPARQL mode. For the standalone app, Monaco is
fine. For embedded widgets, CodeMirror is more appropriate.

Recommendation: CodeMirror 6 with a custom SPARQL language package that uses
Quipu's own parser for validation.

### 3. Sigma.js Version

Sigma.js v3 (current) is TypeScript-first with a clean API. The WASM interop
boundary should target v3. Pin the version to avoid API drift.

### 4. Standalone vs Embedded: Same WASM or Separate Builds?

Option A: One WASM module, used by both standalone app and web components.
Option B: Standalone app is full Leptos SSR, web components are minimal WASM.

Recommendation: Option B. The standalone app benefits from SSR (SEO, initial
load speed, works without JS for basic browsing). Web components are
client-side only by nature. Different build profiles for different contexts.

### 5. How Does This Affect bobbin-fal?

The existing bobbin-fal bead and micro-ui.md design should be revised:

- Remove the `KnowledgeViewSet` trait and all rendering logic
- Replace with web component embedding (`<quipu-*>` elements)
- Bobbin's "knowledge tab" becomes an iframe or web component host
- Bobbin's `/kb/*` routes are removed — link to quipu.svc instead
- Keep the CSS design system sharing (Quipu adopts Bobbin's dark theme tokens)

## Phased Implementation

### Phase 1: Leptos Scaffold + Graph Explorer

- Set up Trunk build, Leptos app, basic routing
- Sigma.js + Graphology integration via wasm-bindgen
- Graph explorer with entity type filtering
- REST API integration (fetch entities, edges, SPARQL results)
- Dark theme

### Phase 2: SPARQL Workbench + Schema Browser

- CodeMirror integration with SPARQL syntax highlighting
- Schema-aware autocomplete from loaded SHACL shapes (WASM-side)
- Client-side query validation (spargebra in WASM)
- Table + graph result views
- Schema browser with type hierarchy tree
- SHACL shape cards

### Phase 3: Temporal Navigator

- Dual-axis time controls (valid-time + transaction-time)
- Graph diff view between time points
- Entity history panel
- Episode timeline view

### Phase 4: Web Component Export

- Extract graph explorer, entity card, SPARQL workbench as `<quipu-*>` elements
- Minimal WASM builds for each component
- postMessage protocol for host communication
- Documentation for embedding in Bobbin or any other host

### Phase 5: Bobbin Embedding

- Bobbin adds `<quipu-graph>` to search results (knowledge panel)
- Bobbin adds entity link badges to code search results
- Bobbin's context preview uses `<quipu-context>` to show knowledge contribution
- Shared CSS custom properties for visual consistency

## Design Principles

1. **Quipu renders Quipu.** No proxy rendering through another tool's templates.
2. **Embeddable, not embedded.** Quipu's UI works standalone. Embedding in
   Bobbin is an integration, not a dependency.
3. **Progressive disclosure.** Graph explorer first (visual, intuitive). SPARQL
   workbench for power users. Schema browser for ontology engineers. Temporal
   navigator for incident responders.
4. **Bitemporal is the differentiator.** No other tool does dual-axis time
   navigation on a knowledge graph. Lean into this.
5. **Dark, monospace, terminal-aesthetic.** This is a homelab tool. It should
   feel like it belongs next to a terminal, not a corporate dashboard.
6. **Deep-linkable everything.** Every entity, query, time point, and schema
   shape has a URL. Share a link to "koror as of March 27th" and it works.

## References

### Graph Visualization

- Sigma.js: <https://www.sigmajs.org>
- Graphology: <https://graphology.github.io>
- Gephi Lite (Sigma.js reference app): <https://github.com/gephi/gephi-lite>
- WebVOWL notation: <http://vowl.visualdataweb.org/v2/>
- SemSpect exploration tree: <https://www.semspect.de>
- Reagraph radial menus: <https://reagraph.dev>

### Frameworks & Build

- Leptos: <https://leptos.dev>
- Trunk: <https://trunkrs.dev>
- wasm-bindgen: <https://rustwasm.github.io/wasm-bindgen/>
- wasm-pack: <https://rustwasm.github.io/wasm-pack/>

### SPARQL & Query

- YASGUI: <https://yasgui.org>
- Sparklis guided queries: <https://github.com/sebferre/sparklis>
- Comunica (client-side query engine): <https://comunica.dev>

### Semantic Web Patterns

- JSON-LD spec: <https://www.w3.org/TR/json-ld11/>
- Linked Data Platform (LDP): <https://www.w3.org/TR/ldp/>
- Content negotiation: <https://www.w3.org/TR/cooluris/>
- Triple Pattern Fragments: <https://linkeddatafragments.org/>
- DBpedia Spotlight: <https://www.dbpedia-spotlight.org/>
- OpenRefine Reconciliation API: <https://reconciliation-api.github.io/specs/>
- Schema.org: <https://schema.org/SoftwareSourceCode>
- Wikidata Query Service: <https://query.wikidata.org>
- SOLID: <https://solidproject.org>
- Drupal RDF module: <https://www.drupal.org/docs/core-modules-and-themes/core-modules/rdf-module>

### Testing

- Playwright: <https://playwright.dev>
- wasm-bindgen-test: <https://rustwasm.github.io/wasm-bindgen/wasm-bindgen-test/>
