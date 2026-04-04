# Installation

## As a Rust Dependency

Add to your `Cargo.toml`:

```toml
[dependencies]
quipu = { git = "https://github.com/scbrown/quipu" }
```

To use SHACL validation (enabled by default):

```toml
[dependencies]
quipu = { git = "https://github.com/scbrown/quipu", features = ["shacl"] }
```

To exclude SHACL (smaller binary, faster compile):

```toml
[dependencies]
quipu = { git = "https://github.com/scbrown/quipu", default-features = false }
```

## From Source

```bash
git clone https://github.com/scbrown/quipu
cd quipu
cargo build --release
```

This produces two binaries:

- `target/release/quipu` -- CLI tool
- `target/release/quipu-server` -- REST API server

## Requirements

- Rust 1.85+ (edition 2024)
- SQLite is bundled via rusqlite -- no system dependency needed
