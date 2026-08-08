#!/bin/bash
# SessionStart hook for Claude Code on the web (remote containers).
#
# Installs the two tool dependencies the quality gate needs but the
# container image lacks, and pre-builds the quipu CLI so agents can run
# it directly. The container state is cached after the hook completes,
# so the expensive cargo installs pay only on the first session.
set -euo pipefail

# Local sessions already have a configured machine; do nothing there.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# mdbook + mdbook-mermaid: `just check` runs the mdbook-build pre-commit
# hook, which hard-fails when the binaries are missing.
if ! command -v mdbook >/dev/null 2>&1; then
  cargo install mdbook --locked
fi
if ! command -v mdbook-mermaid >/dev/null 2>&1; then
  cargo install mdbook-mermaid --locked
fi

# Build the quipu CLI (shacl matches the gate's middle clippy pass and
# what `just demo` uses) and put it on the session PATH so agents can
# run `quipu` without cargo-run ceremony.
cargo build --quiet --features shacl --bin quipu
echo "export PATH=\"$CLAUDE_PROJECT_DIR/target/debug:\$PATH\"" >> "$CLAUDE_ENV_FILE"
