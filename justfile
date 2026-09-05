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


# === Wasm ===

# Browser harness (quipu-qd2/ajz/2l5): just wasm <cmd>
# Commands: check (compile the lib for wasm32), build (harness + JS glue),
# test (Playwright scenarios incl. OPFS reload persistence), bench (the
# wasm half of the §5.5 throughput comparison; pair with the
# wasm_native_baseline example at the same episode count), roundtrip
# (.db interchange: browser export → quipu CLI/sqlite3, browser pack →
# native attach, native .db → browser import).
# Prereqs for build/test/bench: wasm-bindgen-cli matching
# wasm/harness/Cargo.lock, and a `playwright` resolvable from wasm/harness
# (see wasm/harness/README.md).
wasm cmd="check" n="1000":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{cmd}}" in
        check) RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
               cargo check --target wasm32-unknown-unknown --no-default-features --lib ;;
        build) cd wasm/harness && cargo build --release && \
               wasm-bindgen --target web --out-dir www/pkg \
                   target/wasm32-unknown-unknown/release/quipu_wasm_harness.wasm ;;
        test)  just wasm build && cd wasm/harness && node run.mjs ;;
        bench) just wasm build && cd wasm/harness && node bench.mjs {{n}} ;;
        roundtrip) just wasm build && cd wasm/harness && node roundtrip.mjs ;;
        *)     echo "unknown wasm command: {{cmd}} (check|build|test|bench|roundtrip)"; exit 1 ;;
    esac

# === Fixtures ===

# Generate test-fixtures/test-store.db from static assets
seed:
    cargo run --bin seed-fixtures --features shacl

# Serve the test fixture database on localhost:3030
serve-fixtures:
    cargo run --bin quipu-server --features shacl,onnx,server -- --db test-fixtures/test-store.db

# Build the paper PDF (docs/paper/): just paper [clean]
paper cmd="build":
    @if [ "{{ cmd }}" = "build" ]; then \
        cd docs/paper && \
        if command -v tectonic >/dev/null 2>&1; then tectonic main.tex; \
        elif command -v latexmk >/dev/null 2>&1; then latexmk -pdf -interaction=nonstopmode main.tex; \
        elif command -v pdflatex >/dev/null 2>&1; then pdflatex -interaction=nonstopmode main.tex && bibtex main && pdflatex -interaction=nonstopmode main.tex && pdflatex -interaction=nonstopmode main.tex; \
        else echo "no TeX engine found (install tectonic, latexmk, or pdflatex)"; exit 1; fi; \
    elif [ "{{ cmd }}" = "clean" ]; then \
        cd docs/paper && rm -f main.aux main.bbl main.blg main.log main.out main.pdf main.fls main.fdb_latexmk; \
    else echo "unknown paper command '{{ cmd }}' (available: build, clean)"; exit 1; fi

# Build the merge paper PDF (docs/paper-merge/): just paper-merge [clean]
paper-merge cmd="build":
    @if [ "{{ cmd }}" = "build" ]; then \
        cd docs/paper-merge && \
        if command -v tectonic >/dev/null 2>&1; then tectonic main.tex; \
        elif command -v latexmk >/dev/null 2>&1; then latexmk -pdf -interaction=nonstopmode main.tex; \
        elif command -v pdflatex >/dev/null 2>&1; then pdflatex -interaction=nonstopmode main.tex && bibtex main && pdflatex -interaction=nonstopmode main.tex && pdflatex -interaction=nonstopmode main.tex; \
        else echo "no TeX engine found (install tectonic, latexmk, or pdflatex)"; exit 1; fi; \
    elif [ "{{ cmd }}" = "clean" ]; then \
        cd docs/paper-merge && rm -f main.aux main.bbl main.blg main.log main.out main.pdf main.fls main.fdb_latexmk; \
    else echo "unknown paper-merge command '{{ cmd }}' (available: build, clean)"; exit 1; fi

# Run a paper benchmark (see benchmark/<name>/README.md): just bench census [--arm control] [--seed N]
bench name *args:
    @if [ "{{ name }}" = "census" ]; then \
        cargo run --quiet --release --example census -- {{ args }}; \
    elif [ "{{ name }}" = "merge" ]; then \
        cargo run --quiet --release --example mergebench --features shacl -- {{ args }}; \
    else \
        echo "unknown benchmark '{{ name }}' (available: census, merge)"; exit 1; \
    fi

# Load the fictional demo graph and serve the explorer on localhost:3030.
# This is the dataset behind the README screenshot — see examples/demo-graph.
demo:
    rm -f /tmp/quipu-demo.db
    cargo run --bin quipu --features shacl -- knot examples/demo-graph/demo.ttl --db /tmp/quipu-demo.db
    cargo run --bin quipu-server --features full -- --db /tmp/quipu-demo.db

# Re-run the two-store transcript embedded in the Sharing & Federation chapter.
sharing-demo:
    cargo build --bin quipu
    examples/sharing-demo/run.sh --check

# Load the SMAC datalinks tech tree and serve the 3D Datalinks view.
# The graph lives in NeuralAmplifier (scbrown/NeuralAmplifier), which owns and
# regenerates it — point `datalinks` at a checkout rather than vendoring a copy.
# Then open http://localhost:3030/#datalinks
datalinks graph="../NeuralAmplifier/datalinks/thinker/alphax.ttl":
    rm -f /tmp/quipu-datalinks.db
    cargo run --bin quipu --features shacl -- knot {{graph}} --db /tmp/quipu-datalinks.db
    cargo run --bin quipu-server --features full -- --db /tmp/quipu-datalinks.db

# Ingest the sibling repos as code + doc entities, then serve them.
# Produces CodeModule / CodeSymbol / Document / Section against the
# shapes/code-entities.ttl vocabulary, validated on load.
ingest-repos +repos="../quipu ../hank ../NeuralAmplifier ../thinker":
    ./scripts/ingest-repos.py {{repos}} -o /tmp/quipu-code.ttl
    rm -f /tmp/quipu-code.db
    cargo run --bin quipu --features shacl -- knot /tmp/quipu-code.ttl --db /tmp/quipu-code.db
    cargo run --bin quipu-server --features full -- --db /tmp/quipu-code.db

# === Documentation ===

# Documentation management: just docs <cmd>
# Commands: build, serve, lint, fix, fmt, vale, check

# Stage the 3D Datalinks demo's runtime assets into the book source.
# COPIED, not committed twice: ui/ owns these files and the demo is a second
# deployment of the same module. mdbook copies non-markdown files in src/
# verbatim, so they land at /quipu/datalinks/. Gitignored — regenerate with
# this recipe (`just docs build` runs it for you).
docs-assets:
    mkdir -p docs/book/src/datalinks/vendor
    cp ui/datalinks.js ui/graph-canvas.js docs/book/src/datalinks/
    cp ui/vendor/three.module.min.js docs/book/src/datalinks/vendor/

# Stage the "Explore this repository's graph" page's artifacts (aegis-tpqccc):
# just explorer [release|local]
#
#   release  (default) download the newest release's wasm bundle + repository
#            pack — what the docs workflow does, and the only mode that needs no
#            Rust. Requires `gh`.
#   local    build the wasm from THIS tree and mint a pack from the working
#            copy, for iterating on the page against uncommitted changes.
#            Requires a wasm32 toolchain, a wasm-bindgen-cli matching
#            wasm/explorer/Cargo.lock, and `bobbin` for the pack.
#
# Both write into docs/book/src/explore/, which is gitignored. Serve the built
# book (`just docs build` then a static server over docs/book/book) — the page
# needs a real origin for module workers, so file:// will not do.
explorer mode="release":
    #!/usr/bin/env bash
    set -euo pipefail
    DEST=docs/book/src/explore
    mkdir -p "$DEST/pkg"
    if [ "{{mode}}" = "local" ]; then
        REPO=$PWD
        cd wasm/explorer
        cargo build --release
        TARGET=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
        wasm-bindgen --target web --out-dir "$REPO/$DEST/pkg" \
            "$TARGET/wasm32-unknown-unknown/release/quipu_wasm_explorer.wasm"
        cd "$REPO"
        echo "Built the bundle from this tree. Provide a pack yourself, or run"
        echo "  just explorer release   # to fetch the released one"
    else
        TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
        TAG=$(gh release view --json tagName --jq .tagName)
        echo "Staging from release $TAG"
        gh release download "$TAG" --pattern '*-wasm.tar.gz' --dir "$TMP" \
            && tar -C "$TMP" -xzf "$TMP"/*-wasm.tar.gz \
            && find "$TMP" -name 'quipu_wasm_explorer*' -exec cp {} "$DEST/pkg/" \; \
            || echo "warning: $TAG has no wasm bundle yet"
        gh release download "$TAG" --pattern '*-repository.qpack.tar.gz' --dir "$TMP" \
            && cp "$TMP"/*-repository.qpack.tar.gz "$DEST/repository.qpack.tar.gz" \
            || echo "warning: $TAG has no repository qpack"
    fi
    ls -la "$DEST" "$DEST/pkg"

# Regenerate the demo's baked graph payload. Needs a NeuralAmplifier checkout.
docs-data graph="../NeuralAmplifier/datalinks/thinker/alphax.ttl":
    ./scripts/export-datalinks.sh {{graph}}

docs cmd="build":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{cmd}}" in
        build)    just docs-assets && mdbook build docs/book ;;
        serve)    just docs-assets && mdbook serve docs/book --open ;;
        lint)     npx markdownlint-cli2 "docs/book/src/**/*.md" "README.md" "CONTRIBUTING.md" ;;
        fix)      npx markdownlint-cli2 --fix "docs/book/src/**/*.md" "README.md" "CONTRIBUTING.md" ;;
        fmt)      npx prettier --write "docs/book/src/**/*.md" --prose-wrap preserve ;;
        vale)     vale docs/book/src/ ;;
        check)    just docs lint && just docs build ;;
        *)        echo "Unknown: {{cmd}}. Try: build serve lint fix fmt vale check" ;;
    esac

# === Public benchmarks ===

# Regenerate the public conformance page and shields badge endpoints from the
# committed ledgers under benchmark/public/results/.
conformance-report:
    python3 benchmark/public/conformance_report.py

# Fail if the published page/badges disagree with the ledgers, then run the
# benchmark unit tests. CI runs exactly this: a results page that has drifted
# from its own evidence is worse than no page.
conformance-check:
    python3 benchmark/public/conformance_report.py --check
    python3 -m unittest discover -s benchmark/public -p 'test_*.py'

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

# Repository contributor graph: generate, prove the receiver, or build the release share.
contributor cmd="generate" output="/tmp/contributor-knowledge.ttl":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{cmd}}" in
        test) node --test scripts/build-contributor-knowledge.test.mjs && scripts/check-explorer-published.test.sh ;;
        generate) node scripts/build-contributor-knowledge.mjs . "{{output}}" ;;
        verify) node scripts/verify-contributor-knowledge.mjs "${QUIPU_BIN:?}" "{{output}}" ;;
        pack) scripts/build-repository-share.sh "${QUIPU_BIN:?}" "$(command -v bobbin)" "$PWD" "{{output}}" "$(git rev-parse HEAD)" ;;
        *) echo "unknown contributor command" >&2; exit 2 ;;
    esac
