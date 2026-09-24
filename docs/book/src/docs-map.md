# Docs map

Every document in this repository that is not a page of this book, with
one line on what it is. Design notes record how a feature was decided and may
describe an earlier state; the book pages are the current description.

## Design notes

- [Conformance grammar: the versioned step-matching contract](https://github.com/scbrown/quipu/blob/main/docs/design/conformance-grammar.md) (`docs/design/conformance-grammar.md`)
- [Cross-graph concept alignment](https://github.com/scbrown/quipu/blob/main/docs/design/cross-graph-alignment.md) (`docs/design/cross-graph-alignment.md`)
- [Datalinks: a spatial explorer for Quipu graphs](https://github.com/scbrown/quipu/blob/main/docs/design/datalinks-3d.md) (`docs/design/datalinks-3d.md`)
- [A Unified Entailment Regime — Plan](https://github.com/scbrown/quipu/blob/main/docs/design/entailment-regime.md) (`docs/design/entailment-regime.md`)
- [Entity Resolution](https://github.com/scbrown/quipu/blob/main/docs/design/entity-resolution.md) (`docs/design/entity-resolution.md`)
- [Episode-Scoped Logical Retraction](https://github.com/scbrown/quipu/blob/main/docs/design/episode-retraction.md) (`docs/design/episode-retraction.md`)
- [Design: `RemoteProvider` — reaching remote Quipu instances](https://github.com/scbrown/quipu/blob/main/docs/design/federation-remote-provider.md) (`docs/design/federation-remote-provider.md`)
- [Flagship Reasoning Use Cases — Plan](https://github.com/scbrown/quipu/blob/main/docs/design/flagship-use-cases.md) (`docs/design/flagship-use-cases.md`)
- [Design: Fork-at-any-event — persistent named forks](https://github.com/scbrown/quipu/blob/main/docs/design/fork-at-any-event.md) (`docs/design/fork-at-any-event.md`)
- [Golden-path blessing: from a verified trajectory to a governed path](https://github.com/scbrown/quipu/blob/main/docs/design/golden-paths-blessing.md) (`docs/design/golden-paths-blessing.md`)
- [Design: Graph kinds and deep freeze — a data-kind axis on the label lattice, and cold storage that stays composable](https://github.com/scbrown/quipu/blob/main/docs/design/graph-kinds-and-deep-freeze.md) (`docs/design/graph-kinds-and-deep-freeze.md`)
- [Design: Graph Labels — freshness, trust and policy as a lattice over named graphs](https://github.com/scbrown/quipu/blob/main/docs/design/graph-labels.md) (`docs/design/graph-labels.md`)
- [Design: Group Isolation / Multi-Tenant Partitioning](https://github.com/scbrown/quipu/blob/main/docs/design/group-isolation.md) (`docs/design/group-isolation.md`)
- [Design: In-Memory Read Model — query in memory, write to SQLite](https://github.com/scbrown/quipu/blob/main/docs/design/in-memory-read-model.md) (`docs/design/in-memory-read-model.md`)
- [Design: Knowledge Packs — a graph, its shapes, its queries, and its retrieval policy as one artifact](https://github.com/scbrown/quipu/blob/main/docs/design/knowledge-packs.md) (`docs/design/knowledge-packs.md`)
- [Design: Multi-DB Composition — term spaces, ATTACH, and the blob sidecar](https://github.com/scbrown/quipu/blob/main/docs/design/multi-db-composition.md) (`docs/design/multi-db-composition.md`)
- [Design: Named Graphs (Quads) — the `graph × valid-time × tx-time` model](https://github.com/scbrown/quipu/blob/main/docs/design/named-graphs.md) (`docs/design/named-graphs.md`)
- [PageRank & Personalized PageRank — Specification](https://github.com/scbrown/quipu/blob/main/docs/design/pagerank.md) (`docs/design/pagerank.md`)
- [Design: The defaults comparison and the Governed Store principles](https://github.com/scbrown/quipu/blob/main/docs/design/paper-principles.md) (`docs/design/paper-principles.md`)
- [Design: Quipu paper plan — a governed bitemporal knowledge graph store](https://github.com/scbrown/quipu/blob/main/docs/design/paper.md) (`docs/design/paper.md`)
- [Persistence review evidence](https://github.com/scbrown/quipu/blob/main/docs/design/persistence-evidence/README.md) (`docs/design/persistence-evidence/README.md`)
- [Separate-process persistent-engine memory measurement](https://github.com/scbrown/quipu/blob/main/docs/design/persistence-evidence/separate-process-1m-20260914/README.md) (`docs/design/persistence-evidence/separate-process-1m-20260914/README.md`)
- [Separate-process memory comparison — quipu (SQLite) vs Oxigraph (RocksDB), run 4](https://github.com/scbrown/quipu/blob/main/docs/design/persistence-evidence/separate-process-1m-20260914/run4/README.md) (`docs/design/persistence-evidence/separate-process-1m-20260914/run4/README.md`)
- [WatDiv 1M diagnostic checkpoint — 2026-09-14](https://github.com/scbrown/quipu/blob/main/docs/design/persistence-evidence/watdiv-1m-20260914/README.md) (`docs/design/persistence-evidence/watdiv-1m-20260914/README.md`)
- [Persistence without whole-graph residency](https://github.com/scbrown/quipu/blob/main/docs/design/persistence-layer.md) (`docs/design/persistence-layer.md`)
- [Policy by example: from an observed edit to a governed rule](https://github.com/scbrown/quipu/blob/main/docs/design/policy-by-example.md) (`docs/design/policy-by-example.md`)
- [Performant edit hooks for policy](https://github.com/scbrown/quipu/blob/main/docs/design/policy-edit-hooks.md) (`docs/design/policy-edit-hooks.md`)
- [Quipu UI: Knowledge Graph Visualization & Exploration](https://github.com/scbrown/quipu/blob/main/docs/design/quipu-ui.md) (`docs/design/quipu-ui.md`)
- [Quipu Reasoner — Incremental Datalog on the Bitemporal Fact Log](https://github.com/scbrown/quipu/blob/main/docs/design/reasoner.md) (`docs/design/reasoner.md`)
- [Reasoning Engine Fixes — Plan](https://github.com/scbrown/quipu/blob/main/docs/design/reasoning-engine-fixes.md) (`docs/design/reasoning-engine-fixes.md`)
- [Semantic, entity-grounded edit policies](https://github.com/scbrown/quipu/blob/main/docs/design/semantic-grounded-edit-policies.md) (`docs/design/semantic-grounded-edit-policies.md`)
- [Semantic Reasoning Support — Gap Inventory](https://github.com/scbrown/quipu/blob/main/docs/design/semantic-reasoning-gaps.md) (`docs/design/semantic-reasoning-gaps.md`)
- [Design: Shape Versioning — a bitemporal registry for shapes and ontologies](https://github.com/scbrown/quipu/blob/main/docs/design/shape-versioning.md) (`docs/design/shape-versioning.md`)
- [Design: The Signing Plane — governing the trust root like everything else](https://github.com/scbrown/quipu/blob/main/docs/design/signing-plane.md) (`docs/design/signing-plane.md`)
- [Design: Spanner-class capabilities over any structured data](https://github.com/scbrown/quipu/blob/main/docs/design/spanner-capabilities.md) (`docs/design/spanner-capabilities.md`)
- [Standard share artifacts](https://github.com/scbrown/quipu/blob/main/docs/design/standard-share-artifact.md) (`docs/design/standard-share-artifact.md`)
- [Design: statement identity, edge properties, and bounded paths](https://github.com/scbrown/quipu/blob/main/docs/design/statement-identity.md) (`docs/design/statement-identity.md`)
- [Test Fixtures: Seed Data for UI Development and Demos](https://github.com/scbrown/quipu/blob/main/docs/design/test-fixtures.md) (`docs/design/test-fixtures.md`)
- [Quipu: AI-Native Knowledge Graph — Vision](https://github.com/scbrown/quipu/blob/main/docs/design/vision.md) (`docs/design/vision.md`)
- [Design: WebAssembly Support — running Quipu without a server](https://github.com/scbrown/quipu/blob/main/docs/design/wasm-support.md) (`docs/design/wasm-support.md`)

## Papers

- [The paper source](https://github.com/scbrown/quipu/blob/main/docs/paper/README.md) (`docs/paper/README.md`)
- [The merge paper source](https://github.com/scbrown/quipu/blob/main/docs/paper-merge/README.md) (`docs/paper-merge/README.md`)
- [arXiv submission card](https://github.com/scbrown/quipu/blob/main/docs/paper-merge/SUBMISSION.md) (`docs/paper-merge/SUBMISSION.md`)

## Release and operations

- [Releasing](https://github.com/scbrown/quipu/blob/main/docs/RELEASING.md) (`docs/RELEASING.md`)
- [Private-key checks](https://github.com/scbrown/quipu/blob/main/docs/private-key-checks.md) (`docs/private-key-checks.md`)
