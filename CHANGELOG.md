# Changelog

All notable changes to this project will be documented in this file.

## [0.1.0] - 2026-04-04

### Added

- Initial project scaffold with vision doc([6c56fb1](https://github.com/scbrown/quipu/commit/6c56fb11031e250f1d291651eb3299d2db158ade))
- *(core)* Implement EAVT bitemporal fact log schema([49b5321](https://github.com/scbrown/quipu/commit/49b53217d101b65bba95eafd570ad32508fad322))
- *(rdf)* Implement RDF data model layer with oxrdf integration([4e44b38](https://github.com/scbrown/quipu/commit/4e44b38027f8c139de450b609827890d44ca726c))
- *(sparql)* Implement SPARQL-to-SQL query engine via spargebra([a742c91](https://github.com/scbrown/quipu/commit/a742c9123dc6aa05a7c6e2a9115140075b800971))
- Add CLI demo and mdbook documentation([89387ad](https://github.com/scbrown/quipu/commit/89387ade2adb3c450e1eb43703d2b4f7f3b84530))
- *(shacl)* Implement write-time SHACL validation via rudof([08f8cb8](https://github.com/scbrown/quipu/commit/08f8cb8cfd4e71ef520b47a14fb9f01383290d38))
- *(mcp)* Implement MCP tool handlers for Bobbin integration([a53f5c0](https://github.com/scbrown/quipu/commit/a53f5c04bfad7b5afa516eab5b6d4aff4c403c31))
- *(sparql)* Implement ORDER BY, OPTIONAL, and REDUCED([97a9e7e](https://github.com/scbrown/quipu/commit/97a9e7ea16963c57896467bd98dab60acb00b4d9))
- *(cli)* Add cord, unravel, validate commands + SHACL support for knot([3ed26ea](https://github.com/scbrown/quipu/commit/3ed26ead92f191918786616c89af8623372a465f))
- *(vector)* Add SQLite-backed vector search with temporal filtering([0723c08](https://github.com/scbrown/quipu/commit/0723c08f893b64b55a8f7d60aa9de56cabe7a665))
- *(sparql)* Add RDFS subclass inference for type-hierarchy queries([b839298](https://github.com/scbrown/quipu/commit/b839298edaaf955881b4d179e296e6e52c391816))
- *(sparql)* Implement GROUP BY, aggregates, and Extend([c5795ce](https://github.com/scbrown/quipu/commit/c5795ce0f869d6479a774d9894c791fee9adc648))
- *(episode)* Add structured episode ingestion API for agent knowledge([4e26495](https://github.com/scbrown/quipu/commit/4e26495cdc049ac083f840fcf5189856af942f94))
- *(cli)* Add episode subcommand for structured knowledge ingestion([fe0604f](https://github.com/scbrown/quipu/commit/fe0604fd9c49600dcab317a5fd6cbd2684ab192e))
- *(mcp)* Add quipu_retract tool + CLI retract command([3b104fd](https://github.com/scbrown/quipu/commit/3b104fd79960ab337e40bbc2ba82b258d1c63b47))
- *(shacl)* Persistent shape storage with auto-validation on writes([cf4de8d](https://github.com/scbrown/quipu/commit/cf4de8d8970e36078f0765112533d1760375464d))
- *(episode)* Add SHACL validation gate, batch ingestion, provenance query (aegis-1wfq)([9f70a0c](https://github.com/scbrown/quipu/commit/9f70a0cac558319c6c42a52f75a9f81c3192458c))
- *(sparql)* Add temporal SPARQL queries (valid_at + as_of_tx)([46db89f](https://github.com/scbrown/quipu/commit/46db89f743ab39859d1130f58546da21762e297b))
- *(server)* Add axum REST API server mirroring MCP tool surface (aegis-1ghm)([a9eb8fa](https://github.com/scbrown/quipu/commit/a9eb8facdbcd17e6dc34c5d562c9cd8c2ef7ce61))
- *(graph)* Add petgraph projection API with algorithms (aegis-9wqz)([d270132](https://github.com/scbrown/quipu/commit/d270132789ad4e277e7087b5d8df680b78e9924a))
- *(provider)* Add GraphProvider trait for federated queries (aegis-8s9c)([0842816](https://github.com/scbrown/quipu/commit/0842816e9694857a8edd7d37dfbc2d35e687ce8e))
- *(config)* Add QuipuConfig with .bobbin/config.toml support (aegis-fvta)([c13baf2](https://github.com/scbrown/quipu/commit/c13baf282be414ff5005bdd76c6a48e83b000300))
- *(context)* Add unified context pipeline for knowledge-code blending (aegis-xk08)([815e640](https://github.com/scbrown/quipu/commit/815e64010334c091b40193b6910d5cf098a543e5))
- *(sparql)* ASK/CONSTRUCT/DESCRIBE + module splits + pre-commit hook (aegis-kqvf)([8102262](https://github.com/scbrown/quipu/commit/8102262df4d2e19d18b27ac3504857d4dad0013c))
- *(vector)* Add hybrid SPARQL + vector search tool (aegis-tni7)([ff46399](https://github.com/scbrown/quipu/commit/ff46399277233e1690ca101b145dc4128f4e312b))
- Add KnowledgeVectorStore trait + LanceDB backend (qp-cqv.1)([ea669c9](https://github.com/scbrown/quipu/commit/ea669c90f3a8a18b212771f79ebccfbd11a344df))
- Add release-plz workflow with changelog generation (qp-fgn.2)([01b7808](https://github.com/scbrown/quipu/commit/01b7808daae97b1cc6330077147401c97f8e96ce))
- Add LanceDB hybrid search with predicate pushdown (qp-cqv.2)([bb86cb6](https://github.com/scbrown/quipu/commit/bb86cb63527b027136ce81c203def6a32383c90c))
- Crates.io publishing prep — metadata, exclude, missing_docs lint (qp-fgn.3)([84bf8b0](https://github.com/scbrown/quipu/commit/84bf8b0d6ea4fad1e8a5d7d6ef1eb43d5dc132a0))
- Add auto-embed entities on write (knot/episode hooks) (qp-cqv.4)([126b7ea](https://github.com/scbrown/quipu/commit/126b7eabcbddf63afcc7f068dfa56ff796d894bf))
- Unified search results with source tagging for Bobbin integration (qp-sbu.3)([f1be2e0](https://github.com/scbrown/quipu/commit/f1be2e0b4b1044737ff63bf1f35488e188730a9a))
- LanceDB full-text search for context pipeline (qp-cqv.5)([bebda6f](https://github.com/scbrown/quipu/commit/bebda6f3570e294fdd8a1d813f75e939aabeab23))
- SHACL shapes for code entities (CodeModule, CodeSymbol, Document, Section, Bundle) (qp-1bw.1)([182dfa7](https://github.com/scbrown/quipu/commit/182dfa78d199d1544c2cb1ed5a084ac54106c8fa))
- Register bobbin: namespace and code entity IRI patterns (qp-1bw.2)([dee600c](https://github.com/scbrown/quipu/commit/dee600c5d58eb68cd39e56a1ffe21cdd6f9d0b56))
- Delegate vector search to external provider when knowledge feature enabled (qp-1bw.3)([2fe48a7](https://github.com/scbrown/quipu/commit/2fe48a77ceaf309fece21e3ed8c4ea87048f75ff))
- Post-index reconciliation to resolve cross-repo import edges (qp-1bw.4)([a3b148d](https://github.com/scbrown/quipu/commit/a3b148d7d80ce40e99bace5bb3dcb7e180c7fd0f))

### CI/CD

- Split Rust CI into fmt, clippy, test, build jobs with caching (qp-fgn.1)([c05d534](https://github.com/scbrown/quipu/commit/c05d534ddd20bc21fb73844b616ddfb034f255dc))

### Changed

- Extract hardcoded namespace strings to constants module (qp-i16)([868d63a](https://github.com/scbrown/quipu/commit/868d63a2ea8891b954939bfd27139cfcd62d4920))

### Documentation

- Comprehensive mdbook update — all features documented (aegis-zikg)([fde105b](https://github.com/scbrown/quipu/commit/fde105b0a936c5d91197b1af37367405f592b4c7))
- Refresh README with feature showcase and comparison table (aegis-nfeg)([bc25708](https://github.com/scbrown/quipu/commit/bc25708783711112486affd4b01c2c4abdf52efb))
- Enrich CLAUDE.md + CONTRIBUTING.md for polecat agents([05b6148](https://github.com/scbrown/quipu/commit/05b61483ec1f47cdfac3310134aff3f37ac40fb4))
- Add quipu SVG logo and polish README with emoji([3bc4fa4](https://github.com/scbrown/quipu/commit/3bc4fa4f447d791994e75ce449ed1ef140c2e44b))
- Update mdbook + README with LanceDB, CI/CD, and Bobbin integration (qp-b7q)([baf1154](https://github.com/scbrown/quipu/commit/baf115451a35617880c9bf526fb19ea531aa7af3))

### Fixed

- *(store)* Retract closes original assertion's valid_to([98f0072](https://github.com/scbrown/quipu/commit/98f00724f8a6d2444619e332be95fb856a7e472b))
- Resolve rebase conflicts + clippy fixes in provider/context (aegis-kqvf)([7d7e423](https://github.com/scbrown/quipu/commit/7d7e423ee6389b8a28677e32e83576fe08dac075))
- *(ci)* Allow docs deploy on workflow_dispatch, not just push (aegis-4tid)([bf1aaa4](https://github.com/scbrown/quipu/commit/bf1aaa4394170b088ff9f1fb0810c4deaf4ce7c9))
- Handle feature-gated Triple variants in match arms (qp-sbu.1)([a476f2a](https://github.com/scbrown/quipu/commit/a476f2a7bd5fb3dbe0603ff5234053bf46823ad1))
- Resolve clippy and markdown lint failures (qp-28q)([e231399](https://github.com/scbrown/quipu/commit/e2313998261fb015719f9bbdb9ec2858436800af))

### Miscellaneous

- Downgrade rusqlite to 0.33 for Bobbin compatibility (aegis-w6rt)([7a5f2b2](https://github.com/scbrown/quipu/commit/7a5f2b2d95c2b73629021f699764c7f6e6f03ab4))
- Update Cargo.lock for lancedb deps (qp-cqv.1)([026acb8](https://github.com/scbrown/quipu/commit/026acb80bc73f56dba5ac30d694513eb71f25070))
- Fix trailing newlines and markdown lint errors (qp-fgn.3)([38cf31a](https://github.com/scbrown/quipu/commit/38cf31a2cd8ddc7556432c7d745ed05b73f31b10))

### Testing

- *(sparql)* Add HAVING, COUNT(*) empty, GROUP BY+SUM tests (aegis-i2o7)([4d0434c](https://github.com/scbrown/quipu/commit/4d0434c10682efd39b11a0918c0c5a3a6908b51f))
# Changelog

All notable changes to this project will be documented in this file.
