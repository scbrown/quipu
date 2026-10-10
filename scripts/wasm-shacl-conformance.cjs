#!/usr/bin/env node
// Adapter for benchmark/public/shacl_core.py using the actual Node WASM bundle.
const { readFileSync } = require("node:fs");
const { resolve } = require("node:path");

try {
  if (!process.env.QUIPU_WASM_BUNDLE) {
    throw new Error("QUIPU_WASM_BUNDLE must name the generated Node bundle");
  }
  const args = process.argv.slice(2);
  if (args.length !== 4 || args[0] !== "--shapes" || args[2] !== "--data") {
    throw new Error("usage: wasm-shacl-conformance.cjs --shapes <ttl> --data <ttl>");
  }
  const wasm = require(resolve(process.env.QUIPU_WASM_BUNDLE));
  console.log(wasm.validateShapes(readFileSync(args[1], "utf8"), readFileSync(args[3], "utf8")));
} catch (error) {
  console.error(String(error.message ?? error));
  process.exit(1);
}
