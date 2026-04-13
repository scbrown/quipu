#!/usr/bin/env bash
set -euo pipefail

if ! command -v mdbook &>/dev/null; then
    echo "ERROR: mdbook not found on PATH." >&2
    echo "Install with: cargo install mdbook mdbook-mermaid" >&2
    exit 1
fi

if ! command -v mdbook-mermaid &>/dev/null; then
    echo "ERROR: mdbook-mermaid not found on PATH." >&2
    echo "The book uses Mermaid diagrams and will fail without the preprocessor." >&2
    echo "Install with: cargo install mdbook-mermaid" >&2
    exit 1
fi

exec mdbook build docs/book
