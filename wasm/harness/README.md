# quipu wasm harness

Browser acceptance harness for quipu on `wasm32-unknown-unknown`
(quipu-qd2). Opens a real `quipu::Store` in a dedicated Web Worker, ingests
synthetic episodes (the `scale_bench` shape), runs the three representative
reads, and — for OPFS — proves the data survives a page reload and a full
browser relaunch.

Design context: `docs/design/wasm-support.md` §8 Phase 3, §9 (how this runs
headless, including in the Claude Code remote container).

## Layout

- `src/lib.rs` — wasm-bindgen exports: `install_opfs`, `scenario_write`,
  `scenario_read`. The wasm side reports counts; assertions live in the
  driver.
- `www/` — static page + module worker that host the wasm.
- `run.mjs` — Playwright driver: serves `www/`, runs the memory-VFS scenario
  and the OPFS reload/relaunch scenario, exits non-zero on any failed check.

## Running

```bash
# 1. Build the wasm (target and getrandom cfg come from .cargo/config.toml)
cargo build --release

# 2. Generate the JS glue (CLI version must match the wasm-bindgen in
#    Cargo.lock — `cargo install wasm-bindgen-cli --version <that version>`)
wasm-bindgen --target web --out-dir www/pkg \
  target/wasm32-unknown-unknown/release/quipu_wasm_harness.wasm

# 3. Drive it (needs `playwright` resolvable here and a Playwright-managed
#    Chromium; `npm i playwright` or symlink a global install into
#    node_modules/)
node run.mjs            # headless
node run.mjs --headed   # watch it
```

Or from the repo root: `just wasm build` / `just wasm test`.

OPFS state lives in `.profile/` (the Chromium user profile). Delete it for a
clean slate; the relaunch checks depend on it persisting between runs of the
same invocation only.

## Benchmarking (quipu-ajz)

`bench.mjs` is the wasm half of the §5.5 wasm-vs-native comparison — memory
VFS and OPFS, read model off and on, fresh page per configuration:

```bash
node bench.mjs 5000            # or: just wasm bench 5000
cargo run --release --no-default-features \
  --example wasm_native_baseline -- 5000   # the native half, from repo root
```

Both halves are methodology-identical by construction (same episode shape,
same cold-then-warm query loops) — change one, change the other.
