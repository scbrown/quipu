# Changelog

All notable changes to this project will be documented in this file.

## [0.3.14] - 2026-08-01

### Fixed

- *(store)* Scope the committed read path to ROOT (#56)([e3e13c6](https://github.com/scbrown/quipu/commit/e3e13c64f206a296dfb01bc446834e4461ebee36))

## [0.3.13] - 2026-08-01

### Added

- *(sparql)* VALUES and FILTER IN / NOT IN (#51, #52)([0576868](https://github.com/scbrown/quipu/commit/0576868ba9a3bf4b560942dde50df33115207cee))

### Fixed

- *(embedding)* Name the missing config instead of failing silently (#53)([5465b58](https://github.com/scbrown/quipu/commit/5465b5858e380e4c27a2c245d42a81198f3b1936))
- *(lint)* Unbreak CI and split the files this repo's own limit rejects([ecab46f](https://github.com/scbrown/quipu/commit/ecab46f96d139f392a09df27b3bfad22bb9b2133))

## [0.3.12] - 2026-07-28

### Added

- *(sparql)* Named-graph query scoping — GRAPH <iri> / GRAPH ?g (#36) (#49)([dda3ee8](https://github.com/scbrown/quipu/commit/dda3ee82e7e4e3f06058413dabe8ce59e85ed6ea))
- *(metrics)* Process memory telemetry — RSS/VSZ/peak + facts-written([876bbdd](https://github.com/scbrown/quipu/commit/876bbdd259445ab85d93e99f957a3624e091e411))
- *(shapes)* Aegis:CrewRole + CrewMember.hasRole — role becomes a trait-set([b7ff07a](https://github.com/scbrown/quipu/commit/b7ff07a714350fb4b30d12e033602f80c00ded86))
- *(shapes)* TraitValue precedence — resolves the composable-role single-valued axes([c1dc430](https://github.com/scbrown/quipu/commit/c1dc430699a90865ae734a71d8700b352cb09053))
- *(shapes)* Aegis:ExternalService — consumed third-party dependencies([e6bc8ad](https://github.com/scbrown/quipu/commit/e6bc8adf9d449896e70f10ad585d91f16728ae26))
- *(shapes)* AlertRule carries alertExpr, severity, keeper, guardsProducer([d603b79](https://github.com/scbrown/quipu/commit/d603b794dfeed0e348337ae0da366a17820981bf))

### Fixed

- *(vector)* Brute-force search no longer loads ALL entity text into RAM([a2309cf](https://github.com/scbrown/quipu/commit/a2309cfd756a55c0e5500076be062bbaa0f09486))
- *(episode)* Emit one triple per array element instead of silently dropping it([74be82c](https://github.com/scbrown/quipu/commit/74be82c882c6902597d742c6bde7b502478e6c4a))

### Perf

- *(shacl)* Cache the parsed shapes graph — validation was parsing 86KB twice per write([face36e](https://github.com/scbrown/quipu/commit/face36e3889c7d754b28ca8e58788150b1aae8a1))

## [0.3.11] - 2026-07-23

### Miscellaneous

- Update Cargo.toml dependencies([0000000](https://github.com/scbrown/quipu/commit/0000000))

## [0.3.10] - 2026-07-23

### Added

- *(quipu_ask)* Entity-centric provenance queries entity_work + cochanged_with (quipu#37)([f47fc31](https://github.com/scbrown/quipu/commit/f47fc313e02a84c723240e0d6e5fa8c2466280ca))

## [0.3.9] - 2026-07-23

### Added

- *(events)* P2 push delivery — subscription registry + webhook worker (realtime + batched)([5b79b8a](https://github.com/scbrown/quipu/commit/5b79b8ae2ec794d9437ff578019002a8d0da6072))
- *(ui)* Node page + first-class IRI + graph-driven deep-links for every node([8d1e2dd](https://github.com/scbrown/quipu/commit/8d1e2dd5bbcd4f9d69bed1b8985ff0060a306cbf))
- *(store)* /set — atomic single-call replace/supersede for a predicate([2df3e2f](https://github.com/scbrown/quipu/commit/2df3e2fe1ba51f676ae9bf77e63efa3b87c85301))
- *(shapes)* I5/I6/I7 — vocabulary drift gate, both directions + live census([2399786](https://github.com/scbrown/quipu/commit/23997862756dd71c6cbcc60f0cdf78c007c52c8e))
- *(shapes)* Backend must be a concrete address — first VALUE-QUALITY constraint([0ee60b7](https://github.com/scbrown/quipu/commit/0ee60b70face3f9bf8ab8e720aaad3d512cfb7b2))
- *(shapes)* Declare the Commit kind — hundreds of promotion-emitted entities were unvalidated([f740025](https://github.com/scbrown/quipu/commit/f740025985276d0548d6428f807d7f645b3ae6ea))
- *(sparql)* Evaluation budget enforced INSIDE join/merge loops + intermediate row cap([400da45](https://github.com/scbrown/quipu/commit/400da4537ef6e6177a31735b6fde640f6ea0d5b7))
- *(shapes)* CrewStatus convention (retired/never-instantiated) + ResponsibilityDomain class([323d7b2](https://github.com/scbrown/quipu/commit/323d7b21fcb1330e351fc22ecd3bc05674132d12))
- *(server)* Read-only /resolve route — resolution dry-run without writing([4b43896](https://github.com/scbrown/quipu/commit/4b438969fa24442ac248ef068b3450f37644e2c5))
- *(sparql)* Evaluate FILTER (NOT) EXISTS, incl. property paths inside([57a100a](https://github.com/scbrown/quipu/commit/57a100af9b2fe1903417abb99e8a60edd3ddfc50))

### Fixed

- *(server)* Clone the store handle before the router consumes it — shacl,onnx build was broken by the push worker([ea15b18](https://github.com/scbrown/quipu/commit/ea15b18571197a7f9eac046190aada36050318e9))
- *(ui)* Node page renders immediately — incoming edges patch in async (object-bound scan is ~30s on a large store, LIMIT 100 bounded, slice-indexed when loaded)([9d2a062](https://github.com/scbrown/quipu/commit/9d2a062bb6851b013e7a238d7ce787bfdd3c38df))
- *(ui)* Boot the node page BEFORE the bulk entity load — exact-IRI queries otherwise queue behind /cord on the store mutex([09f205b](https://github.com/scbrown/quipu/commit/09f205b1e3b71e724de9a5a24a28ced9c8298d81))
- *(ui)* Load the deep-link registry with the node page, not after it([db103a8](https://github.com/scbrown/quipu/commit/db103a828ea716e7614e49048cf917067179a657))
- *(ui)* A failed deep-link-registry load retries instead of latching empty for the session([6c78b76](https://github.com/scbrown/quipu/commit/6c78b766fa7100d48b6dc2c7d9972fcebc2eed37))
- *(ui)* Node-page fetch phase gets a deadline + one retry — a request fired mid-restart hangs a timeout-less fetch forever([a6f04d7](https://github.com/scbrown/quipu/commit/a6f04d7e3e8f96b56290c1f49582941bd38fd6c6))
- *(ui)* Hash-set at init re-entered navigate() and fired /cord in parallel with the node page's queries — the store mutex then starves them; track the boot promise and defer the bulk load, and popstate no longer re-opens the same page([b190dfe](https://github.com/scbrown/quipu/commit/b190dfe58561682029b6af58f1247cc3b7201c62))
- *(ui)* Keep the entity graph readable now that the code plane is 75% of the store([f1414b8](https://github.com/scbrown/quipu/commit/f1414b816fee8b815babd04bde8dcdca76dd4d30))
- *(ui)* Cap 1-hop expansion — a modest type filter can pull thousands of neighbours([c59f37f](https://github.com/scbrown/quipu/commit/c59f37ffe5cca541c6021b30c9c37f50091bced6))
- *(set)* Scope the bare-string guard to ref-only/empty predicates; {"str"} states literal intent([d099019](https://github.com/scbrown/quipu/commit/d099019f36de2f3daad73d4a814d74a8b6187946))
- *(server)* Spotlight scan runs OUTSIDE the store lock — reader starvation fix([ea97a95](https://github.com/scbrown/quipu/commit/ea97a954fc92dc954732fcbeef10ee7401eaec23))
- *(server)* Cache the spotlight entity list keyed on store generation([e9ecfaa](https://github.com/scbrown/quipu/commit/e9ecfaaf276d9ad6f8071ffffaba9fbfff21a605))
- *(server)* Episode auto-embed runs OUTSIDE the store lock + fair mutex([f67c7d6](https://github.com/scbrown/quipu/commit/f67c7d6ff0ab244b600e446ad96e3ff79414bac2))

### Miscellaneous

- *(clippy)* Satisfy the new stable clippy (1.96) lints — unbreak CI([65f3965](https://github.com/scbrown/quipu/commit/65f39654c6611fe7b8270c7a5278d4de609550c7))
- Normalize trailing newlines in aegis-ontology.shapes.ttl (end-of-file-fixer)([129c2fe](https://github.com/scbrown/quipu/commit/129c2feb374e37e6b59bb7c899969f6a75d1b13a))
- *(clippy)* Backtick inner_eval in eval_pattern_seeded doc — unbreak CI([ab126ca](https://github.com/scbrown/quipu/commit/ab126caf268008f67a170e90efb461487aa5d0c6))

### Perf

- *(sparql)* Seed EXISTS inner eval with the outer row — no per-row full re-eval([17dfed8](https://github.com/scbrown/quipu/commit/17dfed8122b516428e035525eba3d99d0a4b720e))
- *(server)* Cache /stats behind a generation key so polling is O(1)([406c0a2](https://github.com/scbrown/quipu/commit/406c0a29e593c06d9134728c859a9a904644c1d6))

### Style

- Rustfmt drift in mcp/tools.rs (pre-existing; found by cargo fmt --check)([2dddf34](https://github.com/scbrown/quipu/commit/2dddf3497a236581b7679bc23a91ec20f20f7599))

## [0.3.8] - 2026-07-23

### Added

- *(events)* P1 event log + pull API + consumer registry([a81b3e7](https://github.com/scbrown/quipu/commit/a81b3e763070d095150e33070a0a6d3f757bc5ad))
- *(server)* Prometheus /metrics — request counts, per-endpoint latency, policy outcomes, graph gauges([415de52](https://github.com/scbrown/quipu/commit/415de52b72c15e06c8801ad0601b470307908be3))
- *(shacl)* Quipu:onViolation emit|reject — violations as events without gating the write (event P3)([d2a8d69](https://github.com/scbrown/quipu/commit/d2a8d690be1f5ee6ac75e008a32512fee5057327))
- *(shapes)* Declare Metric — scraped series are first-class nodes (schema-gate ruling)([994064c](https://github.com/scbrown/quipu/commit/994064c12f4eb563416ebc4f63836bbb00a3e689))
- *(shapes)* Wire the relationship constraints — 15 targeted, 4 held on measured blockers([d6f41d9](https://github.com/scbrown/quipu/commit/d6f41d98fdc0bea4183066c91d71cf0c47e1bb40))
- *(shapes)* Vt9m — 23 legacy classes legitimized, class hierarchy declared, synonym classes retired([8e66b22](https://github.com/scbrown/quipu/commit/8e66b226d2045c1d4bd44a32e77afb71fb7792d4))

### Fixed

- *(server)* Raise the request body limit to 64MB — axum's 2MB default 413s real code-graph promotions([d1025bd](https://github.com/scbrown/quipu/commit/d1025bd96cd55bab8307ba80c0557af9c38ebc71))
- *(shapes)* Target ContainsShape so provenance can actually fire([5da7d5f](https://github.com/scbrown/quipu/commit/5da7d5f205b5aaa6f6302aec89fd8794ad65af2e))

### Testing

- *(events)* Prove retraction EMITS — the reported P1 gap did not exist([a2774fb](https://github.com/scbrown/quipu/commit/a2774fb5bf50c8ed207f88f7cabffa678f58b4a6))

## [0.3.7] - 2026-07-23

### Added

- *(sparql)* Query wall-clock budget enforced inside evaluation + request logging([e002a07](https://github.com/scbrown/quipu/commit/e002a07c06c95173f25ab01595105b457d83f112))

## [0.3.6] - 2026-07-23

### Added

- *(governance)* Enforce action-boundary policies on the write path([a0bddd7](https://github.com/scbrown/quipu/commit/a0bddd7d23329d092e31a4a3f9fdcfd56d62f8df))
- *(governance)* Fail closed on require-approval/escalate at the write gate([ea5f667](https://github.com/scbrown/quipu/commit/ea5f667068cb98deeb7ed9b99baf79927f1ff0a2))
- Transactions cursor + Policy assignsWorkflow([84de69f](https://github.com/scbrown/quipu/commit/84de69fd0e651c62e8810077d04faecfe94afab5))
- Tree-sitter-tier policy catalog + Selector/Predicate congruence([356c1c2](https://github.com/scbrown/quipu/commit/356c1c20327cd2b08462ba49308e1b9ca81545ec))
- Aegis:language on Selector for tree-sitter policy projection([2b2ed29](https://github.com/scbrown/quipu/commit/2b2ed29492a7fbe23beddd751b29b22fc2589585))
- *(ui)* Default graph shows named entities + relations, not episodes([9a1ccc6](https://github.com/scbrown/quipu/commit/9a1ccc658278122e22f6ac6354533bf902334801))

### Changed

- *(governance)* Split guard tests into guard_tests.rs([30d87c2](https://github.com/scbrown/quipu/commit/30d87c22d77274fbef02870d12dd84f547d8b22e))

### Documentation

- *(retract)* Document on_orphan and the identity-accounting response; test refuse at the tool level([a22530b](https://github.com/scbrown/quipu/commit/a22530bff1d142f63104521f557685e9109d07c5))
- Correct 6 reference-doc drifts and remove 3 dead pub items; pin the tool count([8e02316](https://github.com/scbrown/quipu/commit/8e02316234719d495f73bd3505aa35deaaac8bbe))
- Note where an operator registers hank's verdict-signing publicKey([305d824](https://github.com/scbrown/quipu/commit/305d824b0d727a5dc00f966b230343da6f55cf00))

### Fixed

- *(reason)* --reactive must refuse loudly without its feature, and the feature must compile([0ba069d](https://github.com/scbrown/quipu/commit/0ba069de15a85b234b9c9abe8cb89db528cd5cfc))
- *(retract)* A bare-string object for an IRI edge is refused, not a silent no-op([466634c](https://github.com/scbrown/quipu/commit/466634c8b47c108a767a45d73fe86fcd096e6770))
- *(retract)* A bare IRI-shaped string errors even when there is no fact to compare, and document the {iri} contract([9a25f16](https://github.com/scbrown/quipu/commit/9a25f1659306447d91bb2f3c3e76a855a7522056))

### Miscellaneous

- Bring repo into compliance with current stable rustfmt + clippy([bb8f0e0](https://github.com/scbrown/quipu/commit/bb8f0e0546020142d08e7d92561fad8b9343f9c3))
- *(docs)* Blank lines around headings/fences in homelab example([8cac704](https://github.com/scbrown/quipu/commit/8cac70495d6ce3852c4fe8f1bea8d30dd8660c23))

### Perf

- *(search)* Scoped /search drives from the selective pattern, and every store handler runs off the reactor([694d570](https://github.com/scbrown/quipu/commit/694d570ee49e71897bcfc57ec987860e5a233f74))
- *(search)* Embed the query OUTSIDE the store lock so concurrent searches do not serialize([ffa75a6](https://github.com/scbrown/quipu/commit/ffa75a6049e7b18443e188b26e5cd7a76770e51e))

## [0.3.5] - 2026-07-20

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
- *(episode)* Add SHACL validation gate, batch ingestion, provenance query([9f70a0c](https://github.com/scbrown/quipu/commit/9f70a0cac558319c6c42a52f75a9f81c3192458c))
- *(sparql)* Add temporal SPARQL queries (valid_at + as_of_tx)([46db89f](https://github.com/scbrown/quipu/commit/46db89f743ab39859d1130f58546da21762e297b))
- *(server)* Add axum REST API server mirroring MCP tool surface([a9eb8fa](https://github.com/scbrown/quipu/commit/a9eb8facdbcd17e6dc34c5d562c9cd8c2ef7ce61))
- *(graph)* Add petgraph projection API with algorithms([d270132](https://github.com/scbrown/quipu/commit/d270132789ad4e277e7087b5d8df680b78e9924a))
- *(provider)* Add GraphProvider trait for federated queries([0842816](https://github.com/scbrown/quipu/commit/0842816e9694857a8edd7d37dfbc2d35e687ce8e))
- *(config)* Add QuipuConfig with .bobbin/config.toml support([c13baf2](https://github.com/scbrown/quipu/commit/c13baf282be414ff5005bdd76c6a48e83b000300))
- *(context)* Add unified context pipeline for knowledge-code blending([815e640](https://github.com/scbrown/quipu/commit/815e64010334c091b40193b6910d5cf098a543e5))
- *(sparql)* ASK/CONSTRUCT/DESCRIBE + module splits + pre-commit hook([8102262](https://github.com/scbrown/quipu/commit/8102262df4d2e19d18b27ac3504857d4dad0013c))
- *(vector)* Add hybrid SPARQL + vector search tool([ff46399](https://github.com/scbrown/quipu/commit/ff46399277233e1690ca101b145dc4128f4e312b))
- Add KnowledgeVectorStore trait + LanceDB backend (.1)([ea669c9](https://github.com/scbrown/quipu/commit/ea669c90f3a8a18b212771f79ebccfbd11a344df))
- Add release-plz workflow with changelog generation (.2)([01b7808](https://github.com/scbrown/quipu/commit/01b7808daae97b1cc6330077147401c97f8e96ce))
- Add LanceDB hybrid search with predicate pushdown (.2)([bb86cb6](https://github.com/scbrown/quipu/commit/bb86cb63527b027136ce81c203def6a32383c90c))
- Crates.io publishing prep — metadata, exclude, missing_docs lint (.3)([84bf8b0](https://github.com/scbrown/quipu/commit/84bf8b0d6ea4fad1e8a5d7d6ef1eb43d5dc132a0))
- Add auto-embed entities on write (knot/episode hooks) (.4)([126b7ea](https://github.com/scbrown/quipu/commit/126b7eabcbddf63afcc7f068dfa56ff796d894bf))
- Unified search results with source tagging for Bobbin integration (.3)([f1be2e0](https://github.com/scbrown/quipu/commit/f1be2e0b4b1044737ff63bf1f35488e188730a9a))
- LanceDB full-text search for context pipeline (.5)([bebda6f](https://github.com/scbrown/quipu/commit/bebda6f3570e294fdd8a1d813f75e939aabeab23))
- SHACL shapes for code entities (CodeModule, CodeSymbol, Document, Section, Bundle) (.1)([182dfa7](https://github.com/scbrown/quipu/commit/182dfa78d199d1544c2cb1ed5a084ac54106c8fa))
- Register bobbin: namespace and code entity IRI patterns (.2)([dee600c](https://github.com/scbrown/quipu/commit/dee600c5d58eb68cd39e56a1ffe21cdd6f9d0b56))
- Delegate vector search to external provider when knowledge feature enabled (.3)([2fe48a7](https://github.com/scbrown/quipu/commit/2fe48a77ceaf309fece21e3ed8c4ea87048f75ff))
- Post-index reconciliation to resolve cross-repo import edges (.4)([a3b148d](https://github.com/scbrown/quipu/commit/a3b148d7d80ce40e99bace5bb3dcb7e180c7fd0f))
- Auto-embed query text in search endpoints via EmbeddingProvider([95e18ee](https://github.com/scbrown/quipu/commit/95e18ee42d0ad96caf33accddb3d759355541263))
- Add quipu_search_nodes and quipu_search_facts MCP tools([3146322](https://github.com/scbrown/quipu/commit/31463221334debb8f3b679d93df3951d05e62c2c))
- Graphiti-compatible REST endpoints /search/nodes and /episodes/complete([daef471](https://github.com/scbrown/quipu/commit/daef47120c55009d1e60d5c68acf67aa04cc03e1))
- Migrate SQLite vectors to LanceDB + dual-mode config (.3)([455a8e8](https://github.com/scbrown/quipu/commit/455a8e8992f8f4a4d0fee99e8116891508fe5a26))
- *(shacl)* Add aegis ontology SHACL shapes([da19a7b](https://github.com/scbrown/quipu/commit/da19a7b332e2b336b1b6f10c4bb604bb6a7a5de1))
- *(sparql)* Implement SPARQL 1.1 property paths([280ac51](https://github.com/scbrown/quipu/commit/280ac517a1295bf9d1ca3c215cd8854210c221f4))
- *(ui)* Add standalone web UI with graph explorer and SPARQL workbench([32cf2ae](https://github.com/scbrown/quipu/commit/32cf2ae1232e8e8294022213ce05e10b8c814326))
- *(ui)* Add Leptos WASM scaffold with Sigma.js graph explorer([4a0aa62](https://github.com/scbrown/quipu/commit/4a0aa623cd847c565ff084bf8c8885a6069c7058))
- *(shacl)* Complete aegis ontology shapes — add missing types and fix constraints([603cc97](https://github.com/scbrown/quipu/commit/603cc97bf78519c55a92220f45b19ad098261ef7))
- *(server)* Add ONNX embedding provider for standalone quipu-server([ad275f1](https://github.com/scbrown/quipu/commit/ad275f15483123643a804ded0626c1d579142247))
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
- Wire schema proposal MCP tools into REST server + add round-trip tests([2d227c7](https://github.com/scbrown/quipu/commit/2d227c71285e46fe26b9df84f300a804c434a972))
- *(owl)* OWL 2 RL ontology layer — class hierarchy, materialization, and validation([ecc6354](https://github.com/scbrown/quipu/commit/ecc6354dd1a8c8503c9d41a9db69786a30ecbfbf))
- *(graph)* PageRank & Personalized PageRank over ProjectedGraph (#11)([7f984b4](https://github.com/scbrown/quipu/commit/7f984b4e568c994b499e3cca7815139b90e0491a))
- *(server)* Add CORS headers for cross-origin API access (GH#5)([b567b77](https://github.com/scbrown/quipu/commit/b567b770c54003bee1e0c2948f519848d2859a4c))
- *(resolution)* Wire entity resolution into episode ingest (#15)([7dbdba1](https://github.com/scbrown/quipu/commit/7dbdba14a52adefabe794535500d918f60bc783b))
- *(search)* Clamp limits + max_limit config + SPARQL row ceiling (#16)([421a3b1](https://github.com/scbrown/quipu/commit/421a3b1982feba4558c8669b7142a653dc4022b4))
- *(shacl)* Auto-apply loaded shapes on episode ingest (#19)([5a88d8d](https://github.com/scbrown/quipu/commit/5a88d8dd46d5c64b183998334a6cf78ced47ff75))
- *(server)* Bearer auth + read-only mode + CORS allowlist (#20)([2d0f48e](https://github.com/scbrown/quipu/commit/2d0f48ef539b4abde54bf78a5d46e48ac3e2fb19))
- *(mcp)* Named-query catalog + quipu_ask tool / /ask endpoint (#22)([61b8cf9](https://github.com/scbrown/quipu/commit/61b8cf9b3d20c9770512edd6593f65e93498ba46))
- *(episode)* Idempotent ingest via content-hash key (#24)([03c9d44](https://github.com/scbrown/quipu/commit/03c9d44a509c8b78631e22d4ee217606b363f855))
- *(episode)* Per-edge confidence qualifier on ingest (#29)([622bd13](https://github.com/scbrown/quipu/commit/622bd13dfec533a47a756cab4c0ec2a1a7c26247))
- *(graph)* Deterministic Louvain community detection (#31)([62fe4fa](https://github.com/scbrown/quipu/commit/62fe4fa24d95cffbeeb99bc85c26b8132f4c3487))
- *(report)* Live graph report endpoint + quipu_report MCP tool (#32)([746dc06](https://github.com/scbrown/quipu/commit/746dc06fa52092a0259fd3b881e9ebdaecda0293))
- *(episode)* Episode-scoped logical retraction endpoint (#33)([804b750](https://github.com/scbrown/quipu/commit/804b750db56f1b19336223a1137656dffc3dccd1))
- *(cli)* --base-ns and --timestamp flags on knot/episode/retract (#28, #27) (#34)([3230d4f](https://github.com/scbrown/quipu/commit/3230d4f5238d6ccd5bf7049549c04545772c0a14))
- *(graph)* Export project() — the typed graph API was unusable from outside([1143b7b](https://github.com/scbrown/quipu/commit/1143b7be703b15d19b843abb4204115b3c5f1903))
- *(store)* Named-graph column on facts — additive ROOT-default foundation (quipu #36)([b57bfab](https://github.com/scbrown/quipu/commit/b57bfab30eb779d2778889b3635dc2949ada6766))
- *(store)* Graph-scoped writes — overlays extend ROOT without mutating it (quipu #36)([aca75f3](https://github.com/scbrown/quipu/commit/aca75f3c74febf580acfd4a30a93bcc9c254cf53))
- *(episode)* /episode `graph` field — write knowledge into a named overlay (quipu #36)([8196a08](https://github.com/scbrown/quipu/commit/8196a084b21608028ef72437c9d78fbcb640cf33))
- *(query)* Content-negotiated W3C SPARQL 1.1 results — fix lossy shape([8e5ea77](https://github.com/scbrown/quipu/commit/8e5ea77d48589f1633467534cc441bb658c96ea5))
- *(store)* Named-graph overlay primitives — create / write / tombstone / compose (#36/#37)([bf1ecd1](https://github.com/scbrown/quipu/commit/bf1ecd13e0e409754c772a81f8ab22b85f1e70f3))
- *(shapes)* Governance-plane ("the loom") ontology + SHACL shapes (Phase 1)([325630c](https://github.com/scbrown/quipu/commit/325630c1ea1b3aabd05feaaaac4b13f6cbebb0b7))
- *(mcp)* Provenance-based work-item co-occurrence — /cooccurrence (quipu#37)([edc845d](https://github.com/scbrown/quipu/commit/edc845de025ca98887fb36b0336e53076d636a29))
- *(mcp)* Committed-tier policy evaluation — /policy/check (the loom, Phase 1 runtime)([02eff07](https://github.com/scbrown/quipu/commit/02eff07ee413d5f3ae3fc21c577303224be996d3))
- *(mcp)* Verifier registry — the Phase-0 authority layer (the loom)([c41ddcb](https://github.com/scbrown/quipu/commit/c41ddcb2b439b23bef91592774fd5beb9d23d84d))
- *(signing)* V1 verdict signing — the loom's Phase-0 root of trust([2336fc7](https://github.com/scbrown/quipu/commit/2336fc7031be230d3e0927cf5851c0ec130ea64d))
- *(retract)* Triple-level retraction — entity + predicate + value([758cf51](https://github.com/scbrown/quipu/commit/758cf5171482cde144466c561685ffb7c9196c88))
- *(shapes)* AnsibleGroup + TerraformResource node shapes([184aef4](https://github.com/scbrown/quipu/commit/184aef4197156d02e34e335e9c23094ea631dcb8))
- *(shapes)* Declare Incident, FailurePattern, SoftwareVersion, StoragePool([25e47a7](https://github.com/scbrown/quipu/commit/25e47a7e2b19274ea1b1c6cb4d0750c1c621d7ac))
- *(shapes)* Declare ClaudeCodeHook; aegis:Guard deliberately NOT declared([b7b31ad](https://github.com/scbrown/quipu/commit/b7b31ad449f5f60cebd7385fe670bb41a0715df7))
- *(server)* GET /version — the git SHA of the running build([6ee8412](https://github.com/scbrown/quipu/commit/6ee84129e2d72ca7d6f2763ea67e3a84e20a1ca8))

### CI/CD

- Split Rust CI into fmt, clippy, test, build jobs with caching (.1)([c05d534](https://github.com/scbrown/quipu/commit/c05d534ddd20bc21fb73844b616ddfb034f255dc))

### Changed

- Extract hardcoded namespace strings to constants module([868d63a](https://github.com/scbrown/quipu/commit/868d63a2ea8891b954939bfd27139cfcd62d4920))

### Documentation

- Comprehensive mdbook update — all features documented([fde105b](https://github.com/scbrown/quipu/commit/fde105b0a936c5d91197b1af37367405f592b4c7))
- Refresh README with feature showcase and comparison table([bc25708](https://github.com/scbrown/quipu/commit/bc25708783711112486affd4b01c2c4abdf52efb))
- Enrich CLAUDE.md + CONTRIBUTING.md for polecat agents([05b6148](https://github.com/scbrown/quipu/commit/05b61483ec1f47cdfac3310134aff3f37ac40fb4))
- Add quipu SVG logo and polish README with emoji([3bc4fa4](https://github.com/scbrown/quipu/commit/3bc4fa4f447d791994e75ce449ed1ef140c2e44b))
- Update mdbook + README with LanceDB, CI/CD, and Bobbin integration([baf1154](https://github.com/scbrown/quipu/commit/baf115451a35617880c9bf526fb19ea531aa7af3))
- Add v0.1.0 CHANGELOG and feature matrix to README([e8d8d2e](https://github.com/scbrown/quipu/commit/e8d8d2e60c1db4ea69715b222d1936abfa7981da))
- *(lint)* Fix pre-existing markdown lint violations([093c4b3](https://github.com/scbrown/quipu/commit/093c4b32f9e98f76a59aebf491a754b9c2db80a3))
- Reconcile quipu docs with implementation([3f1b37a](https://github.com/scbrown/quipu/commit/3f1b37abee2e5b43722993d1fbb39232173ea3f9))
- The documented build must produce quipu-server — name --features onnx([1da8a09](https://github.com/scbrown/quipu/commit/1da8a090a21a60ae7811b7ba3a696476feffed0e))

### Fixed

- *(store)* Retract closes original assertion's valid_to([98f0072](https://github.com/scbrown/quipu/commit/98f00724f8a6d2444619e332be95fb856a7e472b))
- Resolve rebase conflicts + clippy fixes in provider/context([7d7e423](https://github.com/scbrown/quipu/commit/7d7e423ee6389b8a28677e32e83576fe08dac075))
- *(ci)* Allow docs deploy on workflow_dispatch, not just push([bf1aaa4](https://github.com/scbrown/quipu/commit/bf1aaa4394170b088ff9f1fb0810c4deaf4ce7c9))
- Handle feature-gated Triple variants in match arms (.1)([a476f2a](https://github.com/scbrown/quipu/commit/a476f2a7bd5fb3dbe0603ff5234053bf46823ad1))
- Resolve clippy and markdown lint failures([e231399](https://github.com/scbrown/quipu/commit/e2313998261fb015719f9bbdb9ec2858436800af))
- Remove quipu-ui build artifacts from tracking([6fed42d](https://github.com/scbrown/quipu/commit/6fed42db549f46cc48b9f98815f009210baca037))
- Auto-save uncommitted implementation work (gt-pvx safety net)([1be5043](https://github.com/scbrown/quipu/commit/1be50438489fb3c9e39cddea50ae8194c4279ed4))
- Auto-save uncommitted implementation work (gt-pvx safety net)([df3e7e1](https://github.com/scbrown/quipu/commit/df3e7e1938f44b29b932683809bf22ac49567759))
- *(server)* Remove suffix entity routes that panic on axum 0.8+([583de29](https://github.com/scbrown/quipu/commit/583de292adff4ae5eb4b3a2eefcd267258e984e1))
- *(server)* Use sub-path routes for entity format endpoints([4d80832](https://github.com/scbrown/quipu/commit/4d80832ee758e8ecc3b2d4c1e34bf61b2fcddeca))
- Idempotent assertions — deduplicate (e,a,v) facts across transactions([afe3dda](https://github.com/scbrown/quipu/commit/afe3ddad4685140ff6bd8409de100c2c7b1a3986))
- Close pre-commit CI parity gaps (#4)([b280007](https://github.com/scbrown/quipu/commit/b280007607366d55261f37febe640428ceb0cb40))
- *(ui)* Force-directed graph, working SPARQL editor, richer timeline (GH#6,#8,#9)([d1192c2](https://github.com/scbrown/quipu/commit/d1192c2b0f1d53f7ffa5db470c3fd05ce7e9b773))
- *(sparql)* Implement FILTER builtins — CONTAINS/STR/LCASE/isIRI no longer no-op (GH#12)([17ac2eb](https://github.com/scbrown/quipu/commit/17ac2ebc3adee8547007f9a914da6851d9c91416))
- *(sparql)* DISTINCT current-fact rows in BGP — stop COUNT/OPTIONAL inflation (GH#13)([ca091fa](https://github.com/scbrown/quipu/commit/ca091fae507ce705af1c39162065101ce34e3627))
- *(sparql)* Real FILTER REGEX + fail loud on unsupported builtins([5f47ae6](https://github.com/scbrown/quipu/commit/5f47ae659cb115dfe3bb8a3d1f8b812dd5a156dc))
- *(mcp)* Register quipu_project + quipu_context in tool_definitions([beaaebf](https://github.com/scbrown/quipu/commit/beaaebfa878d3c5d4bff3e9397046ac363362ed1))
- *(mcp)* Gate quipu_load_ontology behind owl feature; matrix not 'Planned'([b206053](https://github.com/scbrown/quipu/commit/b2060539736ffa26edd6091e02b54fe7aa4f3886))
- *(time)* Stamp write paths with real clock, not 1970 epoch (#14)([04becf1](https://github.com/scbrown/quipu/commit/04becf17ebd249be321a03b593d94715d3b11865))
- *(search)* Tool_search honors group_ids + entity_type (#17)([2d8d35c](https://github.com/scbrown/quipu/commit/2d8d35c2e91e6485d19fdb6fdadee08e31a7bea9))
- *(embedding)* Cap ONNX tokenizer length + fail loud on dim mismatch (#18)([3f2aed9](https://github.com/scbrown/quipu/commit/3f2aed9b27163ec6e413b5cd89fc2625ef768536))
- *(search)* Rank query-named entity above referencing nodes (#21)([114a492](https://github.com/scbrown/quipu/commit/114a492b9f3606edae75b0bc3682cf8b3bb88e05))
- *(embedding)* Deterministic re-embed on label/comment change (#25)([68e2036](https://github.com/scbrown/quipu/commit/68e203644cb1829adb20685c49853937e8c1cb42))
- *(onnx)* Bind embedder inputs by name, not positionally([cafb2f7](https://github.com/scbrown/quipu/commit/cafb2f75375e5dffcd424d7f55f0389d6f532217))
- *(onnx)* Use real attention mask — pad tokens collapsed embeddings([cb7e620](https://github.com/scbrown/quipu/commit/cb7e62080bc6feabeb09c08053da25eb867f8112))
- *(shapes)* Reconcile SHACL shapes to what /episode actually emits([3d2cad3](https://github.com/scbrown/quipu/commit/3d2cad3dc0d0348a516db004075920391c50bd23))
- *(sparql)* IsBlank() was aliased to isIRI() — it matched the whole store([f0c70b7](https://github.com/scbrown/quipu/commit/f0c70b7fbf5b2cef674fbfe3e8488cb437c92cd3))
- *(episode)* Reject untyped nodes with a clear error, not a whole-episode Turtle 400([3769fa9](https://github.com/scbrown/quipu/commit/3769fa987cbc548d4b8205a42243ba4f84cff0b6))
- *(store)* Create idx_geav in the named-graph migration, not INIT_SQL([ae75e80](https://github.com/scbrown/quipu/commit/ae75e80e50fc9068d3cef89788895648e59f3d10))
- *(overlay)* Dedupe compose_view over re-asserted base facts (found in 69co live deploy)([e1288fe](https://github.com/scbrown/quipu/commit/e1288fe376c7b323cbeb9118a196530eebf7518f))
- *(cli)* --version/--help are pure reads, never open a store([a87ce9f](https://github.com/scbrown/quipu/commit/a87ce9f09a13fde6137150038a5e452bbab78c82))
- *(search)* Dedupe /search results by entity, keep best-scoring row([4f1c506](https://github.com/scbrown/quipu/commit/4f1c50677fa3fafae322a88c78cda7ccb95f3798))
- *(retract)* Episode retraction no longer orphans node identity([bfe7948](https://github.com/scbrown/quipu/commit/bfe7948d087affdd9448d026138ed5a3bb72e637))
- *(rdf)* Preserve language tags and datatypes in the Value model([f4d49df](https://github.com/scbrown/quipu/commit/f4d49dffa3e11ab4cc350bdef4c6ebd5bf447fad))
- *(shapes)* Add `get` (content read-back) and reject unknown actions([eb319be](https://github.com/scbrown/quipu/commit/eb319be56129be7677b77e02e0783b56f691ed87))
- *(retract)* One retraction datum per triple, not per backing row([26bd04b](https://github.com/scbrown/quipu/commit/26bd04bec37ff5af025d445d3f28c0aea2b4663a))
- *(retract)* Two-type coverage + refuse orphaning an entity's last rdf:type([0bae616](https://github.com/scbrown/quipu/commit/0bae6168f585e91187f957194f01d653693633ed))
- Scrub internal identifiers to zero, untrack the runtime store, add the RATCHET([258c6d7](https://github.com/scbrown/quipu/commit/258c6d7744cc5da2b585790a252b496666233473))
- *(auth)* Close 3 write routes that bypassed read-only + bearer auth, and enforce the list([7604448](https://github.com/scbrown/quipu/commit/7604448232809e8e7ea3b5379269c1bbfc2269b0))
- *(config)* Actually mint IRIs under the configured base_ns([7d54b10](https://github.com/scbrown/quipu/commit/7d54b105723324fbacef259519be6c29574f6739))
- *(group_ids)* Make code, doc, test and schema agree — provenance, not isolation([3b1762a](https://github.com/scbrown/quipu/commit/3b1762a453f5b46530639b08d109d0403a8638e8))
- *(server)* Re-tier 5 writing ro_handler! routes as rw, and enforce tier==classification([51a1436](https://github.com/scbrown/quipu/commit/51a1436bcd814bd75774cccc5ec9549338c01df1))
- *(config)* Make federation + the LanceDB backend honest, not silently inert([19730be](https://github.com/scbrown/quipu/commit/19730bea32474b726d0c421a4d431531c1be8bd7))

### Miscellaneous

- Downgrade rusqlite to 0.33 for Bobbin compatibility([7a5f2b2](https://github.com/scbrown/quipu/commit/7a5f2b2d95c2b73629021f699764c7f6e6f03ab4))
- Update Cargo.lock for lancedb deps (.1)([026acb8](https://github.com/scbrown/quipu/commit/026acb80bc73f56dba5ac30d694513eb71f25070))
- Fix trailing newlines and markdown lint errors (.3)([38cf31a](https://github.com/scbrown/quipu/commit/38cf31a2cd8ddc7556432c7d745ed05b73f31b10))
- *(release)* Quipu 0.3.0([7ca9d59](https://github.com/scbrown/quipu/commit/7ca9d59a2e69885cc9ca3c017671a169f6a78490))
- *(release)* Quipu 0.3.1([f358ca3](https://github.com/scbrown/quipu/commit/f358ca3e3e294449996b4e5c1672ba543d6e301f))
- *(beads)* Untrack host-local .beads/redirect (was broken ../../../.beads)([892b1f7](https://github.com/scbrown/quipu/commit/892b1f7b3c2b245d9e473daec65da4d077fd960b))
- Bump version 0.3.1 -> 0.3.2 for release (#30)([36b1db2](https://github.com/scbrown/quipu/commit/36b1db282f864b5861fa474f64e90dc7b74fc6a1))
- Release 0.3.3([dc1724e](https://github.com/scbrown/quipu/commit/dc1724e366f2a7e7fc6fbfb62322917630727a83))
- Release v0.3.4([37fad37](https://github.com/scbrown/quipu/commit/37fad37cc6b0e20e1338a010bbd864ddf21627c2))

### Testing

- *(sparql)* Add HAVING, COUNT(*) empty, GROUP BY+SUM tests([4d0434c](https://github.com/scbrown/quipu/commit/4d0434c10682efd39b11a0918c0c5a3a6908b51f))
- *(shapes)* Guard the SHACL shape invariants against drift([038c2f3](https://github.com/scbrown/quipu/commit/038c2f314024ca10f8f0309e63a5795fe0521ac9))
- *(retract)* Lang/typed literals stay precisely retractable([0579895](https://github.com/scbrown/quipu/commit/057989520b78893191aa4d7145d34821cb0f8c49))

### Merge

- Integrate entity resolution from polecat/fury-entity-resolution([6717a87](https://github.com/scbrown/quipu/commit/6717a87502ae838146a24806f5489fc7288b478e))

### Release

- V0.2.0 — reasoner engine + web UI enhancements([7527ad4](https://github.com/scbrown/quipu/commit/7527ad4eab2a36923c24e8c24cad7d3b3bc6e4b9))

## [0.3.4] - 2026-07-20

### Added

- *(graph)* Export project() — the typed graph API was unusable from outside([1143b7b](https://github.com/scbrown/quipu/commit/1143b7be703b15d19b843abb4204115b3c5f1903))
- *(store)* Named-graph column on facts — additive ROOT-default foundation (quipu #36)([b57bfab](https://github.com/scbrown/quipu/commit/b57bfab30eb779d2778889b3635dc2949ada6766))
- *(store)* Graph-scoped writes — overlays extend ROOT without mutating it (quipu #36)([aca75f3](https://github.com/scbrown/quipu/commit/aca75f3c74febf580acfd4a30a93bcc9c254cf53))
- *(episode)* /episode `graph` field — write knowledge into a named overlay (quipu #36)([8196a08](https://github.com/scbrown/quipu/commit/8196a084b21608028ef72437c9d78fbcb640cf33))
- *(query)* Content-negotiated W3C SPARQL 1.1 results — fix lossy shape([8e5ea77](https://github.com/scbrown/quipu/commit/8e5ea77d48589f1633467534cc441bb658c96ea5))
- *(store)* Named-graph overlay primitives — create / write / tombstone / compose (#36/#37)([bf1ecd1](https://github.com/scbrown/quipu/commit/bf1ecd13e0e409754c772a81f8ab22b85f1e70f3))
- *(shapes)* Governance-plane ("the loom") ontology + SHACL shapes (Phase 1)([325630c](https://github.com/scbrown/quipu/commit/325630c1ea1b3aabd05feaaaac4b13f6cbebb0b7))
- *(mcp)* Provenance-based work-item co-occurrence — /cooccurrence (quipu#37)([edc845d](https://github.com/scbrown/quipu/commit/edc845de025ca98887fb36b0336e53076d636a29))
- *(mcp)* Committed-tier policy evaluation — /policy/check (the loom, Phase 1 runtime)([02eff07](https://github.com/scbrown/quipu/commit/02eff07ee413d5f3ae3fc21c577303224be996d3))
- *(mcp)* Verifier registry — the Phase-0 authority layer (the loom)([c41ddcb](https://github.com/scbrown/quipu/commit/c41ddcb2b439b23bef91592774fd5beb9d23d84d))
- *(signing)* V1 verdict signing — the loom's Phase-0 root of trust([2336fc7](https://github.com/scbrown/quipu/commit/2336fc7031be230d3e0927cf5851c0ec130ea64d))
- *(retract)* Triple-level retraction — entity + predicate + value([758cf51](https://github.com/scbrown/quipu/commit/758cf5171482cde144466c561685ffb7c9196c88))
- *(shapes)* AnsibleGroup + TerraformResource node shapes([184aef4](https://github.com/scbrown/quipu/commit/184aef4197156d02e34e335e9c23094ea631dcb8))
- *(shapes)* Declare Incident, FailurePattern, SoftwareVersion, StoragePool([25e47a7](https://github.com/scbrown/quipu/commit/25e47a7e2b19274ea1b1c6cb4d0750c1c621d7ac))
- *(shapes)* Declare ClaudeCodeHook; aegis:Guard deliberately NOT declared([b7b31ad](https://github.com/scbrown/quipu/commit/b7b31ad449f5f60cebd7385fe670bb41a0715df7))
- *(server)* GET /version — the git SHA of the running build([6ee8412](https://github.com/scbrown/quipu/commit/6ee84129e2d72ca7d6f2763ea67e3a84e20a1ca8))

### Documentation

- The documented build must produce quipu-server — name --features onnx([1da8a09](https://github.com/scbrown/quipu/commit/1da8a090a21a60ae7811b7ba3a696476feffed0e))

### Fixed

- *(onnx)* Bind embedder inputs by name, not positionally([cafb2f7](https://github.com/scbrown/quipu/commit/cafb2f75375e5dffcd424d7f55f0389d6f532217))
- *(onnx)* Use real attention mask — pad tokens collapsed embeddings([cb7e620](https://github.com/scbrown/quipu/commit/cb7e62080bc6feabeb09c08053da25eb867f8112))
- *(shapes)* Reconcile SHACL shapes to what /episode actually emits([3d2cad3](https://github.com/scbrown/quipu/commit/3d2cad3dc0d0348a516db004075920391c50bd23))
- *(sparql)* IsBlank() was aliased to isIRI() — it matched the whole store([f0c70b7](https://github.com/scbrown/quipu/commit/f0c70b7fbf5b2cef674fbfe3e8488cb437c92cd3))
- *(episode)* Reject untyped nodes with a clear error, not a whole-episode Turtle 400([3769fa9](https://github.com/scbrown/quipu/commit/3769fa987cbc548d4b8205a42243ba4f84cff0b6))
- *(store)* Create idx_geav in the named-graph migration, not INIT_SQL([ae75e80](https://github.com/scbrown/quipu/commit/ae75e80e50fc9068d3cef89788895648e59f3d10))
- *(overlay)* Dedupe compose_view over re-asserted base facts (found in 69co live deploy)([e1288fe](https://github.com/scbrown/quipu/commit/e1288fe376c7b323cbeb9118a196530eebf7518f))
- *(cli)* --version/--help are pure reads, never open a store([a87ce9f](https://github.com/scbrown/quipu/commit/a87ce9f09a13fde6137150038a5e452bbab78c82))
- *(search)* Dedupe /search results by entity, keep best-scoring row([4f1c506](https://github.com/scbrown/quipu/commit/4f1c50677fa3fafae322a88c78cda7ccb95f3798))
- *(retract)* Episode retraction no longer orphans node identity([bfe7948](https://github.com/scbrown/quipu/commit/bfe7948d087affdd9448d026138ed5a3bb72e637))
- *(rdf)* Preserve language tags and datatypes in the Value model([f4d49df](https://github.com/scbrown/quipu/commit/f4d49dffa3e11ab4cc350bdef4c6ebd5bf447fad))
- *(shapes)* Add `get` (content read-back) and reject unknown actions([eb319be](https://github.com/scbrown/quipu/commit/eb319be56129be7677b77e02e0783b56f691ed87))
- *(retract)* One retraction datum per triple, not per backing row([26bd04b](https://github.com/scbrown/quipu/commit/26bd04bec37ff5af025d445d3f28c0aea2b4663a))
- *(retract)* Two-type coverage + refuse orphaning an entity's last rdf:type([0bae616](https://github.com/scbrown/quipu/commit/0bae6168f585e91187f957194f01d653693633ed))
- Scrub internal identifiers to zero, untrack the runtime store, add the RATCHET([258c6d7](https://github.com/scbrown/quipu/commit/258c6d7744cc5da2b585790a252b496666233473))
- *(auth)* Close 3 write routes that bypassed read-only + bearer auth, and enforce the list([7604448](https://github.com/scbrown/quipu/commit/7604448232809e8e7ea3b5379269c1bbfc2269b0))
- *(config)* Actually mint IRIs under the configured base_ns([7d54b10](https://github.com/scbrown/quipu/commit/7d54b105723324fbacef259519be6c29574f6739))
- *(group_ids)* Make code, doc, test and schema agree — provenance, not isolation([3b1762a](https://github.com/scbrown/quipu/commit/3b1762a453f5b46530639b08d109d0403a8638e8))
- *(server)* Re-tier 5 writing ro_handler! routes as rw, and enforce tier==classification([51a1436](https://github.com/scbrown/quipu/commit/51a1436bcd814bd75774cccc5ec9549338c01df1))

### Testing

- *(shapes)* Guard the SHACL shape invariants against drift([038c2f3](https://github.com/scbrown/quipu/commit/038c2f314024ca10f8f0309e63a5795fe0521ac9))
- *(retract)* Lang/typed literals stay precisely retractable([0579895](https://github.com/scbrown/quipu/commit/057989520b78893191aa4d7145d34821cb0f8c49))

## [0.3.3] - 2026-07-13

Graph analytics, a live report endpoint, episode-scoped retraction, and
caller-controlled CLI ingest.

### Added

- **Deterministic Louvain community detection (#31)** — community
  structure over the entity graph with stable, reproducible assignments.
- **Live graph report endpoint + `quipu_report` MCP tool (#32)** —
  on-demand orientation over the graph (size, central entities, activity)
  exposed via HTTP and MCP.
- **Episode-scoped logical retraction endpoint (#33)** — retract the
  facts asserted by a named episode without disturbing others.
- **`--base-ns` on `episode` (#28)** — override the namespace IRIs are minted in
  (defaults to the built-in aegis namespace), so non-aegis deployments can use
  the validation-carrying episode abstraction instead of routing around it.
- **`--timestamp` on `knot` / `episode` / `retract` (#27)** — supply the
  source-true `valid_from` (e.g. an upstream event time) instead of the exporter
  wall-clock, so bitemporal history imports keep their original valid-time. A
  lightweight ISO-8601 shape check rejects malformed values.

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
