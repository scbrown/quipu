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
cargo build --release --features full
```

This produces two binaries:

- `target/release/quipu` -- CLI tool
- `target/release/quipu-server` -- REST API server

`quipu-server` requires **both** `onnx` (the embedding runtime it uses to
auto-embed queries) and `server` (axum, tower-http and tokio — the HTTP stack is
feature-gated so the library does not carry a web server; `server` also implies
`remote`, the federation client). The `full` bundle is what releases are built
with and enables both. Neither is a default feature, so a
plain `cargo build --release` produces only the `quipu` CLI and **silently omits
the server** (cargo skips a bin whose `required-features` are off). If you only
want the CLI, `cargo build --release` is enough. Verify the server built:

```bash
ls target/release/quipu-server
```

## The Full Stack (caboodle)

To install Quipu as part of the whole knowledge stack —
[caboodle](https://github.com/scbrown/caboodle) interviews, plans, applies,
and verifies the installation of every stack tool — use the wrapper script:

```bash
# Phase one: install caboodle if absent, write a reviewable plan, STOP.
scripts/install-stack.sh --profile kg

# Phase two: after reviewing caboodle-plan.toml, apply + verify,
# then verify and load knowledge packs into the target store.
scripts/install-stack.sh --profile kg --yes \
  --qpack domain.qpack.db --db my.db
```

The two-phase gate is deliberate and mirrors caboodle's own doctrine: nothing
installs until the written plan has been reviewed (or `--yes` given
explicitly). Every `--qpack` is checked with `quipu pack --verify` before
`quipu unpack` — a content-hash mismatch refuses the pack rather than
installing silently corrupted knowledge. `--dry-run` prints every command the
script would run and executes nothing; `--profile` selects the caboodle
profile (default `kg`).

## Python Client

The REST API has a thin, zero-dependency Python client under `python/`:

```bash
pip install ./python
```

See the [Python Client reference](../reference/python-client.md).

## Requirements

- Rust 1.85+ (edition 2024)
- SQLite is bundled via rusqlite -- no system dependency needed
- Python >= 3.11 for the optional Python client (stdlib only)
