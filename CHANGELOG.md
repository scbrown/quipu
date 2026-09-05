# Changelog

All notable changes to this project will be documented in this file.

## [0.3.39] - 2026-09-05

### Added

- *(cli)* `quipu ingest` — a streaming bulk load that refuses a short one([21f8ed9](https://github.com/scbrown/quipu/commit/21f8ed94e018f7e2396e708ce2816554c66a1a75))

### Fixed

- *(tests)* Gate ingest_cli on `shacl` — the binary is not built without it([1b18623](https://github.com/scbrown/quipu/commit/1b18623309602f0097ed7a6fe87efdb69deb4f3b))
- *(ci)* Run the owl tests, and give the three classes that failed a BFO root([cbf3d1f](https://github.com/scbrown/quipu/commit/cbf3d1fb0c7019c594214521df67b1cabe09385a))
- *(sparql)* An entailment regime must be a SUPERSET of the default answer([176ff15](https://github.com/scbrown/quipu/commit/176ff159fd6120afc8c9ce0bc457ef68a99c8c2a))
- *(rdfs)* Rdfs2 fires over literal-valued premises([a302ebb](https://github.com/scbrown/quipu/commit/a302ebb8fce6c1838c9b7ac18736b04fe603a616))

### Miscellaneous

- *(conformance)* Re-derive the ledgers on this branch's head — and the numbers did not move([7ae4a2e](https://github.com/scbrown/quipu/commit/7ae4a2e6e835e78d6b7197ea6c515cc018361cfe))
- *(conformance)* Re-derive on 89c22f6 after merging main([10f8bc9](https://github.com/scbrown/quipu/commit/10f8bc977ae491f305f7f15a5e6275830b587c21))
- *(conformance)* Re-derive the ledgers on this branch's HEAD([4337220](https://github.com/scbrown/quipu/commit/43372205afdb762e3cdd6a7e5bcf5e879078db12))
- *(conformance)* Re-derive the ledgers on this branch's HEAD([d4cb64c](https://github.com/scbrown/quipu/commit/d4cb64cf8496a3e67e7d463006dfb0f792c44868))
- *(conformance)* Re-derive the ledgers and page on this head([93863c5](https://github.com/scbrown/quipu/commit/93863c5dba02c8477516f04ef63f5a33d7c22ba4))

### Testing

- *(rdfs)* State the rdfs7 gap as a COUNT, not a paragraph (wu, #169)([31cadac](https://github.com/scbrown/quipu/commit/31cadaca977d085ebe8aa51af973d0e4f05f296f))

### Style

- Cargo fmt the rdfs2 literal-premise tests([df044b3](https://github.com/scbrown/quipu/commit/df044b3b8c557948fcfd5c191ca1ec6aa043ec86))

## [0.3.38] - 2026-09-05





### Miscellaneous

- *(conformance)* Repair main's stale ledger([b154582](https://github.com/scbrown/quipu/commit/b154582c184ddde7948df4d80bdf0857d703bc35))

### Demo

- *(align)* The alignment half of the sharing story, as a checked transcript([cbac8df](https://github.com/scbrown/quipu/commit/cbac8df761d1d8513ab528662bd9edbddbe46a69))

## [0.3.37] - 2026-09-05

### Added

- *(align)* A `quipu align` CLI verb — make src/align reachable([8212da0](https://github.com/scbrown/quipu/commit/8212da052ee65fec1e37e099bd6b7cda4722c69f))
- *(align)* REST routes and three MCP tools — the surface is now reachable([ea67440](https://github.com/scbrown/quipu/commit/ea674409daa98efad4a7ad27c6b90bcfb1540a8d))
- Add contributor knowledge and a mobile constellation explorer([d2807e2](https://github.com/scbrown/quipu/commit/d2807e2756e7bd388b6ce75e8b4b3dac6ccbfca7))
- *(mcp)* Entailment regime on tool_query, and a marker that says so([8233447](https://github.com/scbrown/quipu/commit/8233447ca788fd0438b804d92e693fe72863977d))
- *(align)* Count entities examined but unlabelled, so a zero says WHICH zero([84c4fab](https://github.com/scbrown/quipu/commit/84c4fabbe0edc989bd6d03be5986dcf6aeffec98))

### Changed

- *(server)* Assets owns its routes, so server.rs is back under the ratchet([db83628](https://github.com/scbrown/quipu/commit/db83628d9c743d567b4a84fff419bb8172ec1d5a))

### Fixed

- *(conformance)* PR mode for the ledger check, and the push trigger that never fired([0f63470](https://github.com/scbrown/quipu/commit/0f63470c29b119c643cb42e1b6eba299d581310d))
- *(cli)* Document `align` in --help; cargo fmt([8cea89d](https://github.com/scbrown/quipu/commit/8cea89db010ffd8aa0b25e09b2926cb13e1e1c89))
- *(server)* Re-export align tools at the crate root; own route module([0c54147](https://github.com/scbrown/quipu/commit/0c5414786ee021cc783c92c729f1765fd39f0d56))
- *(align)* Who owns the graph NAME decides what an absent graph means([735866f](https://github.com/scbrown/quipu/commit/735866f432f9bfc502c7e3374abdbe999e89bf94))

### Miscellaneous

- *(conformance)* Re-derive ledgers on 0f63470c([e3f3a10](https://github.com/scbrown/quipu/commit/e3f3a10893021173882b45690fec4ae51ee620a1))
- *(conformance)* Re-derive ledgers on the marker head([5908c90](https://github.com/scbrown/quipu/commit/5908c90c69bcd6a9adb1dc479dd8c78e2b45435a))
- *(conformance)* Re-derive ledgers on 326428c0([37263ec](https://github.com/scbrown/quipu/commit/37263ec2d2a8dff7419b477921a15a2ae9d9d91c))
- *(conformance)* Re-derive ledgers on 25336d1b([95544cc](https://github.com/scbrown/quipu/commit/95544cca8659621ebbbd838c984f101c456a21c9))
- *(conformance)* Ledgers for the constellation work([e295944](https://github.com/scbrown/quipu/commit/e2959449250c34cdf53aa225f509375319667628))
- *(conformance)* Re-derive ledgers on 84c4fabb([2a30071](https://github.com/scbrown/quipu/commit/2a30071587d5d44df3271a1f36a53fe4bbfb30ca))
- *(conformance)* Keep this branch's own ledgers through the merge([c3aa818](https://github.com/scbrown/quipu/commit/c3aa818443b789ba1e8ef70009fb3cb57e336205))

## [0.3.36] - 2026-09-05

### Added

- *(rdf)* Streaming chunked ingest, and a declaration that refuses a short load([c0058f4](https://github.com/scbrown/quipu/commit/c0058f4b26452f77947032412d3173d603bc7f06))
- *(align)* R2 — reach an alignment assertion without knowing it exists([5020433](https://github.com/scbrown/quipu/commit/502043357ba2c771d40164e16fb4b63489155510))
- *(conformance)* Let goal regimes EXECUTE, and derive the commitment([cd2672f](https://github.com/scbrown/quipu/commit/cd2672f7a89a0f1af7f1bf88258c39de81358caa))
- *(sparql)* Materialise the RDFS closure into the companion graph([328bfc5](https://github.com/scbrown/quipu/commit/328bfc58f6d9f93b14491e122e5a016eb96a5dda))
- *(cli)* --entailment rdfs answers over the materialised closure([c97b7cc](https://github.com/scbrown/quipu/commit/c97b7cccc5824f2a66207a5bda7db83870ec074f))
- *(conformance)* Ledgers record WHO produced them, and the page prints it([f2a174b](https://github.com/scbrown/quipu/commit/f2a174bc6e8a483d00aff499fffd82aa377748dc))
- *(conformance)* Answer RDFS-regime cases under the closure([9a3fbd5](https://github.com/scbrown/quipu/commit/9a3fbd576fbfc797a1e4ab4ccfc2dc9a8810b2a9))
- *(conformance)* --check now asks whether a ledger matches the code it ships with([4aa67ad](https://github.com/scbrown/quipu/commit/4aa67ad54dd76fc83dd4e8967d1596ba4d6004c3))



### Fixed

- *(import)* An exact name match is a PROPOSAL, not a silent IRI rewrite([baa9d98](https://github.com/scbrown/quipu/commit/baa9d9818b4754828e2a6ab30de76a7b508fc447))
- *(align)* R2 offers a VERDICT, not a command that does not work([6f4cfe9](https://github.com/scbrown/quipu/commit/6f4cfe9f60719f80ce68c3f0e76f785673219060))
- *(cli)* Extract --entailment into its own module; refresh the badge([3314294](https://github.com/scbrown/quipu/commit/33142943df02befca40fdeabdb9e94d13b0a8c45))
- *(conformance)* A failing CASE must not kill the step before the gate runs([c13e154](https://github.com/scbrown/quipu/commit/c13e1545938233a46f361a5d6d72df12a92b131d))

### Miscellaneous

- *(conformance)* Re-derive all five ledgers on the code they ship with([76591ae](https://github.com/scbrown/quipu/commit/76591aebad3ac2eeaeb0c4b5cfd1751728306dc6))

### Bench

- *(entailment)* Publish 23/35, from a CI-produced ledger([cccf2bd](https://github.com/scbrown/quipu/commit/cccf2bdc295eda3ae0b54da8b866ccf530479ad9))
- *(entailment)* 29/35 from a ledger that matches the code it ships with([cc181e9](https://github.com/scbrown/quipu/commit/cc181e9560b8dbe09837fd7f32ae2748739a1c8b))

### Style

- Order mod cli_entailment alphabetically([444028e](https://github.com/scbrown/quipu/commit/444028e6005d518c2247d85d3f60087a8e651753))

## [0.3.35] - 2026-09-05

### Added

- *(align)* Apply — decided mappings become owl:sameAs knots in an alignment graph (#127)([7020e86](https://github.com/scbrown/quipu/commit/7020e86ef9a7d79632582a72424dc23151e03045))
- *(align)* Store-backed enumeration, and a proposal that counts what it set aside (#132)([395b11a](https://github.com/scbrown/quipu/commit/395b11a383a75f274e3033a70b6f157c5b967b56))
- *(align)* Decide — the operator's judgement, applied to a proposed set (#137)([782122d](https://github.com/scbrown/quipu/commit/782122d519fb051df18829c8daae875929f2493e))
- *(share)* Pack_dir, and a wasm-callable delta that REUSES share-delta/v1 (#135)([5e5682f](https://github.com/scbrown/quipu/commit/5e5682f8c627bd0785450b6c2d1b164892825aac))

## [0.3.34] - 2026-09-05

### Added

- *(align)* SSSOM mapping set, propose and verify [] (#123)([2d048a8](https://github.com/scbrown/quipu/commit/2d048a8df40d85b484d12356c8a7355dea8f3090))

## [0.3.33] - 2026-09-05

### Added

- *(docs)* Run the repository's own knowledge pack in the browser [] (#116)([a33c50b](https://github.com/scbrown/quipu/commit/a33c50b8fe271b794c8b19eab573979c10a54a9d))
- *(docs)* Let the book page EDIT the graph and export it back [] (#117)([a284daf](https://github.com/scbrown/quipu/commit/a284daf2fe02f363dd6ff25b91b237954f1d541d))

## [0.3.32] - 2026-09-05

### Added

- *(auth)* Make the attestation replay set durable and savepoint-joined (#114)([64b63b7](https://github.com/scbrown/quipu/commit/64b63b7dbd2737ac962b14e94757781e405b2e6e))

## [0.3.31] - 2026-09-04

### Documentation

- *(sharing)* Prove the two-store story end to end [.3]([af5682f](https://github.com/scbrown/quipu/commit/af5682fb2ae8f4fd0b0849b72dbf235db2bb95c6))

### Testing

- Clean up temporary test directories [] []([72f4990](https://github.com/scbrown/quipu/commit/72f49909e593024e30f03090c5851664f7565019))

## [0.3.30] - 2026-09-04

### .2

- Publish 10/10 SPARQL result formats([ab46b57](https://github.com/scbrown/quipu/commit/ab46b5730e5390e257213474a1bf731b7cf9b5b8))
- Add SPARQL query form transport defaults([3749cfc](https://github.com/scbrown/quipu/commit/3749cfcdea4f821b672c618eab9d3f06f7c13563))
- Satisfy full-bundle clippy([7ef18ea](https://github.com/scbrown/quipu/commit/7ef18ea0dbdbc54fb4a9e452213ce1fb23e6c945))
- Reach full SPARQL query protocol conformance([5b5e11f](https://github.com/scbrown/quipu/commit/5b5e11fe025af69c7c47633e1c2cd7aae148736c))
- Complete SPARQL protocol and update conformance([6f01994](https://github.com/scbrown/quipu/commit/6f019948eea02b25f4c4e311eae5e8410eb17c58))

### .3

- Add pinned SHACL and entailment ledgers([f34a712](https://github.com/scbrown/quipu/commit/f34a7120690bc75bdcbe90c06889b14ce168472d))

### .7

- Score W3C federated query suite([7f77b9b](https://github.com/scbrown/quipu/commit/7f77b9b350772ef33469cc4e496e60a3243ac2df))

### Added

- *(release)* Embed repository knowledge share (.4)([5459846](https://github.com/scbrown/quipu/commit/5459846ed8c4b965090940d41ad890a626b8be29))

### Documentation

- Sharing reference, and tests against two kinds of doc rot [.2]([148780a](https://github.com/scbrown/quipu/commit/148780aeb74de779be62820dc2cfa5cd452b993a))
- *(benchmarks)* Link conformance back to Sharing & Federation []([48b31e4](https://github.com/scbrown/quipu/commit/48b31e4f9aaff7b5e9f64db4cfbc6420d74d8cf8))
- *(share)* Align CLI reference with text artifacts (.2)([ca3d3e9](https://github.com/scbrown/quipu/commit/ca3d3e9c28057f91d9b145b3939118caf2f8a3de))

### Fixed

- *(release)* Require conforming fresh-store share proof (.4)([78eb8a1](https://github.com/scbrown/quipu/commit/78eb8a1e3013f1e0f6de13c4d76b6c6cee9478b4))
- *(service)* Preserve RDF result term identity([304ebe2](https://github.com/scbrown/quipu/commit/304ebe21c13444baa053892d80b232a48d8a6e4f))
- *(ci)* Split source-size ratchet violations([7877c74](https://github.com/scbrown/quipu/commit/7877c74bcb5e9e0947428d6e3292c1f3401a669d))

## [0.3.29] - 2026-09-03

## [0.3.28] - 2026-09-03

### .1

- Implement language and datatype constructors([7c5c029](https://github.com/scbrown/quipu/commit/7c5c029f1aa3a8595ba98aeeab1dbba7e0d5617f))
- Publish 108 query evaluation passes([d2c258a](https://github.com/scbrown/quipu/commit/d2c258a108475af94466013d86fd70c8cac0b155))
- Evaluate date-time accessor functions([9b8ce87](https://github.com/scbrown/quipu/commit/9b8ce87636743c608c0eb6f070223897a862776d))
- Publish 116 query evaluation passes([4c2b430](https://github.com/scbrown/quipu/commit/4c2b430df08cce45479666bfcbc5a6953918c214))
- Evaluate conditional expressions([cc61bdf](https://github.com/scbrown/quipu/commit/cc61bdf84dc6282bebfa7238ab4008a00988740f))
- Publish 119 query evaluation passes([0d7355d](https://github.com/scbrown/quipu/commit/0d7355d1228e4ec2e9830ee2b56fa8c5ea8e015b))
- Evaluate runtime and datatype builtins([775fe7b](https://github.com/scbrown/quipu/commit/775fe7be9e6931505b2f2d2dc2155318ee42a64b))
- Publish 125 query evaluation passes([b33da15](https://github.com/scbrown/quipu/commit/b33da15c00823bd4272c6d1191fd9f772314f3c7))
- Preserve aggregate numeric semantics([c4c06c4](https://github.com/scbrown/quipu/commit/c4c06c4a89c011bec4ed541ece7146db585669b2))
- Publish 134 query evaluation passes([e2cb5a6](https://github.com/scbrown/quipu/commit/e2cb5a65fbe686e7ca389e47d725ea11d501ed76))
- Project boolean expressions([d38f77f](https://github.com/scbrown/quipu/commit/d38f77ff4c638a546e12a3231493a9fef54be329))
- Publish 135 query evaluation passes([13bdfb7](https://github.com/scbrown/quipu/commit/13bdfb7518039750a046aa9ce46b30848977fc8e))
- Evaluate MINUS graph patterns([69b889b](https://github.com/scbrown/quipu/commit/69b889b81f86bbb44aa243f071015c86e9b66b8d))
- Publish 142 query evaluation passes([4844044](https://github.com/scbrown/quipu/commit/4844044d7c4eecf3393df412a35dc10420dd538c))
- Complete zero-length path evaluation([6e31220](https://github.com/scbrown/quipu/commit/6e312207bcf00350d872cab7719e2c2b720f7dbb))
- Publish 144 query evaluation passes([b744ba3](https://github.com/scbrown/quipu/commit/b744ba3a28058e7fe282184c315160cee6c557a2))
- Resolve IRI builtins against query base([5a59e82](https://github.com/scbrown/quipu/commit/5a59e829fa43dd4d47c9945712b12cf981494543))
- Publish 145 query evaluation passes([eabee33](https://github.com/scbrown/quipu/commit/eabee339f83b1920d661d7a14d7a60600bf94769))
- Publish 150 query evaluation passes([5206b91](https://github.com/scbrown/quipu/commit/5206b9197109ba5bb8e75274e8b2f64ee0a35ba4))
- Publish 168 query evaluation passes([bff33d5](https://github.com/scbrown/quipu/commit/bff33d56e339078ebee461678818daa924d881f7))

### Added

- *(sparql)* Evaluate numeric arithmetic expressions([a0fc8fc](https://github.com/scbrown/quipu/commit/a0fc8fca0b89919d0fd11d567ffb178bdf819069))
- *(pack)* Verify and incrementally load repo artifacts([2c7675f](https://github.com/scbrown/quipu/commit/2c7675fa232378217d73d7fab03014e3887fd17e))
- *(sparql)* Evaluate numeric builtins [.1]([d3f9a19](https://github.com/scbrown/quipu/commit/d3f9a1926d02fa9f0ed00b86fc1114e8ac29d044))
- *(sparql)* Evaluate string builtins [.1]([88aed04](https://github.com/scbrown/quipu/commit/88aed0460791ffa99ae8f90bf21f1b2b7a8f783a))
- *(sparql)* Evaluate hash builtins [.1]([f910f24](https://github.com/scbrown/quipu/commit/f910f24fc72d91e99def68267a232533a5e3864b))
- *(cli)* Load W3C named graph fixtures (.1)([7895151](https://github.com/scbrown/quipu/commit/78951512a99cbf491fb06095bf71a55686f43c13))
- *(sparql)* Complete query evaluation conformance (.1)([07173bd](https://github.com/scbrown/quipu/commit/07173bd8f3bdbfe8af504074521e215126317840))
- *(share)* Load text qpacks by reference (.5)([d664328](https://github.com/scbrown/quipu/commit/d664328c565021b12d479cbb6a29cb8bc18ddf1d))
- *(share)* Canonicalize and describe share payloads (.5)([ef9e406](https://github.com/scbrown/quipu/commit/ef9e4068d851d1597bb572458f512c2488f3583e))
- *(share)* Add parent-bound delta artifacts (.5)([5179018](https://github.com/scbrown/quipu/commit/5179018fbbc6c8812eae004fe6be1e690609f80c))

### Changed

- *(pack)* Isolate verified load path([a349689](https://github.com/scbrown/quipu/commit/a3496898af390c95adc01c642469b8e4f26dcd65))

### Documentation

- *(conformance)* Publish 81 query evaluation passes [.1]([1a6e4d7](https://github.com/scbrown/quipu/commit/1a6e4d7989ee4b7e2f3440d5cee2bffcf6ac95dd))
- *(conformance)* Publish 96 query evaluation passes [.1]([2d52b45](https://github.com/scbrown/quipu/commit/2d52b45d5c429f2028910b370c76c26a1bdfb734))
- *(conformance)* Publish 104 query evaluation passes [.1]([5331256](https://github.com/scbrown/quipu/commit/53312569f1d437412b4d3fc055b399fae2e4e348))
- Tell sharing and federation as one primitive [.1]([312f48a](https://github.com/scbrown/quipu/commit/312f48aa3576b1c63286a6c5ecd1740e4e270023))
- *(share)* Publish unified artifact contract (.5,.1)([a5a8444](https://github.com/scbrown/quipu/commit/a5a8444fd29fad8205ecfdd42a9223cfe737ec5c))

### Fixed

- *(sparql)* Support feature-unified pattern enums (.1)([c558b8e](https://github.com/scbrown/quipu/commit/c558b8ecddd13d00649a24698b6f7660b4c7d116))

### Testing

- *(conformance)* Compare CONSTRUCT RDF graphs (.1)([093d107](https://github.com/scbrown/quipu/commit/093d107622bea57b78109a579e500b5ce193db02))

### Bench

- Compare W3C IRIs by lexical value([dcd9a12](https://github.com/scbrown/quipu/commit/dcd9a12c2760e763c3ef4ee10add2d039df66c7e))
- Publish 77-case SPARQL evaluation baseline([6c2f33f](https://github.com/scbrown/quipu/commit/6c2f33f7dc9713a341d3094b503fb03f35136846))
- Publish conformance results, with gates that keep them honest([c76d5a1](https://github.com/scbrown/quipu/commit/c76d5a1316971e8635d784cbd739e82e49feeec9))

## [0.3.27] - 2026-08-25

### Fixed

- *(ci)* Restore file-size and changelog gates([652c4d8](https://github.com/scbrown/quipu/commit/652c4d887a33cd570d6d9a2c8fdcb158afb648d9))

## [0.3.26] - 2026-08-25

### Added

- *(pack)* Ship a pack in a designated term space with --space([d5ddb05](https://github.com/scbrown/quipu/commit/d5ddb0545fd8583aa01b9b58b5453ae1dbb86db9))
- *(governance)* Placement gate requires a backoffFormula on throttle effects([fedc261](https://github.com/scbrown/quipu/commit/fedc261e86e7f24a93bdbf17b7cc899fd7d5938f))
- *(governance)* Verify agent transition signatures at the write gate([beaca8a](https://github.com/scbrown/quipu/commit/beaca8a547bf5b79bfbde9a10bc1a13a5d4bac8e))
- *(federation)* Declared trust labels at the federation edge([f4623b0](https://github.com/scbrown/quipu/commit/f4623b051baf345fc68e07b765b777f8ff5342af))
- *(freeze)* Archive packs carry entity embeddings([71a0295](https://github.com/scbrown/quipu/commit/71a0295e0c004c272536127222f8367bf639aac7))
- *(config)* [[quipu.attachments]] mounts declared layers at open([3c0034a](https://github.com/scbrown/quipu/commit/3c0034a5bf9f8186559ecb09e612e5c5619292dc))
- *(vector)* Vector.backend selects the LanceDB backend in-binary([b70305e](https://github.com/scbrown/quipu/commit/b70305e7bd4cffdc7a83baf9365ba14cd7943027))
- *(audit)* Quipu audit namespace reports base-namespace drift([83f92f6](https://github.com/scbrown/quipu/commit/83f92f68fe9ff1b6ce1d2f4661bdb3c1f26b243f))

### Documentation

- Document quipu policy / quipu path CLI and the two /path/* routes([7fd8704](https://github.com/scbrown/quipu/commit/7fd870416c5d7dd548cdb0a063be7118416303e8))
- *(design)* Correct two stale implementation banners([5086291](https://github.com/scbrown/quipu/commit/50862914c15151abcbeddd949b986e07726d1a02))

### Fixed

- *(test)* Isolate cached_validator_retains_its_shapes from the global validator cache([8f40206](https://github.com/scbrown/quipu/commit/8f402060fefa749ff9d12dbe285da6b192b1d301))

### Miscellaneous

- *(beads)* Adopt the documented JSONL-only convention (jsonl export)([1650abf](https://github.com/scbrown/quipu/commit/1650abfdbae3a957be1f95ce6670933d63c0fa85))
- *(beads)* File six gap beads from the 2026-08-25 design-vs-impl survey (jsonl export)([2fb2c61](https://github.com/scbrown/quipu/commit/2fb2c61cb4fc8b5e31914e7c250e92bdb76ffcbf))

## [0.3.25] - 2026-08-24

### Added

- *(shapes)* Expose loaded class vocabulary([5c29abe](https://github.com/scbrown/quipu/commit/5c29abe1c9458b525ba5920535edc0a36d52a7dc))
- *(shapes)* Declare Chunk, Credential and BeadScope([12dec00](https://github.com/scbrown/quipu/commit/12dec007251e5117398dcf81bf1ef35a911b10ee))
- *(vocabulary)* Advise when a write types a node no shape governs([3524a82](https://github.com/scbrown/quipu/commit/3524a825020389d6908e1e6cb74e403dbaff2105))
- *(ontology)* Describe desired crew composition([5e05ebf](https://github.com/scbrown/quipu/commit/5e05ebf6dc8c02ec8b37e4e3ad8349c01e5f69a0))
- *(governance)* Tripwire policies — declare aegis:appliesTo, ship the path-boundary catalog([0ae7e88](https://github.com/scbrown/quipu/commit/0ae7e88e480d9fd1d5603a9f574fedb874e37267))
- DataKind label axis — categorical graph-kind dimension on the label lattice([f9e0ce7](https://github.com/scbrown/quipu/commit/f9e0ce7e0b790ae2da14c7e412d42226eaa3ef81))
- GET /graphs listing + include_kinds query widening([20e6676](https://github.com/scbrown/quipu/commit/20e66769cdb69aaebad4b0383871f0928cee65e0))
- Deep freeze — relocate a graph's full history into a composable read-only archive([df12db9](https://github.com/scbrown/quipu/commit/df12db93a416f267233e9b24319bd0a1c626650a))

### CI/CD

- *(shapes)* Run verify_shape_invariants — the gate was wired into nothing([d4206b7](https://github.com/scbrown/quipu/commit/d4206b7a245eccf88113014c983c69c3377392e4))

### Changed

- *(vocabulary)* Move the episode advisory out of episode/mod.rs([7596a1a](https://github.com/scbrown/quipu/commit/7596a1ab83ed51ac7dca327ebab1a31ef5071a10))
- *(read-model)* Split the applicability guard out to unbreak check-file-size([08be8a4](https://github.com/scbrown/quipu/commit/08be8a4c35f35a90c694567bcde66d54e5854923))
- *(read-model)* Split the index out, rather than raise the size baseline([afb8707](https://github.com/scbrown/quipu/commit/afb870792dfcfadcf9cb197cd5e131e4840e59da))
- *(episode)* Isolate description reconciliation([01e342e](https://github.com/scbrown/quipu/commit/01e342e3947b4f2f6aadfca3b6266ea0da1b3ce1))

### Documentation

- Document graph scopes and agent access order([c3c1a9d](https://github.com/scbrown/quipu/commit/c3c1a9d8df80f0954d0982290496bfc0c60a5beb))
- Catch the book and README up with the dataKind axis, tripwires, and episode description revisions([4c446aa](https://github.com/scbrown/quipu/commit/4c446aa38097c077a04456d2460a8ba48f39c2e4))

### Fixed

- *(ci)* Restore green ratchets and clippy([a2b1f42](https://github.com/scbrown/quipu/commit/a2b1f42acd50cb20fd57701ac8d1b9131cb61c22))
- *(ci)* Satisfy remote-feature clippy([9ef4f18](https://github.com/scbrown/quipu/commit/9ef4f1822603eb16ce31614f4b423e234b84476b))
- *(ci)* Baseline integrated reasoner growth([94faeeb](https://github.com/scbrown/quipu/commit/94faeeb91418c2db5e427abe2cb2fa3dd98ac306))
- *(shapes)* Stop I1/I2/I3 claiming coverage they never had([d3d9d8b](https://github.com/scbrown/quipu/commit/d3d9d8b33684afd6b708afe84df4166417be9761))
- *(shacl)* Repair an incremental write with the floor the store already holds([37f5c3b](https://github.com/scbrown/quipu/commit/37f5c3bb5efb0b97379edb34645aaa605073498f))
- *(read-model)* Check freshness against the database, not in-process bookkeeping([382ac15](https://github.com/scbrown/quipu/commit/382ac1555b63ab0056f5cd255d6e2dbd0812f11d))
- *(ci)* Rustfmt + doc backticks in read_model tests([8cd37f0](https://github.com/scbrown/quipu/commit/8cd37f0e1454433834f78572509d23578d6cb63b))
- *(ci)* Rustfmt read_model.rs too — the Format gate covers both files([8506e32](https://github.com/scbrown/quipu/commit/8506e32308e2ca3ed553b870cb0d90bdce0550d8))
- *(release)* Reject tracked files hidden by ignore rules([9f925cd](https://github.com/scbrown/quipu/commit/9f925cdfa7796e55e012358c2f8adeec35ba4eaf))
- Preserve canonical slash node paths in episodes([adb1b27](https://github.com/scbrown/quipu/commit/adb1b273f6287cfa4c99b0061dabbd817d8a209e))
- /episode graph field registers the graph instead of bare-interning (camayoc-s0h)([9401926](https://github.com/scbrown/quipu/commit/9401926b71d7364261ec714108f70eb1c9815176))
- *(deploy)* Serialize Quipu installs with attributed lock([8f042c3](https://github.com/scbrown/quipu/commit/8f042c36bbaa144753cee3d0b05f9b8cbde4c469))
- *(ci)* Count CODE lines in the file-size ratchet, and split the MCP manifest([3aa1327](https://github.com/scbrown/quipu/commit/3aa1327d986a432dddeb82393c877f0635e4ad62))
- *(ci)* Missing trailing newline in the file-size selftest([7d3aa2d](https://github.com/scbrown/quipu/commit/7d3aa2d41c93099bbc0a65070c56d705781999f9))
- Keep metrics off the writer queue([8264870](https://github.com/scbrown/quipu/commit/82648700e692139a4d669066d9077211b6fd9907))
- *(metrics)* Cap caller identities, not endpoint pairs([15cb77f](https://github.com/scbrown/quipu/commit/15cb77f1b50c336bfea828c9be8b1d0c17fc6b26))
- *(shapes)* Govern Concept and Component([e8c5e0c](https://github.com/scbrown/quipu/commit/e8c5e0c16876849220a176d7f99b666f478cf428))
- *(ci)* Restore wasm and file-size gates([fffa9ae](https://github.com/scbrown/quipu/commit/fffa9ae4a94ca074ab75cd2a580026fd5cb49032))
- *(ci)* Keep classified graph routes visible([8fb9f83](https://github.com/scbrown/quipu/commit/8fb9f839befaeb48f57fac8be1c6f0549ede3c7f))
- *(episode)* Preserve superseded descriptions([4cd620a](https://github.com/scbrown/quipu/commit/4cd620a7e5feaee071061c1e90a290df66a91684))
- Pooled readers compose frozen packs — sync against the registry on acquisition([eecdea9](https://github.com/scbrown/quipu/commit/eecdea984114d80ac6bf112f4dc6fc3145913319))
- *(episode)* Dedupe multi-type description revisions([b633a7f](https://github.com/scbrown/quipu/commit/b633a7f5888756ac00f11b9fb6d76cb0dc18d810))

### Miscellaneous

- *(beads)* Close quipu-uq2/-k7w/-cbh — graph kinds + deep freeze shipped (jsonl export)([1a0f911](https://github.com/scbrown/quipu/commit/1a0f911036a865e4d01ab571c70756924c1c41ef))

### Testing

- *(episode)* Cover legacy comment migration([94ab1cb](https://github.com/scbrown/quipu/commit/94ab1cb75b8be9c19b0137b7b84571f57a6e52ba))

## [0.3.24] - 2026-08-22

### Added

- *(wasm)* SQLite on wasm32 via rusqlite 0.40 — quipu-qd2 lands([6609c0a](https://github.com/scbrown/quipu/commit/6609c0ae842ddb504de21b4a343e88e34b3dbe63))
- *(wasm)* Measure wasm-vs-native throughput — quipu-ajz lands([da88223](https://github.com/scbrown/quipu/commit/da8822372f448aa9338a385165f639032ea2506f))
- *(wasm)* .db export/import and the pack round-trip — quipu-2l5 lands([38952e0](https://github.com/scbrown/quipu/commit/38952e0206d2cc5a4e6cef8b3c428dd9406cef40))
- *(governance)* Entity-grounded predicate vocabulary — candidateSource, groundingQuery, grounded match types, embedding/model tiers([5eeea9e](https://github.com/scbrown/quipu/commit/5eeea9ee7444e2f30107dc14fabe970727043e7a))
- *(governance)* Attach nearest decided DecisionRequests as scored precedent when minting([9cbc747](https://github.com/scbrown/quipu/commit/9cbc7477ef3200f1f14579295644db639b607477))
- *(governance)* Claimed-linkage verification — typed three-outcome check of aegis:implements claims([1589f18](https://github.com/scbrown/quipu/commit/1589f185ddd90843056f46fd42591e9fa48d8ada))
- *(governance)* Policy by example — exemplar linkage, advisory drafting scaffold, pre-creation backtest, reject-to-policy seam([0f46030](https://github.com/scbrown/quipu/commit/0f460300867c3d1bdd05045957e098179fd0ba39))
- *(server)* Expose graph registration and labelling over HTTP([f3d93e5](https://github.com/scbrown/quipu/commit/f3d93e596fe73570d99142e6b0bf9c7026a6dbba))
- *(context)* The pipeline says what the budget cut([9ce21ee](https://github.com/scbrown/quipu/commit/9ce21eef8763b4f95d61e9088c4a66aca0f6f1ab))
- *(ask)* Work-item disclosure rungs in the named-query catalog([c35fa39](https://github.com/scbrown/quipu/commit/c35fa394ede66df789688fa521c4f957a75708a3))
- *(path)* Golden-path analysis — grammar v1, provenance cone, backtest, draft (quipu-gp1..gp4)([8c150b1](https://github.com/scbrown/quipu/commit/8c150b1034d9089193f09a229393128626985bad))
- *(knot)* Route /knot writes to registered committed graphs([22b3569](https://github.com/scbrown/quipu/commit/22b3569d5673556e3baf4afa81c580fb5ddf2284))
- *(fork)* Persistent named forks — fork ROOT at any tx, promote through the gates([b636458](https://github.com/scbrown/quipu/commit/b63645817fdf842a81838f7e82a13e99f95567a0))
- *(events)* Record write refusals as queryable write.refused events([71440ff](https://github.com/scbrown/quipu/commit/71440ff13b5be216b3a0a2d4b450eca1156b8d32))

### CI/CD

- *(wasm)* The browser harness enters the matrix — quipu-ame lands([6cf8864](https://github.com/scbrown/quipu/commit/6cf886452450a2b634afe592ea122df5c10ecd58))

### Changed

- Shrink seven files past the size ratchet's full-scan baseline([8da0c7a](https://github.com/scbrown/quipu/commit/8da0c7a5b097948fedd95012e624e8361d9b78c5))
- *(namespace)* Bobbin: is the aegis base; delete dead bobbin.dev IRIs([ee0c5a6](https://github.com/scbrown/quipu/commit/ee0c5a623e2a5a2e95b530619c408cae8728dd18))

### Documentation

- *(design)* Verified headless wasm test harness; correct qd2/ajz triage([ccacf6e](https://github.com/scbrown/quipu/commit/ccacf6e1de79c8de328be73757863350642e0dfc))
- *(patents)* Disclosure timeline and provisional draft for the governance cluster([f9d8594](https://github.com/scbrown/quipu/commit/f9d85942de5ae67cbc654e4f58cd89b8da1a4265))
- *(patents)* Apply adversarial review to the governance provisional([3ce18ae](https://github.com/scbrown/quipu/commit/3ce18aebcb1b165d359342a7c648c21f335b79e1))
- *(patents)* Rev 2 of disclosure timeline after adversarial re-derivation([a1127ab](https://github.com/scbrown/quipu/commit/a1127abe0a7dbd185047de8eb9e1006170371b88))
- *(patents)* Filing-day cover sheet data for provisionals A and B([1806710](https://github.com/scbrown/quipu/commit/1806710c781e7e999bc8599e61bd066de5b400f8))
- *(patents)* Add provisional C (NeuralAmplifier) rows to disclosure timeline([e548d47](https://github.com/scbrown/quipu/commit/e548d47666f071b569d974caa339cb2e8dbaca8d))
- *(patents)* Filing-ready PDF of provisional A([8d911e1](https://github.com/scbrown/quipu/commit/8d911e1f1cec1d3a15a1c182cd620b312c2e5d2d))
- *(patents)* Add provisional C to filing cover sheet data([3c5134b](https://github.com/scbrown/quipu/commit/3c5134b4980bc2aa250e7c22b1ac0a9b221fc261))
- *(patents)* Rev 2 of cluster C timeline rows after adversarial re-derivation([49f5af3](https://github.com/scbrown/quipu/commit/49f5af34cbdfe6c1503764c1177299bd46797e83))
- *(patents)* Fix blank pages 25-26 in provisional A PDF; numeral 108 in brief description([cecd53b](https://github.com/scbrown/quipu/commit/cecd53bc5309c03528faacd697e96268efc4f195))
- *(patents)* Preliminary prior-art sweep notes for provisional A([748a867](https://github.com/scbrown/quipu/commit/748a8671773802a832e63088c3eebd267828428a))
- *(patents)* Cluster D timeline rows, cover sheet entry, prior-art notes([b5703e6](https://github.com/scbrown/quipu/commit/b5703e690571e31ad760000c7537f3b1c9b1dbc8))
- *(design)* Semantic, entity-grounded edit policies; timeline row; grounding pitch bead([9345228](https://github.com/scbrown/quipu/commit/9345228417cbbb58aaf275f61d5936fe7db5f130))
- *(design)* Rev 2 — drop the regex; tokenized membership and the embedding tier([5148da0](https://github.com/scbrown/quipu/commit/5148da05bbb90203808c0c65b30bbbfde1f176ec))
- *(design)* Further applications of similarity-as-grounding; claimed-linkage pitch([cca1ef3](https://github.com/scbrown/quipu/commit/cca1ef3826e8575891a2b27ad2ddca14897fa57a))
- *(design)* The identify-and-inform-before-refusing ordering; escalation-precedent pitch([19242e8](https://github.com/scbrown/quipu/commit/19242e8705236b1ef14bc50831ff0f603ada3414))
- *(design)* Policy by example — one gesture from an observed edit to a governed advisory rule([9bd745d](https://github.com/scbrown/quipu/commit/9bd745db0053754e92d9ccc8f19cb74af872c95f))
- *(design)* Policy-by-example status — quipu-side steps 1/3/4 built([b0a2e16](https://github.com/scbrown/quipu/commit/b0a2e16789f6101639253adbebe8c7a03c6eaf20))
- *(patents)* Disclosure timeline rev 3 — 2026-08-15 cluster-D rows([fd1d752](https://github.com/scbrown/quipu/commit/fd1d752ea91d241f4e8e7d4c1cf5d85fcddd7c2e))
- *(paper)* External measurement of the fast-plane consumer (§6)([67a7fc6](https://github.com/scbrown/quipu/commit/67a7fc62b13489be6488483f00b136f53eb3d2d8))
- *(patents)* All four specs are filing-ready; retire the inferred-name caveat([61257af](https://github.com/scbrown/quipu/commit/61257af18a12d311fe140ef3010af5c19aa49e9d))
- *(patents)* The account gate is ~30 minutes, not days([8110039](https://github.com/scbrown/quipu/commit/8110039ee2bbe58fde878925aa0f35baec6f8f49))
- *(patents)* Fee confirmed at $520 total, and file B first([9255671](https://github.com/scbrown/quipu/commit/925567181b2236febe12e54ff7a27fb7f7be9f06))
- *(patents)* Record the filing — A is 64/135,410, filed 2026-08-17([48486cd](https://github.com/scbrown/quipu/commit/48486cd6200f0008d7627137da8029550d1ca7cf))
- *(agents)* Work and push directly to main([3ad7e25](https://github.com/scbrown/quipu/commit/3ad7e25877864331aceb0200712558d9c307f21f))
- *(design)* Golden-path blessing pipeline (design only)([7a85a20](https://github.com/scbrown/quipu/commit/7a85a20c8ac244746042cf280621270ac0b2d269))
- *(paper)* Cite ActiveGraph (Nakajima, arXiv:2605.21997) in related work([f1be5c7](https://github.com/scbrown/quipu/commit/f1be5c7cff8b85da42f730c29def0e72282cad3c))
- *(design)* Fork-at-any-event — semantics, gate routing, v1 scope([f4e6f5e](https://github.com/scbrown/quipu/commit/f4e6f5ea4c9e89673b8399b6fdfde6212ee910c4))
- *(design)* Named-graphs — retire the stale /knot deferral, scope the event-log claim([647db1c](https://github.com/scbrown/quipu/commit/647db1c125f7a63c9a6cee66121cb4a7456b6b16))
- *(design)* Multi-db-composition — aliases are built; GRAPH <attached-iri> is served([b8fb055](https://github.com/scbrown/quipu/commit/b8fb055f29fa46fe7956b267f7ddca9cea463582))
- *(design)* Policy-edit-hooks — a refused write now leaves a write.refused trace([6e0bd04](https://github.com/scbrown/quipu/commit/6e0bd047168f859d6af748de7a366afa2f46cce3))
- *(design)* Correct two banners the code outran — federation routing, read-model phases([b856018](https://github.com/scbrown/quipu/commit/b856018261397529edc14b3f872d917e21b0311c))
- *(book)* Document the fork surfaces and the graph/fork query params([ef76567](https://github.com/scbrown/quipu/commit/ef76567b0d3513e7dd50cdd3bb70edbb1422a509))

### Fixed

- *(lint)* Unnecessary_sort_by in rank write-back, red on CI's newer clippy([e4e565c](https://github.com/scbrown/quipu/commit/e4e565c4dcbf9866ff82117738b01faff3f9261c))
- *(governance)* Project ?layer in the placement SELECT; refuse escalating effects outside the escalation class([0328775](https://github.com/scbrown/quipu/commit/0328775c24747d9c647f64a285a42065d2b2c329))
- *(governance)* Re-mint expired DecisionRequests; accept only signed decisions from registered deciders([0cc3a05](https://github.com/scbrown/quipu/commit/0cc3a05a8d40b25e56ef9ac1ba697ee32d6b211d))
- *(labels)* Compare the cached trust rank in the drift sweep and verify it at read([6e59557](https://github.com/scbrown/quipu/commit/6e5955700b79182849deca1cbd3e4069c86b7b26))
- *(just)* Serve-fixtures was missing the server feature quipu-server requires([529b8b0](https://github.com/scbrown/quipu/commit/529b8b0d65dfbf87ef86f625ee42a34506436a9a))
- *(server)* Quipu-server did not compile under --features full([f5d9041](https://github.com/scbrown/quipu/commit/f5d90419bb261ac0fd08f5ca6b6c4fd6801f2956))
- *(resolution)* Honour quipu:distinctFrom, read the composed store, resolve a write in one pass([34bf864](https://github.com/scbrown/quipu/commit/34bf86469386a4ad2f64238cf52a562775a0deb0))
- *(shacl)* Graph-scope the store-context repair for named-graph knot writes([d0c24b7](https://github.com/scbrown/quipu/commit/d0c24b7f76982b9f7ca7fc06e862174da6ea6950))

### Miscellaneous

- *(beads)* File five governance-plane bugs found by doc-vs-code review([0aa1826](https://github.com/scbrown/quipu/commit/0aa1826b369ca9ec96b7e49b1c36bf5f603dd9e4))
- *(beads)* Export issues.jsonl with the five governance-plane bug beads([4f4c560](https://github.com/scbrown/quipu/commit/4f4c5605cf96068e018654ec61a99e591844b9a2))
- *(beads)* Export tracker state — all nine open beads closed([bb2d68e](https://github.com/scbrown/quipu/commit/bb2d68e0c6dd0c963c1f24c01f886312b0ccef0d))
- *(beads)* File progressive-disclosure and work-item-containment work (jsonl export)([5ea6d22](https://github.com/scbrown/quipu/commit/5ea6d22cdd70369696f3681d2b383fa49de479ae))
- *(beads)* Close the disclosure work that landed (jsonl export)([53b82c2](https://github.com/scbrown/quipu/commit/53b82c228fcdaabc882d16733541ff855ab1cdcf))
- *(beads)* Close quipu-oql — the read pool already fixed it (jsonl export)([0c008bb](https://github.com/scbrown/quipu/commit/0c008bbc63d316effed64d1b4c8dbf43a7337dc3))
- *(beads)* File golden-path gap beads (jsonl export)([ba3a003](https://github.com/scbrown/quipu/commit/ba3a003a38786a41fe8f1df232ff804ef9abbd2d))
- *(beads)* Close quipu-gp1..gp4 (jsonl export)([469c7f6](https://github.com/scbrown/quipu/commit/469c7f6e77482ef55504164ceea20817c90d5f56))
- *(beads)* File quipu-gp5 — fork-at-any-event ergonomics (jsonl export)([b5a9168](https://github.com/scbrown/quipu/commit/b5a91685bf78f0dd34a901b636e9a905203e8c96))
- *(beads)* File quipu-er1..er4 — entity resolution defects (jsonl export)([7b4f01b](https://github.com/scbrown/quipu/commit/7b4f01b016808d23759ddd6bf07023a1778a35d0))
- *(beads)* Close quipu-er1..er4 (jsonl export)([004e3d2](https://github.com/scbrown/quipu/commit/004e3d222ca1b03be2a9a69587bd94859f2f7139))
- *(beads)* File quipu-080 — SHACL type-context is ROOT-only for named-graph writes (jsonl export)([bc11cc8](https://github.com/scbrown/quipu/commit/bc11cc80ede13db4643dcb7b7ff55750deccfbb1))
- *(beads)* Close quipu-080 — SHACL repair context is now graph+ROOT scoped for named-graph knot writes (jsonl export)([fd3d6cd](https://github.com/scbrown/quipu/commit/fd3d6cd1270b6adce7d410aac9deb131bb09fbce))
- *(beads)* Close quipu-gp5 — persistent named forks with gate-routed promotion (jsonl export)([a7082cf](https://github.com/scbrown/quipu/commit/a7082cf9c0c4208352f23347a31b73d686055395))
- *(beads)* Close quipu-0d3 — write refusals recorded as queryable write.refused events (jsonl export)([37bfc06](https://github.com/scbrown/quipu/commit/37bfc06a33081d26177cd7176de45c7cab3647e8))

### Style

- Cargo fmt, and a missing final newline in three configs([c27e00f](https://github.com/scbrown/quipu/commit/c27e00f9eb8c31b52aa855556ce1edadfbf91d83))

## [0.3.23] - 2026-08-12

### Added

- *(portability)* Wasm-safe clock shim and std::fs gating (quipu-gsg)([8d573b8](https://github.com/scbrown/quipu/commit/8d573b8ce60b238a7621410f2606843e65e4c744))
- *(read-model)* Per-graph read models scoped to the derived layer (quipu-nip)([02c9a33](https://github.com/scbrown/quipu/commit/02c9a3354ef07688f87db18365b5c11143846615))
- *(graph)* PageRank write-back and PPR re-ranking (quipu-mq7, phases 3-4)([4aaad19](https://github.com/scbrown/quipu/commit/4aaad19ce3b529e21a2dba2110c913b3dbede21a))
- *(graph)* Temporal and counterfactual PageRank (quipu-bli, phase 5)([2b01ac1](https://github.com/scbrown/quipu/commit/2b01ac1e7b34cf6a940ef583c1565518d3ea393e))

### Documentation

- *(book)* Pages for the six shipped subsystems the book omitted (quipu-a08)([885f811](https://github.com/scbrown/quipu/commit/885f811626e5e9f0fdac732d57d3e37835cdf882))

### Miscellaneous

- *(ui)* Remove the unwired Leptos scaffold, record the decision (quipu-dzd)([1c35f8b](https://github.com/scbrown/quipu/commit/1c35f8b14a513ee8a3f1b031cc418d0b87fe8b9f))
- *(beads)* Triage quipu-qd2 as blocked on a wasm runtime([2df5635](https://github.com/scbrown/quipu/commit/2df5635e8d5f91276bd847308950d63910c7511b))

### Perf

- *(store)* Drop the never-chosen idx_eavt fact index (quipu-fcg)([e37eb74](https://github.com/scbrown/quipu/commit/e37eb7430b9845a8072bab7d72bb8373d62867be))

## [0.3.22] - 2026-08-12

### Added

- *(graph)* Scope project() to a named graph and memoize it (quipu-tz5)([7c35d8f](https://github.com/scbrown/quipu/commit/7c35d8f98024709d56c00a061123a30d11f0ca5e))
- *(events)* Opt-in event-log retention that honours consumer offsets (quipu-9z9)([77c0d68](https://github.com/scbrown/quipu/commit/77c0d6876961d47084ea0e651cfc36ee8c551c81))
- *(mcp)* Advertise the seven REST-only governance/overlay tools (quipu-227)([a717057](https://github.com/scbrown/quipu/commit/a7170578cdb010df921050bc2b59b31a4eca59e4))
- *(federation)* Route queries through the federated provider (quipu-tkh)([6d52884](https://github.com/scbrown/quipu/commit/6d52884db1d165b19a3e1aae2c3c7ebe272f8d31))

### Changed

- *(store)* Split mod.rs and ops.rs along their seams (quipu-bu3)([f459f57](https://github.com/scbrown/quipu/commit/f459f57eccc7b320cd1338617a351f63100a64a2))
- Green the file-size full scan — 6 files back under baseline (quipu-sd1)([835c9c8](https://github.com/scbrown/quipu/commit/835c9c8315c83006017017aed9a4b7c1f79b0f7d))

### Documentation

- Catch the stale banners and rosters up to the code([1e9ab04](https://github.com/scbrown/quipu/commit/1e9ab0464f9738ebc393aa2e123e0a5fa69b2259))
- *(book)* Document the 11 CLI commands the reference omitted([21d0078](https://github.com/scbrown/quipu/commit/21d0078e71970fe7699d82e347e880a612998754))
- *(book)* REST reference covers every route, pinned by a test (quipu-83v)([6f465d2](https://github.com/scbrown/quipu/commit/6f465d20de58a3e23a7e04145e634f8345930afd))

### Perf

- *(sparql)* LIMIT pushdown and selectivity-ordered joins (quipu-0lr)([bc4eec6](https://github.com/scbrown/quipu/commit/bc4eec6f69c0cd35c2d7a30679c33c0d79e04457))

## [0.3.21] - 2026-08-10

### Documentation

- Add the Zenodo DOI badge([964accb](https://github.com/scbrown/quipu/commit/964accbf1f03126b721494da88cbab978df08fe0))
- *(paper)* Cite the archived artifact DOI for the census numbers([313eeec](https://github.com/scbrown/quipu/commit/313eeec2b02087f33a52d80f47b174c85e74eb7b))
- *(paper)* Drop the Draft label from the title page([6cb012b](https://github.com/scbrown/quipu/commit/6cb012b4140893915b4bb31c7647389346749a66))
- Record the author ORCID, and catch CITATION.cff up to v0.3.20([cd7d12a](https://github.com/scbrown/quipu/commit/cd7d12abd7908c8c9274f2f21334e781b6652d35))

### Fixed

- *(paper)* Stop the figure boxes colliding([42adcd2](https://github.com/scbrown/quipu/commit/42adcd28ae18823821b4696335ab9c024201909c))

## [0.3.20] - 2026-08-10

### Added

- *(features)* Gate the HTTP stack behind `server` and `remote`([e05963a](https://github.com/scbrown/quipu/commit/e05963a9f614e5fe7680ddeace869ec80071a716))
- *(store)* In-memory read model over one graph's current facts([72ffa13](https://github.com/scbrown/quipu/commit/72ffa1316e77a7aea1ef307a719652ab75c55ddd))
- *(store)* Resident read model and the scope guard([848824f](https://github.com/scbrown/quipu/commit/848824fe6544f4ec850cb492d744eb325fbd04b5))
- *(sparql)* Model-backed pattern evaluation, off by default([36ae6f0](https://github.com/scbrown/quipu/commit/36ae6f0a42247fda744b824baf9a6fb92c17f97d))
- *(census)* Benchmark skeleton and committed-graph registration([f46829c](https://github.com/scbrown/quipu/commit/f46829cc4ab46e678672df3f029631f346e33a9e))
- *(census)* Execute phases 2-4 - recording, correction, composition([1668e8a](https://github.com/scbrown/quipu/commit/1668e8a882b21db1c3a78fd75f8a9ec21eeea6d8))
- *(census)* Phases 5-6 - amendment, as-of replay, in-store audit([c76c869](https://github.com/scbrown/quipu/commit/c76c869bd1fcab0fced74e4735762836040aab77))
- *(census)* External SARC checker arm (CEN-X1)([fd20532](https://github.com/scbrown/quipu/commit/fd20532f24b550ee34b61104106d76291e0e027e))
- *(census)* Census-in-the-wild - a real hank trace through the audit([fd8038c](https://github.com/scbrown/quipu/commit/fd8038c29e40856d614e6e5acdbb1293a6e6c493))
- *(census)* Agent arm - external writer against the gate([32eadc9](https://github.com/scbrown/quipu/commit/32eadc99839574d72587d2e34f007e29f6037e01))
- Run DEMM-Bench against quipu as a ninth evidence regime([38aced2](https://github.com/scbrown/quipu/commit/38aced23802c0b1c2d69c9698ccfff4629883e26))
- *(governance)* Seal verdict attribution into the signed evidence hash (Q-VERDICT-ATTRIB)([674dec3](https://github.com/scbrown/quipu/commit/674dec3398acd6fa1c13602a863371571f433ea5))
- *(census)* Repeat the agent arm across four models, three trials each([9346609](https://github.com/scbrown/quipu/commit/9346609a27d15110ec6bf47e1affb932c3b40060))

### Documentation

- Wasm support design — measured limits and the join ceiling([5859201](https://github.com/scbrown/quipu/commit/5859201b865ebddc30c86bb14b3c997ffc27d586))
- In-memory read model — query in memory, write to SQLite([f2e6a8e](https://github.com/scbrown/quipu/commit/f2e6a8e9aec4f291add87d86149ab162bbb76cda))
- Retract the project() superlinearity claim — it was cold-cache([2de8be7](https://github.com/scbrown/quipu/commit/2de8be767d3bd69f949daf6ea35df4e43685dc2a))
- *(design)* Add paper plan for a governed-bitemporal-store paper([3e8f515](https://github.com/scbrown/quipu/commit/3e8f5150b76b22c51bde408de8c18f7672e9d89b))
- *(book)* Document the in-memory read model([20c4147](https://github.com/scbrown/quipu/commit/20c414734579fd74b3d0469235c3f602c693494f))
- *(design)* Refocus paper plan on governance, bitemporality, strictness([83b4366](https://github.com/scbrown/quipu/commit/83b436630c42d6f132f23efda4b9cf034030831c))
- *(design)* Raise paper plan to contract + system + benchmark([bf8e5fd](https://github.com/scbrown/quipu/commit/bf8e5fdb95888101f0dd5199f71e31a2a3521c84))
- *(design)* Reframe paper plan as system-first([a95164c](https://github.com/scbrown/quipu/commit/a95164c912afac2b37288a457fe6680db1734bd0))
- *(design)* Add defaults comparison and GS1-GS6 principles page([81cab7d](https://github.com/scbrown/quipu/commit/81cab7d2fa159c7bc1262bbaeaa6ba1471a109e3))
- *(census)* Determinism note with measured hashes([107f57c](https://github.com/scbrown/quipu/commit/107f57c65b4850a9aee7f268bb8901077a94645e))
- *(paper)* LaTeX source - full draft from measured results([7b1fb2b](https://github.com/scbrown/quipu/commit/7b1fb2b0dda8b504208d141de642bf3d054dd4e1))
- *(paper)* Scalable fonts for pdflatex; TeX in the session hook([5226397](https://github.com/scbrown/quipu/commit/522639750c064157084ec893eb0710f08d385947))
- Change paper author to Steve Brown([1fc4742](https://github.com/scbrown/quipu/commit/1fc47426f0e946af626bdc69a55e0edfa8c108f9))
- *(paper)* Figures, tables, and the impartiality framing for the DEMM run([4bbe270](https://github.com/scbrown/quipu/commit/4bbe270a0585e31a347e4b106b63a1295bcda710))
- *(demm)* Package the upstream DEMM-Bench regime contribution([59f95a5](https://github.com/scbrown/quipu/commit/59f95a5bfe5162a27f7234bd9437be08d2ed2ac6))
- *(paper)* Cite the SARC successor line and gloss the audit notation([4d72cbe](https://github.com/scbrown/quipu/commit/4d72cbed39659ce8dca7edeebda49c31cf33d2ea))
- *(paper)* Cite ForCoding as practitioner-side D4 convergence([8b2210d](https://github.com/scbrown/quipu/commit/8b2210d64c83f864daf8ed36d10ba022510c4e94))
- *(paper)* The governed structural writer is now Yupana([99e996e](https://github.com/scbrown/quipu/commit/99e996e0f4e9eea0bbf2d567cf857e297a77e208))
- *(paper)* Footnote the stack's deployments, kept light([c8f4760](https://github.com/scbrown/quipu/commit/c8f47602e532e186d35d8181c4fc7654ba0dc4ce))
- *(paper)* Address adversarial-review findings([f132da2](https://github.com/scbrown/quipu/commit/f132da2e8b983e14f98a8f1cbc446963ce358d60))
- *(design)* The signing plane — governing the trust root like everything else([fd96239](https://github.com/scbrown/quipu/commit/fd962394d08664be702a6ca0ae8fffc6d3ee5b3b))

### Fixed

- *(ci)* Make the file-size check a working ratchet([89612a9](https://github.com/scbrown/quipu/commit/89612a90898d308d4e71e9029796b0cdc25df41d))
- *(deploy)* Carry the 'server' required-feature in FEATURES and the t1u2h gate floor([c5a4755](https://github.com/scbrown/quipu/commit/c5a475598295d47b6180fea15757e774f7e0d8d8))
- *(demm)* Match baseline semantics to the DEMM-Bench paper and temper the paper claim([cea41f7](https://github.com/scbrown/quipu/commit/cea41f71abe2e1fef102a25368205f65d8df4867))
- *(hooks)* Exclude the upstream demm patch from whitespace fixers([a956fe9](https://github.com/scbrown/quipu/commit/a956fe9567a2540358b75e5fd48846b7ae1e9073))
- *(paper)* Render dashes that were silently dropped from the PDF([f825dc9](https://github.com/scbrown/quipu/commit/f825dc9d2085ff8c8c9a043f370e246c9c167545))
- *(ci)* Correct release changelogs automatically, and fix the 1.97 clippy lint([2bb770a](https://github.com/scbrown/quipu/commit/2bb770a1fa25e97e152ee3aec71f7339dddad59f))

### Miscellaneous

- *(beads)* Tidy bd init artifacts([1b080cf](https://github.com/scbrown/quipu/commit/1b080cf87f0090a81740df9cd38ca009df929770))
- *(claude)* Install docs toolchain and build quipu in remote sessions([7caa122](https://github.com/scbrown/quipu/commit/7caa122df5f3bd29b03e1afef429594418b07e42))
- Add citation and Zenodo deposit metadata([d0d5fb1](https://github.com/scbrown/quipu/commit/d0d5fb1d33ba6620d452436b1cde00a99bf55c02))

### Perf

- *(store)* Memoize the term dictionary([0f7d363](https://github.com/scbrown/quipu/commit/0f7d363906e66cdfb7b5dea0c16feb7b057689f4))
- *(store)* Bound the term cache([ed4fcdd](https://github.com/scbrown/quipu/commit/ed4fcddbd801d367fe6553b6e0e8fc6dddaa7157))
- *(sparql)* Hash-join BGPs through the read model, on by default([8872abd](https://github.com/scbrown/quipu/commit/8872abd3b07be1161c79f8316377dedafaec8b06))

## [0.3.19] - 2026-08-07

### Added

- *(server)* Expose the OWL ontology engine over REST as POST /ontology([c40e86e](https://github.com/scbrown/quipu/commit/c40e86e704eb28bb1c5a52a60851677c299dd7dd))
- *(owl)* Materialize rdfs:subPropertyOf — it was parsed and then dropped on the floor([2bbcd4b](https://github.com/scbrown/quipu/commit/2bbcd4b05a6f571be9fd0fd84a1df8cd905b4f2c))
- *(owl)* Wire Ontology::validate() into the write path, and correct the doc that claimed it already was([91d94e1](https://github.com/scbrown/quipu/commit/91d94e14d663bdbf2a430c3bb392617a3f0792fb))
- *(server)* Register the reactive reasoner so derived facts stay fresh on write([9c04b20](https://github.com/scbrown/quipu/commit/9c04b20026bac0badbf9a7e08e8a9e2e7e3269bc))
- *(shapes)* Land the declared class hierarchy, and say what actually makes it work([fd05e71](https://github.com/scbrown/quipu/commit/fd05e71f889b7c14ab42e6bbe220df0d239cc050))
- *(shapes)* Govern Host, OperationalRule and Capability([ec1f082](https://github.com/scbrown/quipu/commit/ec1f0825664015ea38e274a0566b9f1738be8c8e))
- *(shapes)* Govern CodeModule in_repo as a walkable IRI([302080c](https://github.com/scbrown/quipu/commit/302080c750238c6325e06f01c499af6eb73c395f))
- *(shapes)* Govern Service and Tool, and make I8 fatal([8c627de](https://github.com/scbrown/quipu/commit/8c627deb3238c1a939563697dc9cf4cab41f39b5))
- *(shapes)* Commit is subsumed by GitCommit, as a LIVE rule not a one-shot materialization([c8c4d53](https://github.com/scbrown/quipu/commit/c8c4d53532e46761943ceb592e123cc75c7eccce))
- *(metrics)* Attribute request time to the CALLER, and make restarts visible([26d3027](https://github.com/scbrown/quipu/commit/26d3027f3c67072eebafd34bd667a79c6cfda0fa))
- *(metrics)* Separate store WAIT from store HELD, so causation stops reading as suffering([43e5c20](https://github.com/scbrown/quipu/commit/43e5c20a9e24334c05204bfb91da26133e9c4284))
- *(shapes)* Require rdfs:label on the four code-entity classes([2c3badb](https://github.com/scbrown/quipu/commit/2c3badb22f06fc541d2fba717d9f9a622cef3459))
- *(episode)* Report WHAT an ingest did, so an idempotent retry stops reading as a failure([f323477](https://github.com/scbrown/quipu/commit/f32347754e3ba1276ac652b5c9d51cd3bd5872fd))
- *(store)* Term spaces — the registry and space-aware allocation (quipu #74)([ae19939](https://github.com/scbrown/quipu/commit/ae199392eb06f8c39f787ee9dc4e5de66ad3cbd8))
- *(store)* Quipu db respace — offline term-space remap (quipu #74)([41ca407](https://github.com/scbrown/quipu/commit/41ca4078696277d2daf32cf1ba75fd5fe91f9fba))
- *(store)* ATTACH read-only layers — mount, verify, register (quipu #75)([8ead674](https://github.com/scbrown/quipu/commit/8ead674dd89dd9e60eeb0fe07e89f5b5e7864455))
- *(store)* Facts_source union — composed reads over attached layers (quipu #75)([df4fffb](https://github.com/scbrown/quipu/commit/df4fffb35d7c58689ce168af8cf6f44f2ad89441))
- *(store)* Term aliases — the alias table, lookup_all, and the adversarial fixture (quipu #76)([958d028](https://github.com/scbrown/quipu/commit/958d0288f8f78dc1cb3081cc259fe0303519c5c5))
- *(store)* Resolve aliases across query term spaces (quipu #76)([ab5235b](https://github.com/scbrown/quipu/commit/ab5235b814ef3e739e58f6d50364634a5f01d110))
- *(store)* Import graphs with eager term remap (quipu #85)([e546365](https://github.com/scbrown/quipu/commit/e5463652bd1fdf08f73e534303537590aff7e2b1))
- *(pack)* Unpack and surface attached manifests (quipu #82)([435e86c](https://github.com/scbrown/quipu/commit/435e86cdd89b5014dfb503403f093cd61a5ed33b))
- *(pack)* Verify attachments and warn on embedding drift (quipu #82)([603809a](https://github.com/scbrown/quipu/commit/603809ad2589e1bac88cd3c8ec9eff2c3f68b8b3))
- *(store)* Fail loud on cross-db transaction time (quipu #77)([b937edf](https://github.com/scbrown/quipu/commit/b937edf32ed01c2190351560852c65d5ae16d2ed))
- *(sparql)* Scope property paths to named datasets (quipu #36)([32c5ac8](https://github.com/scbrown/quipu/commit/32c5ac8d4c0d250677596ad11797d3a5db709a25))
- *(reasoner)* Evaluate within one named graph (quipu #36)([743ec21](https://github.com/scbrown/quipu/commit/743ec21914aaac4bc895be945708f2cf3f9b9dd7))
- *(episodes)* Support atomic snapshot replacement([d63b714](https://github.com/scbrown/quipu/commit/d63b714943d3f0939578890d1a275097804e5d0d))
- Atomically replace knot snapshots([628d015](https://github.com/scbrown/quipu/commit/628d0151342e1c238f41139987fc30f23132589a))
- *(labels)* Add durability and fact derivation methods([93f9a92](https://github.com/scbrown/quipu/commit/93f9a920fca1726bd3a6f4900fa999a95c707304))
- *(shapes)* Govern text rules([a5eb73b](https://github.com/scbrown/quipu/commit/a5eb73b5d133ec546b30a32b456ef8c6f3df0a47))
- *(owl)* Author functional and disjoint axiom sets([6d58d9a](https://github.com/scbrown/quipu/commit/6d58d9a8d2afcf357a970e6cce4ce2a702176cdf))
- *(owl)* Author safe topology range axiom([1c59bcc](https://github.com/scbrown/quipu/commit/1c59bccba6d9f5515bf0f2338fc182894fdfb631))

### Changed

- *(store)* Remove superseded attached-only refusal (quipu #76)([f3f017a](https://github.com/scbrown/quipu/commit/f3f017a510d07e9667f9deddf22e59ef9bfe380e))

### Documentation

- *(design)* Statement identity, edge properties, and bounded paths([905d12e](https://github.com/scbrown/quipu/commit/905d12e75323c21a327ef7682fcaa449a1200753))
- *(rest-api)* Document the alias write path, /set, and the audit-trail params([4c31f33](https://github.com/scbrown/quipu/commit/4c31f331d9f0213e0adb5b6316d801971f3529a7))

### Fixed

- *(version,deploy)* Report every compiled feature, and refuse a featureless binary([44af8a7](https://github.com/scbrown/quipu/commit/44af8a7565b423c430f36ae14544f5e535ab5f4b))
- *(deploy)* Anchor the feature stamp on a versioned marker — the bare pattern matched binary noise([45a84b2](https://github.com/scbrown/quipu/commit/45a84b29cc3af1708179ba29573063235a4ec251))
- *(owl)* Carry owl.validate_on_write from config into the store — the flag was unreachable([d7510d7](https://github.com/scbrown/quipu/commit/d7510d7493037ab82a2f9ed761ed7feeb4581e01))
- *(owl)* Supersede a functional property on update instead of rejecting it([50b117a](https://github.com/scbrown/quipu/commit/50b117aa844cb001ecdb28b3ed9bc2e6ab513232))
- *(episode)* Resolve foreign-vocabulary edge predicates, refuse what cannot be represented([73928f3](https://github.com/scbrown/quipu/commit/73928f3cc30593252ccecc5c4874a884074326bf))
- *(ci)* Clear the two clippy lints that have held main red for ~17h([a50e268](https://github.com/scbrown/quipu/commit/a50e26877a2d1bee4f53a595e9c0e7c6b848b5da))
- *(episode)* Refuse a comma-separated node type instead of minting a junk class([2294491](https://github.com/scbrown/quipu/commit/229449178d74b75e650efa0f0aec8c911ed588ea))
- *(reasoner)* Constants in body atoms are a SELECTION, not an error and not a no-op([dd7860c](https://github.com/scbrown/quipu/commit/dd7860c009e4e0059f524e81adb2817021b60b8c))
- *(server)* An auth refusal must SAY it refused — 401/403 returned a zero-length body([4b4eee8](https://github.com/scbrown/quipu/commit/4b4eee81c7b7c6bef1763d99589b384a021e3ecf))
- *(server)* Scope the /project explanation to /project([1760904](https://github.com/scbrown/quipu/commit/1760904dc6cd02220212a4590f0bd0dfbab74c5b))
- *(shacl)* Validate a write against the store, not just its request body([7e29558](https://github.com/scbrown/quipu/commit/7e29558af53f95157c5bf9618060b35ac97e8733))
- *(lint)* Backtick SQLite in read-pool doc comments, alias the tool-case tuple([0be1d5a](https://github.com/scbrown/quipu/commit/0be1d5aa3f96365b8b703111f4dd5ac662c9011a))
- *(ci)* Restore the --no-default-features build and clear -D warnings([c7221f8](https://github.com/scbrown/quipu/commit/c7221f8f9cb403ef5fa3eb03ddd6c14f7c8ac9d1))
- *(sparql)* Keep alias dedup linear on wildcard scans([48d905b](https://github.com/scbrown/quipu/commit/48d905b944360c23e5148b1ce91104bcadc8b25c))
- *(derivation)* Satisfy full-feature lint([b7e0c56](https://github.com/scbrown/quipu/commit/b7e0c56ad7b0ea6b3e2b8750ffc9fb5389ce4212))
- *(owl)* Apply domain and range on new writes([f1943b5](https://github.com/scbrown/quipu/commit/f1943b5a4577a6ab720073e6e261a2809d9dd8dd))
- *(ci)* Compile staged writes without owl([a19536f](https://github.com/scbrown/quipu/commit/a19536fd868ee2b6fe53d950285561998b7e5982))
- *(graph-view)* Bound edges with a budget — node cap alone no longer bounds the payload([3831b2a](https://github.com/scbrown/quipu/commit/3831b2a48f69ea4c3b33d998151309fb0089b498))

### Testing

- *(shacl)* Pin the subset property on an episode-shaped payload([d7849b0](https://github.com/scbrown/quipu/commit/d7849b0003dc45c0ab5a6c81b4fb5ea4f1474cc9))
- *(shacl)* Label the valid code-entity fixtures per the tightened shapes([c2d0929](https://github.com/scbrown/quipu/commit/c2d092983227eabf4102b74866d17049d2ecc663))
- *(shacl)* Make the code-entity negative tests prove WHICH constraint fired([779512f](https://github.com/scbrown/quipu/commit/779512f7d90106b2bd1251599297d448cb9840ee))
- *(shacl)* Pin root shapes across named graphs (quipu #36)([872de7b](https://github.com/scbrown/quipu/commit/872de7beb08400f58009b6b5f09b642cbf90b681))
- *(episodes)* Pin snapshot removal with external refs([211e876](https://github.com/scbrown/quipu/commit/211e87612e0cd74cb08593bf779a7be4c0731e02))

### Perf

- *(server)* Serve reads from a read-only connection pool([2f9ee0b](https://github.com/scbrown/quipu/commit/2f9ee0b05ac169ec74353d7a6974fa8e484fb828))

### Style

- Rustfmt src/server.rs([60e9c73](https://github.com/scbrown/quipu/commit/60e9c73b3701737e4979ae9a7eb034d44046df2a))
- *(metrics)* Cargo fmt + clippy doc_markdown([4b55218](https://github.com/scbrown/quipu/commit/4b552189c17b616ef289c21592e0861b708a39ad))
- *(episodes)* Apply stable rustfmt([14e6628](https://github.com/scbrown/quipu/commit/14e66287ac3d491c6e18d3fb6f561a2eeabcf7cb))

## [0.3.18] - 2026-08-03

### Added

- *(scripts)* Ingest sibling repos as code and doc entities([f44e79d](https://github.com/scbrown/quipu/commit/f44e79db801ae0bad677229db7342152c404359f))
- *(ui)* 3D Datalinks view over the prerequisite DAG([1a83e48](https://github.com/scbrown/quipu/commit/1a83e48037898050b11fbf2fee0c6903e58ac521))
- *(docs)* Publish the Datalinks demo to GitHub Pages([1ca9832](https://github.com/scbrown/quipu/commit/1ca983226eb27931c693fcdd38ab9f488f12f94f))
- *(governance)* SARC constraint metadata and class-placement conformance([fd3e6f8](https://github.com/scbrown/quipu/commit/fd3e6f8ac3bbf7d4482988ec8d1ebc1f652a6da1))
- *(governance)* Persist the write-gate verdict as a signed fact([5e7a316](https://github.com/scbrown/quipu/commit/5e7a316126c9c8fdbfcf268fb4c1d88b9e351b4f))
- *(governance)* Escalation router — bounded, actionable human oversight([fbf8be7](https://github.com/scbrown/quipu/commit/fbf8be7c2b013dfea84440893dddc4635a11e8da))
- *(governance)* Authority intersection over named graphs (SARC I5)([01fef79](https://github.com/scbrown/quipu/commit/01fef793a825c8eb83723c5e6ef78826cf27f061))
- *(governance)* The T ⊨ Σ audit checker, and a CLI to run it([888cac9](https://github.com/scbrown/quipu/commit/888cac967ffe5dc9b2a801086ecf0beff9e91c7b))
- *(governance)* The dispatch-graph inventory (SARC I7)([ac60931](https://github.com/scbrown/quipu/commit/ac60931aff48377c6d50f87b406123f511b279bc))
- *(governance)* Check the claimed hosting layer against the record (I6)([0847114](https://github.com/scbrown/quipu/commit/084711461bb22faa69185b00cdd2fac4aedaa48a))
- *(governance)* Replay — what a window says about promoting a rule([473aa8e](https://github.com/scbrown/quipu/commit/473aa8ed8be8d9625664a75ddd3a8ee8c8e509f0))
- *(governance)* Reassemble the attribution tree (SARC §9.5)([cab967f](https://github.com/scbrown/quipu/commit/cab967f320240b5df72a8be5221f25e8a4a581de))
- *(governance)* Constraint inheritance and trust boundaries (SARC §9.5)([6a7a9a8](https://github.com/scbrown/quipu/commit/6a7a9a831bc49ca2f4f22e2f6bbf17349901b26d))

### Documentation

- *(ci)* The changelog gate is dark per HEAD-COMMIT author, not per PR author([9be5a84](https://github.com/scbrown/quipu/commit/9be5a8490bbeaf377733fc912b673ab9e6426586))
- *(ci)* A hand-corrected release PR is discarded, and every generated one is bad([03e3231](https://github.com/scbrown/quipu/commit/03e323194d4b578dead277ef938469cd7f2c90e7))
- *(design)* Spatial explorer for Quipu graphs([72d09b8](https://github.com/scbrown/quipu/commit/72d09b8dabe15e2354dd2885f43be8b7634b3080))
- *(design)* SARC gaps as Q-SARC-* backlog beads([0ac14bc](https://github.com/scbrown/quipu/commit/0ac14bce5d88d2e71dc5aa6b699a2de697a1c985))
- *(design)* Cite SARC properly in the conformance section([efa2a4c](https://github.com/scbrown/quipu/commit/efa2a4c1a46baafc53be247c77a9d06c0fff8a74))
- *(shapes)* Describe the governance plane's vocabulary (Q-SARC-VOCAB)([a795998](https://github.com/scbrown/quipu/commit/a79599878f7b6ced4c29860c14eeb0f8549d93ac))
- README and backlog catch up with the governance plane([01be9a5](https://github.com/scbrown/quipu/commit/01be9a5de4214b930e1f83515f4b8b151069b1cf))

### Fixed

- *(release)* Take the version baseline from our git tags, not a stranger's crate([fbfe1b5](https://github.com/scbrown/quipu/commit/fbfe1b57cf1f88220498b26794278439a22072a9))
- *(governance)* Enforce the safety enums on the write path, and describe the vocabulary([bc1ffaa](https://github.com/scbrown/quipu/commit/bc1ffaab089c666f602dac007db0bc0c57599580))
- *(changelog)* Regenerate the 0.3.18 section — release-plz dropped 8 commits([babb13a](https://github.com/scbrown/quipu/commit/babb13ae28adbbc10e8c6625562e73df79ffeaff))
- *(clippy)* Inline the format arg CI's --all-targets clippy rejected([8562c30](https://github.com/scbrown/quipu/commit/8562c30ed89c7b3a697df34e44841b7bdfa9e0a4))

### Miscellaneous

- Release v0.3.18([330fd08](https://github.com/scbrown/quipu/commit/330fd0807be060aa7a6c11d863f1faa68d923864))

### Reverted

- *(ci)* The release-pr/release reorder did not fix the changelog dump([792a9a2](https://github.com/scbrown/quipu/commit/792a9a29427f2a7b6bbe9812cbf5b11dffd8aa32))

## [0.3.17] - 2026-08-02

### Fixed

- *(ci)* The changelog guard never ran, and could not have failed if it had([7e3e503](https://github.com/scbrown/quipu/commit/7e3e5033da2149bda01903f8bd381319aa959514))
- *(ci)* Narrow the changelog exemption that let real commits go undocumented([d272576](https://github.com/scbrown/quipu/commit/d272576bafd742456bcbcec0ac06e74d3e635662))

## [0.3.16] - 2026-08-02

### Added

- *(sparql)* Announce rdfs:subClassOf inference in the query response([531098f](https://github.com/scbrown/quipu/commit/531098f6f6a3f90df25864804aef08987faa989e))
- *(shapes)* Shape SearchService, and catch the gap no count can see([c21c61d](https://github.com/scbrown/quipu/commit/c21c61dd4d89b0f96d131638f0fb758c29d15cb5))
- *(sparql)* Carry the subclass-inference marker on ASK, CONSTRUCT and the W3C shapes([f44afc9](https://github.com/scbrown/quipu/commit/f44afc918a54d7e2cc6ea3a59ae82f2876fe2c16))
- *(deploy)* Refuse to deploy anonymously, and log who + what is SERVING([ca2d0a0](https://github.com/scbrown/quipu/commit/ca2d0a05a1d877ae1ac83ec35f8792faf4a3dd51))

### Documentation

- *(rest-api)* Document POST /resolve([d2d32fc](https://github.com/scbrown/quipu/commit/d2d32fc6ca9f23348d414b1d49073827b60677e4))

### Fixed

- *(embed)* Chunk the deferred ONNX embed — unbounded batch pinned GBs of arena([64c49f7](https://github.com/scbrown/quipu/commit/64c49f7480af0f4dd4c51c25c6d1973d37e03f83))
- *(deploy)* The shapes gate reported a SHACL outage it had merely failed to authenticate to([ca55655](https://github.com/scbrown/quipu/commit/ca55655b0f31d2ad403a136565b3485f50f965b6))
- *(deploy)* The shapes gate's own failure path reported "HTTP 000000"([95f3319](https://github.com/scbrown/quipu/commit/95f3319298c6c636402ac48f06f751a02f6484d1))
- *(ci)* Green the Format and Markdown-lint legs on main([1be0d65](https://github.com/scbrown/quipu/commit/1be0d658ca6da217bf6afc1767c5e60048499e3c))
- *(ci)* Green the last two legs — a vendored file two hooks rewrote, and a feature-dependent wildcard([14ea79f](https://github.com/scbrown/quipu/commit/14ea79fd7db73cde37689552dc115be450965288))

### Miscellaneous

- Untrack the tool telemetry sidecar that made every build report git_dirty([4aba263](https://github.com/scbrown/quipu/commit/4aba263d4bb2c9b20da133f3ee3530b38d8ee354))

## [0.3.15] - 2026-08-01

### Added

- *(ui)* Draw the graph on canvas from one /graph payload([7a634b6](https://github.com/scbrown/quipu/commit/7a634b6dcba8d8965a42ac864898dc5a13726256))

### CI/CD

- Ship release binaries built from a `full` feature bundle([51adae0](https://github.com/scbrown/quipu/commit/51adae020f32b6c185587ead7f8a20f54ad90ff6))

### Documentation

- *(ui)* Fictional demo graph, and tidy the node-link layout([dd8463b](https://github.com/scbrown/quipu/commit/dd8463b0d545ec32f0637b4531b4dee7e7058ae1))
- Fix the Pages artifact path and publish the orphaned pages([5975328](https://github.com/scbrown/quipu/commit/5975328bddb2feb36c15fa08756bae98d5a23da2))

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
