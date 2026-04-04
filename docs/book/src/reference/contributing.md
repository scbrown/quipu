# Contributing

See [CONTRIBUTING.md](https://github.com/scbrown/quipu/blob/main/CONTRIBUTING.md)
in the repository root for development setup and guidelines.

## Quick Start

```bash
git clone https://github.com/scbrown/quipu
cd quipu
just setup    # Install pre-commit hooks
just check    # Run all quality checks
just test     # Run tests
just lint     # Run clippy
```

## Architecture

Quipu is organized as a single Rust crate with four core modules:

- `store` -- SQLite-backed EAVT fact log
- `rdf` -- RDF data model bridge (oxrdf)
- `sparql` -- SPARQL query evaluator (spargebra)
- `types` -- shared data structures
