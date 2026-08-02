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

# Build + deploy quipu-server with the fail-loud feature gate. Use THIS, never a
# bare `cargo build`, to ship the server: quipu-server has required-features and
# a plain build silently skips it, shipping a stale binary. Override targets via
# env (INSTALL_TARGETS, SERVICE, HEALTH_URL, BUILD_DIR); NO_DEPLOY=1 to gate only.
deploy-server:
    bash scripts/build-deploy-server.sh

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

# Load the fictional demo graph and serve the explorer on localhost:3030.
# This is the dataset behind the README screenshot — see examples/demo-graph.
demo:
    rm -f /tmp/quipu-demo.db
    cargo run --bin quipu --features shacl -- knot examples/demo-graph/demo.ttl --db /tmp/quipu-demo.db
    cargo run --bin quipu-server --features full -- --db /tmp/quipu-demo.db

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

# === Release ===

# Verify the newest CHANGELOG.md section matches git-cliff EXACTLY — no commit
# missing, and none that belongs to another release. Catches both shapes of
# release-plz's commit mis-selection. Run before merging any release-plz PR.
changelog-verify *args:
    ./scripts/verify-changelog.sh {{args}}

# Prove changelog-verify can still fail in BOTH directions. It shipped able to
# fail in only one, and passed a section holding 221 commits against 1 expected.
changelog-verify-test:
    ./scripts/test-verify-changelog.sh
