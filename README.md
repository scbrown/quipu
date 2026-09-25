<p align="center">
  <img src="assets/logo.svg" width="200" alt="Quipu logo — knotted strings forming a knowledge graph"/>
</p>

<h1 align="center">quipu</h1>

<p align="center">
  <em>🪢 A memory for your agents that refuses facts that break your rules</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"/></a>
  <a href="https://github.com/scbrown/quipu/actions/workflows/ci.yml"><img src="https://github.com/scbrown/quipu/actions/workflows/ci.yml/badge.svg" alt="CI"/></a>
  <a href="https://github.com/scbrown/caboodle"><img src="https://img.shields.io/badge/stack-quipu-8B5E3C.svg" alt="Part of the caboodle stack"/></a>
  <a href="https://doi.org/10.5281/zenodo.21878428"><img src="https://zenodo.org/badge/1201016929.svg" alt="DOI"/></a>
</p>

**Quipu is a knowledge graph for AI coding agents and the people who run them.
It stores what your agents learn as facts, checks every write against rules you
declare (SHACL shapes), and refuses the facts that break them, so bad knowledge
never gets in.** It keeps every version of every fact, so you can ask what was
true last Tuesday, and it answers standard SPARQL 1.1. It comes as a command-line
tool, a REST server, and a Rust library, and agents reach it through
[bobbin](https://github.com/scbrown/bobbin)'s MCP server.

A [quipu](https://en.wikipedia.org/wiki/Quipu) is the Andean knotted-cord record:
cords are entities, knots are facts.

## Why you would want it

- **Bad facts are refused, not cleaned up later.** A write that breaks a shape
  fails with the rule it broke, so an agent can correct it on the spot.
- **Nothing is overwritten.** Every fact carries when it was recorded and when
  it was true, so you can query the graph as it was at any moment.
- **Standard, and measured.** It passes all Working Group–approved W3C SPARQL
  1.1 Query, Update, Protocol and Results tests, scored alongside other stores
  in [the conformance report](docs/book/src/benchmarks/conformance.md).

The long form (sharing between stores, the feature list, the architecture and
a comparison): [Why Quipu](docs/book/src/why-quipu.md).

## Install

Linux x86_64: download the checksummed release.

```bash
V=0.8.1
curl -fsSLO "https://github.com/scbrown/quipu/releases/download/quipu-ai-v$V/quipu-quipu-ai-v$V-x86_64-unknown-linux-gnu.tar.gz"
curl -fsSLO "https://github.com/scbrown/quipu/releases/download/quipu-ai-v$V/quipu-quipu-ai-v$V-x86_64-unknown-linux-gnu.tar.gz.sha256"
sha256sum -c "quipu-quipu-ai-v$V-x86_64-unknown-linux-gnu.tar.gz.sha256"
tar -xzf "quipu-quipu-ai-v$V-x86_64-unknown-linux-gnu.tar.gz"
mkdir -p ~/.local/bin
install -m 755 "quipu-quipu-ai-v$V-x86_64-unknown-linux-gnu/quipu" \
  "quipu-quipu-ai-v$V-x86_64-unknown-linux-gnu/quipu-server" ~/.local/bin/
```

Anywhere else, build from source (needs a Rust toolchain):

```bash
cargo install quipu-ai --locked --features full
```

The crate is `quipu-ai`; it installs `quipu` and `quipu-server`, plus two test
utilities (`quipu-shacl-conformance`, `seed-fixtures`) you can ignore.
Check it:

```bash
quipu --version
```

```text
quipu 0.8.1
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
| ask a SPARQL question | `quipu read '<sparql>'` |
| see the graph as it was on a date | `quipu read '<sparql>' --valid-at 2026-09-01` |
| see what depends on an entity | `quipu impact <entity-IRI>` |
| keep shapes in the store, so every write is checked | `quipu shapes load <name> <shapes.ttl>` |
| explore it in a browser, or over HTTP | `quipu-server --db <file> --bind 127.0.0.1:3030` |

Every command and flag: [CLI reference](docs/book/src/reference/cli.md). The HTTP
endpoints: [REST API](docs/book/src/reference/rest-api.md).

## Wire it into your agent

Agents can connect directly: `quipu-server` serves streamable HTTP at `/mcp`, and
`quipu mcp --db store.db` provides stdio using the companion server binary.
Build both with `cargo build --release --features full`. Protected stdio writes use
`--mcp-token-file /path/to/private-token`; HTTP writes use the existing bearer policy.
Quipu defines 46 MCP tools (48 with `owl`), from one shared schema manifest.
Bobbin's `knowledge_*` tools and existing REST-backed proxies remain compatible.
See the [connection and authentication guide](docs/book/src/reference/mcp-tools.md#connect-directly).

```bash
claude mcp add quipu -- /absolute/path/to/quipu mcp --db /absolute/path/to/store.db
```

No binaries? Quipu is in the [MCP Registry](https://registry.modelcontextprotocol.io),
so VS Code's `@mcp` Extensions search (and other registry-aware clients) can
install it in one click. That runs the published image, which needs Docker and
nothing else:

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

**Platforms.** The release is built for Linux x86_64. On macOS and anywhere
else, build from source; it needs a Rust toolchain and nothing else.

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

| tool | what it gives your agents |
|---|---|
| [caboodle](https://github.com/scbrown/caboodle) | one wizard that installs the stack and proves it works |
| [quipu](https://github.com/scbrown/quipu) **(you are here)** | a knowledge graph that refuses facts that break its rules |
| [camayoc](https://github.com/scbrown/camayoc) | the starter vocabulary, and how new knowledge earns its way in |
| [bobbin](https://github.com/scbrown/bobbin) | search and context over your repositories, served over MCP |
| [yupana](https://github.com/scbrown/yupana) | which code calls which: the blast radius before an edit |
| [desire-path](https://github.com/scbrown/desire-path) | the tool calls your agents get wrong, so you can fix them |

## Contributing

```bash
just build
just test
just check   # every quality check (the pre-push gate)
```

See [RELEASING.md](docs/RELEASING.md) for how a release is cut.

## 📜 License

[MIT](LICENSE)
