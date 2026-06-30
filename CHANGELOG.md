# Changelog

All notable changes to this project will be documented in this file.

## [0.3.2] - 2026-06-30

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
- Auto-embed query text in search endpoints via EmbeddingProvider (qp-xlz)([95e18ee](https://github.com/scbrown/quipu/commit/95e18ee42d0ad96caf33accddb3d759355541263))
- Add quipu_search_nodes and quipu_search_facts MCP tools (qp-xu7)([3146322](https://github.com/scbrown/quipu/commit/31463221334debb8f3b679d93df3951d05e62c2c))
- Graphiti-compatible REST endpoints /search/nodes and /episodes/complete (qp-5w2)([daef471](https://github.com/scbrown/quipu/commit/daef47120c55009d1e60d5c68acf67aa04cc03e1))
- Migrate SQLite vectors to LanceDB + dual-mode config (qp-cqv.3)([455a8e8](https://github.com/scbrown/quipu/commit/455a8e8992f8f4a4d0fee99e8116891508fe5a26))
- *(shacl)* Add aegis ontology SHACL shapes([da19a7b](https://github.com/scbrown/quipu/commit/da19a7b332e2b336b1b6f10c4bb604bb6a7a5de1))
- *(sparql)* Implement SPARQL 1.1 property paths([280ac51](https://github.com/scbrown/quipu/commit/280ac517a1295bf9d1ca3c215cd8854210c221f4))
- *(ui)* Add standalone web UI with graph explorer and SPARQL workbench (qp-w2p)([32cf2ae](https://github.com/scbrown/quipu/commit/32cf2ae1232e8e8294022213ce05e10b8c814326))
- *(ui)* Add Leptos WASM scaffold with Sigma.js graph explorer (qp-m2g)([4a0aa62](https://github.com/scbrown/quipu/commit/4a0aa623cd847c565ff084bf8c8885a6069c7058))
- *(shacl)* Complete aegis ontology shapes — add missing types and fix constraints (aegis-uito)([603cc97](https://github.com/scbrown/quipu/commit/603cc97bf78519c55a92220f45b19ad098261ef7))
- *(server)* Add ONNX embedding provider for standalone quipu-server (aegis-k36k)([ad275f1](https://github.com/scbrown/quipu/commit/ad275f15483123643a804ded0626c1d579142247))
- *(reasoner)* Add `quipu impact` CLI (Phase 1)([c49ee8e](https://github.com/scbrown/quipu/commit/c49ee8ef5423e63ca12523304b8a90b8ff432879))
- *(reasoner)* Phase 2 skeleton — rule AST and Turtle parser([1f71b44](https://github.com/scbrown/quipu/commit/1f71b445df244e780c844b6bf91ac4bd17e1a0c3))
- *(reasoner)* Stratify rules with cycle detection([8710ea8](https://github.com/scbrown/quipu/commit/8710ea846ab602e0eba0c03143e6ed72c09d4af7))
- *(reasoner)* Evaluate rules on datafrog with provenance([2473eb4](https://github.com/scbrown/quipu/commit/2473eb415bb4c48dea952fd22e99668b6e7a702f))
- *(reasoner)* Add quipu reason CLI and aegis rules([37c192e](https://github.com/scbrown/quipu/commit/37c192eb25641836b32552fb865813db0baea514))
- *(reasoner)* Phase 3 — reactive evaluation via TransactObserver([aab6d30](https://github.com/scbrown/quipu/commit/aab6d304e42475d55cf34e143d5c5b8416d48641))
- *(reasoner)* Phase 4 — counterfactual queries via speculate()([563e6c2](https://github.com/scbrown/quipu/commit/563e6c28cbd4842550964b329bf994bb7a8eca5a))
- *(ui)* Phase 3 — temporal navigator + episode timeline([fc0e0ab](https://github.com/scbrown/quipu/commit/fc0e0ab766556efd929458a2bdec6a31a5a02d36))
- *(ui)* Phase 4 — web component export + semantic web APIs([2153019](https://github.com/scbrown/quipu/commit/2153019dece687888a375f56c5d89a4430173e3c))
- *(fixtures)* Static test assets — shapes, episodes, embed HTML([564436e](https://github.com/scbrown/quipu/commit/564436ee1670a19f72a5b00b5b8076fd38c11df8))
- *(fixtures)* Seed binary + justfile recipes([cf0518a](https://github.com/scbrown/quipu/commit/cf0518a5874572f1b77bf37bb0f4ceabd11a06a6))
- *(proposal)* Agent-driven schema evolution proposals([1a5ae98](https://github.com/scbrown/quipu/commit/1a5ae9801b38f7d19ab2ed284787c60e16b71717))
- Wire schema proposal MCP tools into REST server + add round-trip tests (aegis-g883)([2d227c7](https://github.com/scbrown/quipu/commit/2d227c71285e46fe26b9df84f300a804c434a972))
- *(owl)* OWL 2 RL ontology layer — class hierarchy, materialization, and validation([ecc6354](https://github.com/scbrown/quipu/commit/ecc6354dd1a8c8503c9d41a9db69786a30ecbfbf))
- *(graph)* PageRank & Personalized PageRank over ProjectedGraph ([#11](https://github.com/scbrown/quipu/pull/11))([7f984b4](https://github.com/scbrown/quipu/commit/7f984b4e568c994b499e3cca7815139b90e0491a))
- *(server)* Add CORS headers for cross-origin API access (GH#5)([b567b77](https://github.com/scbrown/quipu/commit/b567b770c54003bee1e0c2948f519848d2859a4c))
- *(resolution)* Wire entity resolution into episode ingest (hq-uye) ([#15](https://github.com/scbrown/quipu/pull/15))([7dbdba1](https://github.com/scbrown/quipu/commit/7dbdba14a52adefabe794535500d918f60bc783b))
- *(search)* Clamp limits + max_limit config + SPARQL row ceiling (hq-gkd) ([#16](https://github.com/scbrown/quipu/pull/16))([421a3b1](https://github.com/scbrown/quipu/commit/421a3b1982feba4558c8669b7142a653dc4022b4))
- *(shacl)* Auto-apply loaded shapes on episode ingest (hq-c6s) ([#19](https://github.com/scbrown/quipu/pull/19))([5a88d8d](https://github.com/scbrown/quipu/commit/5a88d8dd46d5c64b183998334a6cf78ced47ff75))
- *(server)* Bearer auth + read-only mode + CORS allowlist (hq-azs) ([#20](https://github.com/scbrown/quipu/pull/20))([2d0f48e](https://github.com/scbrown/quipu/commit/2d0f48ef539b4abde54bf78a5d46e48ac3e2fb19))
- *(mcp)* Named-query catalog + quipu_ask tool / /ask endpoint (hq-h75) ([#22](https://github.com/scbrown/quipu/pull/22))([61b8cf9](https://github.com/scbrown/quipu/commit/61b8cf9b3d20c9770512edd6593f65e93498ba46))
- *(episode)* Idempotent ingest via content-hash key (hq-fhc) ([#24](https://github.com/scbrown/quipu/pull/24))([03c9d44](https://github.com/scbrown/quipu/commit/03c9d44a509c8b78631e22d4ee217606b363f855))
- *(episode)* Per-edge confidence qualifier on ingest (hq-cug6) ([#29](https://github.com/scbrown/quipu/pull/29))([622bd13](https://github.com/scbrown/quipu/commit/622bd13dfec533a47a756cab4c0ec2a1a7c26247))
- *(graph)* Deterministic Louvain community detection (hq-zlph) ([#31](https://github.com/scbrown/quipu/pull/31))([62fe4fa](https://github.com/scbrown/quipu/commit/62fe4fa24d95cffbeeb99bc85c26b8132f4c3487))
- *(report)* Live graph report endpoint + quipu_report MCP tool (hq-ct27) ([#32](https://github.com/scbrown/quipu/pull/32))([746dc06](https://github.com/scbrown/quipu/commit/746dc06fa52092a0259fd3b881e9ebdaecda0293))

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
- Add v0.1.0 CHANGELOG and feature matrix to README (qp-u7l)([e8d8d2e](https://github.com/scbrown/quipu/commit/e8d8d2e60c1db4ea69715b222d1936abfa7981da))
- *(lint)* Fix pre-existing markdown lint violations([093c4b3](https://github.com/scbrown/quipu/commit/093c4b32f9e98f76a59aebf491a754b9c2db80a3))
- Reconcile quipu docs with implementation (hq-66e)([3f1b37a](https://github.com/scbrown/quipu/commit/3f1b37abee2e5b43722993d1fbb39232173ea3f9))

### Fixed

- *(store)* Retract closes original assertion's valid_to([98f0072](https://github.com/scbrown/quipu/commit/98f00724f8a6d2444619e332be95fb856a7e472b))
- Resolve rebase conflicts + clippy fixes in provider/context (aegis-kqvf)([7d7e423](https://github.com/scbrown/quipu/commit/7d7e423ee6389b8a28677e32e83576fe08dac075))
- *(ci)* Allow docs deploy on workflow_dispatch, not just push (aegis-4tid)([bf1aaa4](https://github.com/scbrown/quipu/commit/bf1aaa4394170b088ff9f1fb0810c4deaf4ce7c9))
- Handle feature-gated Triple variants in match arms (qp-sbu.1)([a476f2a](https://github.com/scbrown/quipu/commit/a476f2a7bd5fb3dbe0603ff5234053bf46823ad1))
- Resolve clippy and markdown lint failures (qp-28q)([e231399](https://github.com/scbrown/quipu/commit/e2313998261fb015719f9bbdb9ec2858436800af))
- Remove quipu-ui build artifacts from tracking (qp-m2g)([6fed42d](https://github.com/scbrown/quipu/commit/6fed42db549f46cc48b9f98815f009210baca037))
- Auto-save uncommitted implementation work (gt-pvx safety net)([1be5043](https://github.com/scbrown/quipu/commit/1be50438489fb3c9e39cddea50ae8194c4279ed4))
- Auto-save uncommitted implementation work (gt-pvx safety net)([df3e7e1](https://github.com/scbrown/quipu/commit/df3e7e1938f44b29b932683809bf22ac49567759))
- *(server)* Remove suffix entity routes that panic on axum 0.8+([583de29](https://github.com/scbrown/quipu/commit/583de292adff4ae5eb4b3a2eefcd267258e984e1))
- *(server)* Use sub-path routes for entity format endpoints([4d80832](https://github.com/scbrown/quipu/commit/4d80832ee758e8ecc3b2d4c1e34bf61b2fcddeca))
- Idempotent assertions — deduplicate (e,a,v) facts across transactions([afe3dda](https://github.com/scbrown/quipu/commit/afe3ddad4685140ff6bd8409de100c2c7b1a3986))
- Close pre-commit CI parity gaps (aegis-6h0r) ([#4](https://github.com/scbrown/quipu/pull/4))([b280007](https://github.com/scbrown/quipu/commit/b280007607366d55261f37febe640428ceb0cb40))
- *(ui)* Force-directed graph, working SPARQL editor, richer timeline (GH#6,#8,#9)([d1192c2](https://github.com/scbrown/quipu/commit/d1192c2b0f1d53f7ffa5db470c3fd05ce7e9b773))
- *(sparql)* Implement FILTER builtins — CONTAINS/STR/LCASE/isIRI no longer no-op (GH#12)([17ac2eb](https://github.com/scbrown/quipu/commit/17ac2ebc3adee8547007f9a914da6851d9c91416))
- *(sparql)* DISTINCT current-fact rows in BGP — stop COUNT/OPTIONAL inflation (GH#13)([ca091fa](https://github.com/scbrown/quipu/commit/ca091fae507ce705af1c39162065101ce34e3627))
- *(sparql)* Real FILTER REGEX + fail loud on unsupported builtins (hq-9hs)([5f47ae6](https://github.com/scbrown/quipu/commit/5f47ae659cb115dfe3bb8a3d1f8b812dd5a156dc))
- *(mcp)* Register quipu_project + quipu_context in tool_definitions (hq-6v4)([beaaebf](https://github.com/scbrown/quipu/commit/beaaebfa878d3c5d4bff3e9397046ac363362ed1))
- *(mcp)* Gate quipu_load_ontology behind owl feature; matrix not 'Planned' (hq-8wd)([b206053](https://github.com/scbrown/quipu/commit/b2060539736ffa26edd6091e02b54fe7aa4f3886))
- *(time)* Stamp write paths with real clock, not 1970 epoch (hq-tb4) ([#14](https://github.com/scbrown/quipu/pull/14))([04becf1](https://github.com/scbrown/quipu/commit/04becf17ebd249be321a03b593d94715d3b11865))
- *(search)* Tool_search honors group_ids + entity_type (hq-93d) ([#17](https://github.com/scbrown/quipu/pull/17))([2d8d35c](https://github.com/scbrown/quipu/commit/2d8d35c2e91e6485d19fdb6fdadee08e31a7bea9))
- *(embedding)* Cap ONNX tokenizer length + fail loud on dim mismatch (hq-7v0) ([#18](https://github.com/scbrown/quipu/pull/18))([3f2aed9](https://github.com/scbrown/quipu/commit/3f2aed9b27163ec6e413b5cd89fc2625ef768536))
- *(search)* Rank query-named entity above referencing nodes (hq-5gi) ([#21](https://github.com/scbrown/quipu/pull/21))([114a492](https://github.com/scbrown/quipu/commit/114a492b9f3606edae75b0bc3682cf8b3bb88e05))
- *(embedding)* Deterministic re-embed on label/comment change (hq-xqc) ([#25](https://github.com/scbrown/quipu/pull/25))([68e2036](https://github.com/scbrown/quipu/commit/68e203644cb1829adb20685c49853937e8c1cb42))

### Miscellaneous

- Downgrade rusqlite to 0.33 for Bobbin compatibility (aegis-w6rt)([7a5f2b2](https://github.com/scbrown/quipu/commit/7a5f2b2d95c2b73629021f699764c7f6e6f03ab4))
- Update Cargo.lock for lancedb deps (qp-cqv.1)([026acb8](https://github.com/scbrown/quipu/commit/026acb80bc73f56dba5ac30d694513eb71f25070))
- Fix trailing newlines and markdown lint errors (qp-fgn.3)([38cf31a](https://github.com/scbrown/quipu/commit/38cf31a2cd8ddc7556432c7d745ed05b73f31b10))
- *(release)* Quipu 0.3.0([7ca9d59](https://github.com/scbrown/quipu/commit/7ca9d59a2e69885cc9ca3c017671a169f6a78490))
- *(release)* Quipu 0.3.1([f358ca3](https://github.com/scbrown/quipu/commit/f358ca3e3e294449996b4e5c1672ba543d6e301f))
- *(beads)* Untrack host-local .beads/redirect (was broken ../../../.beads)([892b1f7](https://github.com/scbrown/quipu/commit/892b1f7b3c2b245d9e473daec65da4d077fd960b))
- Bump version 0.3.1 -> 0.3.2 for release ([#30](https://github.com/scbrown/quipu/pull/30))([36b1db2](https://github.com/scbrown/quipu/commit/36b1db282f864b5861fa474f64e90dc7b74fc6a1))

### Testing

- *(sparql)* Add HAVING, COUNT(*) empty, GROUP BY+SUM tests (aegis-i2o7)([4d0434c](https://github.com/scbrown/quipu/commit/4d0434c10682efd39b11a0918c0c5a3a6908b51f))

### Merge

- Integrate entity resolution from polecat/fury-entity-resolution([6717a87](https://github.com/scbrown/quipu/commit/6717a87502ae838146a24806f5489fc7288b478e))

### Release

- V0.2.0 — reasoner engine + web UI enhancements([7527ad4](https://github.com/scbrown/quipu/commit/7527ad4eab2a36923c24e8c24cad7d3b3bc6e4b9))

## [0.3.1] - 2026-06-27

Critical SPARQL query-engine correctness fixes.

### Fixed

- **FILTER builtins were no-ops (#12)** — `FILTER(CONTAINS(...))`, `isIRI`,
  `STRSTARTS/STRENDS`, and nested `CONTAINS(LCASE(STR(?x)), ..)` returned ALL
  rows regardless of the predicate (only `Regex` was handled; everything else
  passed through). Implemented CONTAINS/STRSTARTS/STRENDS/isIRI/isBlank/
  isLiteral/isNumeric (bool) + STR/LCASE/UCASE (value). This restores text
  search in `tool_context`/`unified_search` (and Bobbin's `knowledge_context`,
  which wraps it) and entity-linking filters.
- **COUNT / OPTIONAL inflation (#13)** — a triple re-asserted across
  transactions left multiple current rows for the same `(e,a,v)`; BGP queries
  lacked `DISTINCT`, so duplicates multiplied under OPTIONAL/joins (e.g. 23174
  rows for 11 entities) and inflated `COUNT` (the bogus Shapes-Distribution
  counts). Added `DISTINCT` to the current-fact selects (BGP, rdf:type/subclass,
  property paths) — one solution per current triple.

## [0.3.0] - 2026-06-27

Graph-algorithm ranking, cross-origin API access, and a Web UI overhaul.

### Added

- **PageRank & Personalized PageRank** over the projected graph —
  `page_rank()` + `PageRankConfig`, exposed via `tool_project`
  (`"algorithm": "pagerank"` / `"ppr"`, with `seeds`/`damping`/`max_iters`),
  the `quipu project` CLI, REST `POST /project`, and MCP. Closes the
  long-standing "centrality" gap (only `in_degree` shipped before). Consumed by
  Bobbin's PPR retrieval ranking signal.
- **CORS** on the HTTP API (`/query`, `/search`, `/episode`, …) incl. OPTIONS
  preflight, so browser clients like Bobbin's Knowledge tab can call quipu
  cross-origin (#5).

### Fixed (Web UI)

- **Graph Explorer** uses a force-directed (cose) / hierarchical layout instead
  of the unreadable grid at large entity counts (#6).
- **Timeline** orders newest-first, hides decommissioned `graphiti-fact-*`
  episodes, and shows a summary line + per-episode entity-count chips (#8;
  partial #7).
- **Workbench SPARQL editor** renders on first view (was blank when initialized
  in a hidden container) with a plain-textarea fallback (#9).

### Known issues (fast-follow, 0.3.1)

- #7 (remove `graphiti-fact-*` episode data) and #10 (merge duplicate
  `aegis:WebApplication` entities) are deploy-gated live-data migrations; the UI
  symptom for #7 is already mitigated above.

## [0.2.0] - 2026-04-12

### Reasoner

- **Impact analysis CLI** — `quipu impact <entity-IRI>` walks entity edges via
  BFS with configurable hop depth and predicate filters
  ([c49ee8e](https://github.com/scbrown/quipu/commit/c49ee8e))
- **Datalog rule engine** — rule AST, Turtle DSL parser, stratified
  negation-as-failure with cycle detection, semi-naive evaluation via `datafrog`
  with full provenance tracking; `quipu reason` CLI command
  ([1f71b44](https://github.com/scbrown/quipu/commit/1f71b44),
  [8710ea8](https://github.com/scbrown/quipu/commit/8710ea8),
  [2473eb4](https://github.com/scbrown/quipu/commit/2473eb4),
  [37c192e](https://github.com/scbrown/quipu/commit/37c192e))
- **Reactive evaluation** — `TransactObserver` keeps derived facts fresh as base
  facts change; delta-aware re-evaluation triggered only by affected predicates
  ([aab6d30](https://github.com/scbrown/quipu/commit/aab6d30))
- **Counterfactual queries** — `Store::speculate()` forks a hypothetical view via
  SQLite SAVEPOINT; `quipu impact --remove` flag, REST `POST /impact` endpoint,
  and `quipu_impact` MCP tool
  ([563e6c2](https://github.com/scbrown/quipu/commit/563e6c2))

### Web UI

- **SPARQL Workbench** — syntax-highlighted CodeMirror editor with tabular/JSON
  output, query examples library, and time-travel parameter support
  ([65b5967](https://github.com/scbrown/quipu/commit/65b5967))
- **Temporal Navigator** — episode timeline with chronological view, extracted
  entities, and metadata display
  ([fc0e0ab](https://github.com/scbrown/quipu/commit/fc0e0ab))
- **Web component export** — embeddable `<quipu-graph>`, `<quipu-sparql>`,
  `<quipu-entity>`, `<quipu-timeline>`, `<quipu-schema>` custom elements for
  embedding Quipu panels in any page
  ([2153019](https://github.com/scbrown/quipu/commit/2153019))
- **Semantic Web APIs** — Spotlight entity recognition (`POST /spotlight`),
  Triple Pattern Fragments (`GET /fragments`), OpenRefine reconciliation
  (`POST /reconcile`), and content negotiation on `/entity/{iri}`
  ([2153019](https://github.com/scbrown/quipu/commit/2153019))

### Server

- **Entity format sub-path routes** — `GET /entity/{iri}/json` and
  `/entity/{iri}/ttl` replace suffix-based routes for axum 0.8+ compatibility
  ([583de29](https://github.com/scbrown/quipu/commit/583de29),
  [4d80832](https://github.com/scbrown/quipu/commit/4d80832))

### Test Fixtures

- **Seed binary and justfile recipes** — `just fixtures seed` and
  `just fixtures load` for populating test databases with realistic data
  ([cf0518a](https://github.com/scbrown/quipu/commit/cf0518a),
  [564436e](https://github.com/scbrown/quipu/commit/564436e))

### Documentation

- Comprehensive mdbook chapters for the reasoner — concepts, rule-builder
  tutorial, and CLI reference
  ([860dec3](https://github.com/scbrown/quipu/commit/860dec3))
- Reasoner design document
  ([340a55d](https://github.com/scbrown/quipu/commit/340a55d))
- Test fixtures design document
  ([3638c16](https://github.com/scbrown/quipu/commit/3638c16))

## [0.1.0] - 2026-04-05

Initial public release.

### Knowledge Graph Core

- **EAVT bitemporal fact log** — immutable fact storage with transaction time
  and valid time, time-travel queries, full audit trail
  ([49b5321](https://github.com/scbrown/quipu/commit/49b5321))
- **RDF data model** — IRIs, blank nodes, typed literals via oxrdf; import/export
  Turtle, N-Triples, JSON-LD, RDF/XML
  ([4e44b38](https://github.com/scbrown/quipu/commit/4e44b38))
- **SPARQL 1.1 query engine** — SELECT, ASK, CONSTRUCT, DESCRIBE with BGP, JOIN,
  UNION, FILTER, OPTIONAL, ORDER BY, GROUP BY, HAVING, aggregates, BIND, property
  paths, RDFS subclass inference, and temporal queries (`valid_at`, `as_of_tx`)
  ([a742c91](https://github.com/scbrown/quipu/commit/a742c91),
  [97a9e7e](https://github.com/scbrown/quipu/commit/97a9e7e),
  [c5795ce](https://github.com/scbrown/quipu/commit/c5795ce),
  [8102262](https://github.com/scbrown/quipu/commit/8102262),
  [b839298](https://github.com/scbrown/quipu/commit/b839298),
  [46db89f](https://github.com/scbrown/quipu/commit/46db89f),
  [280ac51](https://github.com/scbrown/quipu/commit/280ac51))
- **SHACL validation** — write-time schema enforcement with persistent shape
  storage and structured feedback (severity, focus node, path, message); optional
  via `shacl` feature flag
  ([08f8cb8](https://github.com/scbrown/quipu/commit/08f8cb8),
  [cf4de8d](https://github.com/scbrown/quipu/commit/cf4de8d),
  [9949807](https://github.com/scbrown/quipu/commit/9949807))
- **Aegis ontology SHACL shapes** — pre-built shapes for infrastructure entities
  ([da19a7b](https://github.com/scbrown/quipu/commit/da19a7b))
- **Code entity SHACL shapes** — shapes for CodeModule, CodeSymbol, Document,
  Section, Bundle
  ([182dfa7](https://github.com/scbrown/quipu/commit/182dfa7))

### AI-Native Features

- **Episode ingestion** — structured write path for agent-extracted knowledge
  with typed nodes, edges, provenance tracking, SHACL validation gate, and
  batch ingestion
  ([4e26495](https://github.com/scbrown/quipu/commit/4e26495),
  [9f70a0c](https://github.com/scbrown/quipu/commit/9f70a0c))
- **Dual vector backends** — default SQLite (brute-force cosine similarity) or
  optional LanceDB (ANN with predicate pushdown, Arrow columnar storage, full-text
  search) via `--features lancedb`
  ([0723c08](https://github.com/scbrown/quipu/commit/0723c08),
  [ea669c9](https://github.com/scbrown/quipu/commit/ea669c9),
  [bb86cb6](https://github.com/scbrown/quipu/commit/bb86cb6),
  [455a8e8](https://github.com/scbrown/quipu/commit/455a8e8))
- **Hybrid search** — SPARQL filters candidates, vector similarity ranks them;
  type constraints pushed down into the vector index
  ([ff46399](https://github.com/scbrown/quipu/commit/ff46399))
- **Auto-embed on write** — entities automatically embedded at knot/episode
  ingestion time
  ([126b7ea](https://github.com/scbrown/quipu/commit/126b7ea))
- **Context pipeline** — unified knowledge context for agent consumption with
  text search, link expansion, configurable depth and budget
  ([815e640](https://github.com/scbrown/quipu/commit/815e640))
- **EmbeddingProvider trait** — shared ONNX pipeline for auto-embedding queries
  in search endpoints
  ([95e18ee](https://github.com/scbrown/quipu/commit/95e18ee))

### Interfaces

- **CLI** — `quipu knot`, `quipu read`, `quipu cord`, `quipu unravel`,
  `quipu validate`, `quipu episode`, `quipu retract`, `quipu repl`, `quipu stats`
  ([89387ad](https://github.com/scbrown/quipu/commit/89387ad),
  [3ed26ea](https://github.com/scbrown/quipu/commit/3ed26ea),
  [fe0604f](https://github.com/scbrown/quipu/commit/fe0604f))
- **REST API** — axum server mirroring MCP tool surface with Graphiti-compatible
  `/search/nodes` and `/episodes/complete` endpoints
  ([a9eb8fa](https://github.com/scbrown/quipu/commit/a9eb8fa),
  [daef471](https://github.com/scbrown/quipu/commit/daef471))
- **Web UI** — standalone graph explorer with force-directed visualization,
  SPARQL workbench, episode timeline, and schema inspector
  ([32cf2ae](https://github.com/scbrown/quipu/commit/32cf2ae))
- **MCP tools** — 11 tools for agent integration including `quipu_context`,
  `quipu_episode`, `quipu_search_nodes`, `quipu_search_facts`, `quipu_retract`
  ([a53f5c0](https://github.com/scbrown/quipu/commit/a53f5c0),
  [3146322](https://github.com/scbrown/quipu/commit/3146322),
  [3b104fd](https://github.com/scbrown/quipu/commit/3b104fd))

### Infrastructure

- **Graph projection** — petgraph API with centrality, connected components,
  shortest path algorithms
  ([d270132](https://github.com/scbrown/quipu/commit/d270132))
- **Federation** — `GraphProvider` trait for multi-source queries
  ([0842816](https://github.com/scbrown/quipu/commit/0842816))
- **Configuration** — `QuipuConfig` with `.bobbin/config.toml` support
  ([c13baf2](https://github.com/scbrown/quipu/commit/c13baf2))
- **Bobbin integration** — namespace registration, code entity IRI patterns,
  external vector provider delegation, cross-repo import reconciliation,
  unified search results with source tagging
  ([dee600c](https://github.com/scbrown/quipu/commit/dee600c),
  [2fe48a7](https://github.com/scbrown/quipu/commit/2fe48a7),
  [a3b148d](https://github.com/scbrown/quipu/commit/a3b148d),
  [f1be2e0](https://github.com/scbrown/quipu/commit/f1be2e0))

### CI/CD

- GitHub Actions with fmt, clippy, test, and build jobs with caching
  ([c05d534](https://github.com/scbrown/quipu/commit/c05d534))
- release-plz for automated version bumps and changelog generation
  ([01b7808](https://github.com/scbrown/quipu/commit/01b7808))
- Pre-commit hooks for formatting, linting, and file size limits

### Documentation

- Comprehensive mdbook with persona-driven tutorials, SPARQL guide, and recipes
  ([d6504d2](https://github.com/scbrown/quipu/commit/d6504d2))
