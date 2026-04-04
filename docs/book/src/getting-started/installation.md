# Installation

## As a Rust Dependency

Add to your `Cargo.toml`:

```toml
[dependencies]
quipu = { git = "https://github.com/scbrown/quipu" }
```

## From Source

```bash
git clone https://github.com/scbrown/quipu
cd quipu
cargo build
```

## Requirements

- Rust 1.85+ (edition 2024)
- SQLite is bundled via rusqlite -- no system dependency needed
