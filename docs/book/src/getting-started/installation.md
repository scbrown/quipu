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
cargo build --release --features shacl,onnx
```

This produces two binaries:

- `target/release/quipu` -- CLI tool
- `target/release/quipu-server` -- REST API server

The `onnx` feature is **required for `quipu-server`** — it pulls in the embedding
runtime the server uses to auto-embed queries. It is not a default feature, so a
plain `cargo build --release` produces only the `quipu` CLI and **silently omits
the server** (cargo skips a bin whose `required-features` are off). If you only
want the CLI, `cargo build --release` is enough. Verify the server built:

```bash
ls target/release/quipu-server
```

## Requirements

- Rust 1.85+ (edition 2024)
- SQLite is bundled via rusqlite -- no system dependency needed
