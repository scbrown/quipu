# quipu
# Run `just --list` to see available recipes

# Quiet by default to save context; use verbose=true for full output
verbose := "false"

# Default recipe - show available commands
default:
    @just --list

# === Setup ===

# Install pre-commit hooks and verify dependencies
setup:
    pre-commit install
    @echo "Setup complete."

# === Quality ===

# Run all quality checks (pre-push gate)
check:
    pre-commit run --all-files



# === Rust ===

# Build the project
build:
    cargo build

# Run tests
test *args="":
    cargo test {{args}}

# Run linter
lint:
    cargo clippy -- -D warnings -A missing-docs

# Format code
fmt:
    cargo fmt


# === Fixtures ===

# Generate test-fixtures/test-store.db from static assets
seed:
    cargo run --bin seed-fixtures --features shacl

# Serve the test fixture database on localhost:3030
serve-fixtures:
    cargo run --bin quipu-server --features shacl,onnx -- --db test-fixtures/test-store.db

# === Documentation ===

# Documentation management: just docs <cmd>
# Commands: build, serve, lint, fix, fmt, vale, check

docs cmd="build":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{cmd}}" in
        build)    mdbook build docs/book ;;
        serve)    mdbook serve docs/book --open ;;
        lint)     npx markdownlint-cli2 "docs/book/src/**/*.md" "README.md" "CONTRIBUTING.md" ;;
        fix)      npx markdownlint-cli2 --fix "docs/book/src/**/*.md" "README.md" "CONTRIBUTING.md" ;;
        fmt)      npx prettier --write "docs/book/src/**/*.md" --prose-wrap preserve ;;
        vale)     vale docs/book/src/ ;;
        check)    just docs lint && just docs build ;;
        *)        echo "Unknown: {{cmd}}. Try: build serve lint fix fmt vale check" ;;
    esac
