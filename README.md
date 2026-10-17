<p align="center">
  <img src="assets/logo.svg" width="200" alt="Quipu logo — knotted strings forming a knowledge graph"/>
</p>

<h1 align="center">quipu</h1>

<p align="center">
  <em>🪢 Governed knowledge with a history your agents can query</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache-2.0"/></a>
  <a href="https://github.com/scbrown/quipu/actions/workflows/ci.yml"><img src="https://github.com/scbrown/quipu/actions/workflows/ci.yml/badge.svg" alt="CI"/></a>
  <a href="https://github.com/scbrown/caboodle"><img src="https://img.shields.io/badge/stack-quipu-8B5E3C.svg" alt="Part of the caboodle stack"/></a>
  <a href="https://doi.org/10.5281/zenodo.21878428"><img src="https://zenodo.org/badge/1201016929.svg" alt="DOI"/></a>
  <a href="https://arxiv.org/abs/2608.16813"><img src="https://img.shields.io/badge/arXiv-2608.16813-b31b1b.svg" alt="arXiv: 2608.16813"/></a>
</p>

<p align="center">
  <a href="docs/demos/agent-graph/"><img src="docs/demos/agent-graph/agent-graph.gif" width="900" alt="An agent records services in a terminal while the Quipu graph grows beside it. A guessed fact with no source is refused and the graph does not change; the corrected fact, citing its trace, is accepted; a valid-time read then answers what was true before a migration."/></a>
</p>

An agent records what it learns; with write checking turned on, Quipu checks every write.
A guess with no source is refused, the corrected fact keeps its provenance, and a valid-time read shows what was
true before a change. [How this demo is made](docs/demos/agent-graph/) ·
[terminal-only version](docs/demos/governed-memory-v0.13.0/)

**Quipu is a governed, bitemporal knowledge store for AI coding agents and the
people who run them.** It can check writes against your rules, records where each fact came from, and answers what was true when. *Bitemporal* means keeping both
when a fact was recorded and, when supplied, when it was true.

Write enforcement is opt-in: `shacl.validate_on_write` and `governance.enforce_on_write`
default to `false` ([configuration](https://scbrown.github.io/quipu/getting-started/configuration.html)).
The [60-second demo](docs/demos/governed-memory-v0.13.0/) shows an enabled gate,
a refused write, corrected provenance, and a valid-time read.

Quipu answers standard SPARQL 1.1 as a CLI, REST server and Rust library, with its own MCP server (`quipu mcp` or `quipu-server` over HTTP).

A [quipu](https://en.wikipedia.org/wiki/Quipu) is the Andean knotted-cord record: cords are entities, knots are facts.

## Why you would want it

- **Check knowledge before accepting it.** With write validation enabled, a
  write that breaks a shape fails with the rule it broke, so an agent can fix it.
- **Keep the history.** Facts carry when they were recorded and can carry when
  they were true, so you can query earlier recorded or valid-time views.
- **Standard, and scored.** Its SPARQL 1.1 conformance is measured against the
  Working Group–approved W3C tests, class by class with every exception named,
  and scored alongside other stores in
  [the conformance report](docs/book/src/benchmarks/conformance.md).

The long form (sharing between stores, the feature list, the architecture and
a comparison): [Why Quipu](docs/book/src/why-quipu.md).

## Install

Linux (x86_64, arm64) and macOS (Apple Silicon, Intel): download the
checksummed release.

```bash
V=0.12.1
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  T=x86_64-unknown-linux-gnu ;;
  Linux-aarch64) T=aarch64-unknown-linux-gnu ;;
  Darwin-arm64)  T=aarch64-apple-darwin ;;
  Darwin-x86_64) T=x86_64-apple-darwin ;;
  *) echo "no release for this platform; build from source below"; exit 1 ;;
esac
A="quipu-quipu-ai-v$V-$T"
curl -fsSLO "https://github.com/scbrown/quipu/releases/download/quipu-ai-v$V/$A.tar.gz"
curl -fsSLO "https://github.com/scbrown/quipu/releases/download/quipu-ai-v$V/$A.tar.gz.sha256"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum -c "$A.tar.gz.sha256"
else
  shasum -a 256 -c "$A.tar.gz.sha256"
fi
tar -xzf "$A.tar.gz"
mkdir -p ~/.local/bin
install -m 755 "$A/quipu" "$A/quipu-server" ~/.local/bin/
```

Keep `quipu` and `quipu-server` side by side: `quipu mcp` runs the companion
`quipu-server --mcp-stdio` from the same directory.

Anywhere else, build from source (needs a Rust toolchain):

```bash
cargo install quipu-ai --locked --features full
```

The crate is `quipu-ai`; it installs `quipu` and `quipu-server`. The test
utilities (`quipu-shacl-conformance`, `seed-fixtures`) build only with
`--features dev-tools`.
Check it:

```bash
quipu --version
```

```text
quipu 0.12.1
```

If that prints an older version, another copy is earlier on your `PATH`:
`which -a quipu`.

## First success in three commands

Write a rule (every `Person` needs a name), two people who follow it, and one
who does not:

```bash
mkdir quipu-demo && cd quipu-demo
printf '@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\nex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;\n  sh:property [ sh:path ex:name ; sh:minCount 1 ] .\n' > shapes.ttl
printf '@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name "Alice" ; ex:knows ex:bob .\nex:bob   a ex:Person ; ex:name "Bob" .\n' > people.ttl
printf '@prefix ex: <http://example.org/> .\nex:carol a ex:Person .\n' > carol.ttl
```

Then load the good facts, ask a question, and try the bad one:

```bash
quipu knot people.ttl --shapes shapes.ttl --db demo.db
quipu read 'SELECT ?who ?name WHERE { ?who <http://example.org/name> ?name }' --db demo.db
quipu knot carol.ttl --shapes shapes.ttl --db demo.db
```

<!-- markdownlint-disable MD010 -->
<!-- quipu's real output is tab-separated; the block below is verbatim. -->

```text
SHACL validation passed
knotted 5 facts from people.ttl (tx 1)
who	name
----------------------------------------
http://example.org/alice	"Alice"
http://example.org/bob	"Bob"

2 results
SHACL validation failed: 1 violation(s)
  Violation on http://example.org/carol: MinCount(1) not satisfied
```

<!-- markdownlint-enable MD010 -->

The last command exits 1 and writes nothing: Carol breaks the rule, so she never
enters the graph. That refusal at write time, with the reason attached, is what
Quipu is for.

## On your own data

| you want to | run |
|---|---|
| load Turtle, checked against your shapes | `quipu knot <file.ttl> --shapes <shapes.ttl>` |
| bulk-load a lot of data at once, validated before it touches your graph | `quipu share --output <dir>` from a scratch store, then `quipu import <dir>` (staged and shape-checked; failures are quarantined), then `quipu import promote <share-id>` (one transaction into ROOT) |
| migrate or move a dataset between stores | the same three steps: `share`, `import`, `import promote` |
| ask a SPARQL question | `quipu read '<sparql>'` |
| see the graph as it was on a date | `quipu read '<sparql>' --valid-at 2026-09-01` |
| see what depends on an entity | `quipu impact <entity-IRI>` |
| keep shapes in the store, so every write is checked | `quipu shapes load <name> <shapes.ttl>` |
| explore it in a browser, or over HTTP | `quipu-server --db <file> --bind 127.0.0.1:3030` |

For a bulk load, a migration or a backfill, use **import + promote**, not
thousands of `knot` or SPARQL Update writes. `import` validates the whole dataset
in its own staging graph and never touches ROOT. `promote` then admits it in a
single transaction, and one fork rollback undoes it. Walkthrough:
[Bulk loads and migrations](docs/book/src/recipes/bulk-loads.md).

Every command and flag: [CLI reference](docs/book/src/reference/cli.md). The HTTP
endpoints: [REST API](docs/book/src/reference/rest-api.md).

## Wire it into your agent

Agents can connect directly: `quipu-server` serves streamable HTTP at `/mcp`, and
`quipu mcp --db store.db` provides stdio using the companion server binary.
Build both with `cargo build --release --features full`. Protected stdio writes use
`--mcp-token-file /path/to/private-token`; HTTP writes use the existing bearer policy.
Quipu defines 77 MCP tools (79 with `owl`), from one shared schema manifest.
Bobbin's `knowledge_*` tools and existing REST-backed proxies remain compatible.
See the [connection and authentication guide](docs/book/src/reference/mcp-tools.md#connect-directly).

```bash
claude mcp add quipu -- /absolute/path/to/quipu mcp --db /absolute/path/to/store.db
```

No binaries? Quipu is in the [MCP Registry](https://registry.modelcontextprotocol.io),
so VS Code's `@mcp` Extensions search (and other registry-aware clients) can
install it in one click. That runs the published image, which needs Docker and
nothing else. The registry entry pins its own image version; it can lag the
latest binary release. Check that tag when you need a particular version:

```bash
docker run -i --rm -v quipu-data:/data ghcr.io/scbrown/quipu:latest
```

The graph lives at `/data/quipu.db` in the `quipu-data` volume, so it survives
restarts and starts empty. Change the volume name in your client's MCP config to
keep one graph per project.

- MCP Registry name: `mcp-name: io.github.scbrown/quipu`

Setup and every tool: [bobbin's Quipu integration guide](https://github.com/scbrown/bobbin/blob/main/docs/book/src/guides/quipu-integration.md)
and [Quipu's MCP tools reference](docs/book/src/reference/mcp-tools.md). Any other
client can use the [REST API](docs/book/src/reference/rest-api.md) directly.

## Before you start

**Platforms.** Releases ship Linux (x86_64, arm64) and macOS (Apple Silicon,
Intel) archives. Anywhere else, build from source; it needs a Rust toolchain
and nothing else.

**What each build includes.** `quipu` needs the `shacl` feature and
`quipu-server` needs `shacl`, `onnx` and `server`. `--features full`, as in the
source install above, turns on all of them. The feature list:
[Installation](docs/book/src/getting-started/installation.md).

## What's next

- [The Quipu book](https://scbrown.github.io/quipu/): concepts, tutorials and reference, in reading order
- [Docs map](docs/book/src/docs-map.md): every design note and document in this repository, routed
- [Quick Start](docs/book/src/getting-started/quick-start.md): the library, the REST server and the reasoner, step by step

## 🧺 The stack

Caboodle installs these together and proves each one works; every tool also stands alone.

<!-- stack-table:start -->

The Quipu stack is built around a governed, bitemporal knowledge store: shared
facts, decisions and work history your agents can query. Quipu can check writes
against your rules, records where each fact came from, and answers what was true
when.

| tool | what it gives your agents |
|---|---|
| [caboodle](https://github.com/scbrown/caboodle) | 🧰 Agent Platform Toolkit - installs and configures the quipu stack |
| [quipu](https://github.com/scbrown/quipu) | stores shared facts with provenance and history; can check writes against your rules |
| [camayoc](https://github.com/scbrown/camayoc) | prepares knowledge and vocabulary for Quipu, with checks before it is admitted |
| [bobbin](https://github.com/scbrown/bobbin) | searches your code and can include related knowledge from Quipu |
| [yupana](https://github.com/scbrown/yupana) | shows code impact and can read policies from Quipu to check changes |
| [desire-path](https://github.com/scbrown/desire-path) | turns agent mistakes into feature requests; works on its own without Quipu |
| [seeds](https://github.com/scbrown/seeds) | a work item tracker backed by the Quipu knowledge base |
| [shantytown](https://github.com/scbrown/shantytown) | runs crews of AI agents on work items, triggered by hand, on a schedule or by events; can share work and memory through Quipu |
| [shuttle](https://github.com/scbrown/shuttle) | runs signed agent workflows and keeps their records in Quipu |

<!-- stack-table:end -->

## Contributing

```bash
just build
just test
just check   # every quality check (the pre-push gate)
```

See [RELEASING.md](docs/RELEASING.md) for how a release is cut.

## 📜 License

Licensed under the Apache License, Version 2.0 (see [LICENSE](LICENSE)). Releases up to and including quipu-ai v0.11.0 were MIT-licensed and remain available under MIT.
