[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

# Quipu

AI-native knowledge graph with strict ontology enforcement — structured
knowledge encoded in knotted strings.

A [quipu](https://en.wikipedia.org/wiki/Quipu) is the Incan knotted-string
recording system — a pre-Columbian knowledge graph encoded in textile.
Cords are entities, knots are facts, colors are types, and trained readers
(khipukamayuq) interpret the structure. Quipu brings this philosophy to
modern knowledge graphs: strict structure, enforced by AI agents.

## What Is This?

An embeddable Rust library for building knowledge graphs with:

- **Strict OWL/SHACL ontology** enforced at write time
- **Immutable bitemporal fact log** — time-travel, contradiction detection, full audit trail
- **Native hybrid search** — SPARQL 1.1 + vector similarity (LanceDB)
- **Agent-friendly validation** — structured feedback, not just rejections
- **Incremental reasoning** — only re-derive what changed
- **"SQLite energy"** — single process, no server, inspect with `sqlite3`

Designed as a module for [Bobbin](https://github.com/scbrown/bobbin)
(semantic code search engine). Bobbin holds the thread; Quipu ties knots
of structured meaning into it.

## Status

**Vision phase.** See [docs/design/vision.md](docs/design/vision.md) for
the full design document covering architecture, technology decisions,
competitor analysis, and integration plans.

## Quick Start

TODO: Not yet implemented. See the vision doc for where we're headed.

## Development

This project uses [just](https://github.com/casey/just) as a command runner.

```bash
just --list          # Show available commands
just setup           # Install pre-commit hooks
just check           # Run all quality checks
```

## Documentation

```bash
just docs build      # Build the book
just docs serve      # Serve locally with hot reload
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

[MIT](LICENSE)
