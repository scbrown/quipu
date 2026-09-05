// Acceptance for the packaged wasm bundle OUTSIDE a browser (aegis-onew9p §4.4).
//
// smoke-wasm-explorer.mjs proves the bundle works in a tab. This proves the SAME
// compiled .wasm answers under node with no browser, no display and no Rust
// toolchain on the consuming side — which is what "an agent clones the repo and
// queries the graph" actually requires. `wasm-bindgen --target nodejs` emits a
// DIFFERENT glue layer over that identical .wasm, so a break here is a real
// break the browser smoke cannot see: CommonJS instead of ESM, filesystem
// loading instead of fetch, and no init() to await.
//
// Same discipline as the browser half: mint a real pack with the NATIVE binary
// in this run, and assert on COUNTS. A script that only checked "nothing threw"
// would pass against a store that imported zero triples, which is the failure
// worth catching.
//
// The fixture is deliberately byte-identical to the browser smoke's, so the two
// runs are comparable — a count that differs between them is a finding.
//
// usage: node scripts/smoke-wasm-node.mjs <packaged-bundle-dir>

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createRequire } from "node:module";

const bundleDir = process.argv[2];
if (!bundleDir) {
  console.error("usage: node scripts/smoke-wasm-node.mjs <bundle-dir>");
  process.exit(2);
}
const repo = resolve(import.meta.dirname, "..");

const failures = [];
const check = (name, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${name}${detail === undefined ? "" : `  — ${detail}`}`);
  if (!ok) failures.push(name);
};

const work = mkdtempSync(join(tmpdir(), "quipu-wasm-node-smoke-"));
process.on("exit", () => rmSync(work, { recursive: true, force: true }));

// ---- 1. Produce a real pack with the NATIVE binary ------------------------

const quipu = process.env.QUIPU_BIN ?? join(repo, "target/release/quipu");
if (!existsSync(quipu)) {
  console.error(
    `native quipu binary not found at ${quipu}\n` +
    "  This script mints a share with the NATIVE binary and loads it back through\n" +
    "  the wasm bundle, so the calling job must build it first:\n" +
    "      cargo build --release --locked --bin quipu\n" +
    "  or point QUIPU_BIN at an existing one.",
  );
  process.exit(2);
}
const db = join(work, "smoke.db");
const shapes = join(work, "shapes.ttl");
const seed = join(work, "seed.ttl");

writeFileSync(shapes, `@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:WidgetShape a sh:NodeShape ; sh:targetClass ex:Widget .
`);
writeFileSync(seed, `@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:alpha a ex:Widget ; rdfs:label "Alpha" ; ex:connects ex:beta .
ex:beta  a ex:Widget ; rdfs:label "Beta" .
`);

const run = (args) => execFileSync(quipu, args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
run(["shapes", "load", "smoke", shapes, "--db", db]);
run(["knot", seed, "--db", db]);
const shareDir = join(work, "share");
run(["share", "--output", shareDir, "--db", db]);

execFileSync("tar", ["--sort=name", "--mtime=UTC 1970-01-01", "--owner=0", "--group=0",
  "--numeric-owner", "-C", shareDir, "-czf", join(work, "smoke.qpack.tar.gz"), "."]);
const producerManifest = JSON.parse(readFileSync(join(shareDir, "manifest.json"), "utf8"));

// ---- 2. Load the nodejs bundle in-process ---------------------------------
//
// The nodejs target is CommonJS and reads its own .wasm off disk, so there is
// no init() and no server. createRequire is how an ESM script reaches it — and
// that asymmetry with the web target is exactly why this file exists.

const require = createRequire(import.meta.url);
const entry = join(resolve(bundleDir), "quipu_wasm_explorer.js");
if (!existsSync(entry)) {
  console.error(`no nodejs bundle at ${entry}\n` +
    "  Expected the output of: wasm-bindgen --target nodejs --out-dir <dir> <wasm>");
  process.exit(2);
}
const mod = require(entry);

check("the bundle exports Explorer", typeof mod.Explorer === "function", typeof mod.Explorer);
check("the bundle exports explorerVersion", typeof mod.explorerVersion === "function");
if (failures.length) {
  console.error("\nthe module did not load; no further check would mean anything");
  process.exit(1);
}
const version = JSON.parse(mod.explorerVersion());
check("it reports a quipu version", Boolean(version.version), JSON.stringify(version));

// ---- 3. The read half: consume what the CLI produced ----------------------

const bytes = readFileSync(join(work, "smoke.qpack.tar.gz"));
const ex = mod.Explorer.loadQpack(new Uint8Array(bytes), "node-smoke", new Date().toISOString());
const report = JSON.parse(ex.loadReport());

check("the pack it loaded is the pack the CLI wrote",
  report.manifest.share_id === producerManifest.share_id, report.manifest.share_id);
check("the graph hash survives the round trip",
  report.manifest.graph_hash === producerManifest.graph_hash);
check("the import staged", report.import.outcome === "staged", report.import.outcome);
check("nothing was quarantined", report.import.triples.quarantined === 0,
  `accepted ${report.import.triples.accepted}`);
check("triples were actually accepted", report.import.triples.accepted > 0,
  `accepted ${report.import.triples.accepted}`);
check("nothing was off-vocabulary", report.import.validation.off_vocabulary.length === 0,
  JSON.stringify(report.import.validation.off_vocabulary));
check("it promoted", report.promotion?.outcome === "promoted",
  JSON.stringify(report.promotion ?? null));

const widgets = JSON.parse(ex.query(
  "SELECT ?s ?label WHERE { ?s a <http://example.org/Widget> ; "
  + "<http://www.w3.org/2000/01/rdf-schema#label> ?label }"));
const labels = (widgets.rows ?? []).map((r) => r.label).sort();
check("both seeded widgets are queryable",
  labels.length === 2 && labels[0] === "Alpha" && labels[1] === "Beta",
  JSON.stringify(labels));

const join2 = JSON.parse(ex.query(
  "SELECT ?to WHERE { <http://example.org/alpha> <http://example.org/connects> ?o . "
  + "?o <http://www.w3.org/2000/01/rdf-schema#label> ?to }"));
check("the edge between them traverses",
  (join2.rows ?? []).length === 1 && join2.rows[0].to === "Beta",
  JSON.stringify(join2.rows ?? []));

// ---- 4. The write half ----------------------------------------------------

const setResult = JSON.parse(ex.set(
  "http://example.org/alpha",
  "http://www.w3.org/2000/01/rdf-schema#comment",
  JSON.stringify({ str: "edited under node" })));
check("a write is accepted and gets a transaction",
  setResult.tx_id > 0 && setResult.asserted >= 1, JSON.stringify(setResult));

const readBack = JSON.parse(ex.query(
  "SELECT ?c WHERE { <http://example.org/alpha> "
  + "<http://www.w3.org/2000/01/rdf-schema#comment> ?c }"));
check("the write reads back exactly once",
  (readBack.rows ?? []).length === 1 && readBack.rows[0].c === "edited under node",
  JSON.stringify(readBack.rows ?? []));

// /set REPLACES. A second one must leave one value, not two — the same check
// the browser half runs, because single-valued-by-definition is a claim about
// the store and must hold on every consumer of it.
const setAgain = JSON.parse(ex.set(
  "http://example.org/alpha",
  "http://www.w3.org/2000/01/rdf-schema#comment",
  JSON.stringify({ str: "edited under node, again" })));
const afterSecond = JSON.parse(ex.query(
  "SELECT (COUNT(?c) AS ?n) WHERE { <http://example.org/alpha> "
  + "<http://www.w3.org/2000/01/rdf-schema#comment> ?c }"));
check("a second /set replaces rather than appends",
  Number(afterSecond.rows?.[0]?.n) === 1 && setAgain.retracted >= 1,
  `count=${afterSecond.rows?.[0]?.n} retracted=${setAgain.retracted}`);

// ---- 5. The delta: the reason a headless producer is worth having ---------

const delta = JSON.parse(ex.delta());
check("the delta is non-empty after edits", delta.empty === false,
  `update_bytes=${delta.update_bytes}`);
check("the delta names the pack it came from",
  delta.manifest.parent_share === producerManifest.share_id,
  `${delta.manifest.parent_share} vs ${producerManifest.share_id}`);
check("the delta carries the whole artifact, not delta.ru alone",
  delta.files.length === 4, delta.files.map((f) => f.name).join(", "));
check("its paths sit under the pack dir the manifest declared",
  delta.files.every((f) => f.path.startsWith(`${delta.pack_dir}/deltas/`)),
  `${delta.pack_dir} — ${delta.files[0]?.path}`);

// ---- 6. Hand it back to the native binary --------------------------------
//
// The strongest available check, and the one the browser half also ends on: the
// CLI is the receiver these deltas are FOR, so a delta this bundle produces that
// the CLI cannot materialize is the failure that matters. `import delta` reads
// the parent share directory and the delta directory, verifies the manifest and
// the update against delta_hash, and imports into a fresh store.

const deltaDir = join(work, "delta");
mkdirSync(deltaDir, { recursive: true });
for (const f of delta.files) writeFileSync(join(deltaDir, f.name), f.contents);

let importOut = "";
let importOk = true;
try {
  importOut = run(["import", "delta", shareDir, deltaDir]);
} catch (e) {
  importOk = false;
  importOut = String(e.stdout ?? "") + String(e.stderr ?? e.message ?? "");
}
check("the native CLI materializes the node-produced delta", importOk,
  importOut.trim().split("\n").slice(-2).join(" "));
if (importOk) {
  const imported = JSON.parse(importOut);
  const total = imported.triples.accepted + imported.triples.quarantined;
  // THE INVARIANT IS THE TRIPLE COUNT, NOT THE VERDICT. 5 from the parent plus
  // the one edit made above: if the CLI reconstructs six, the delta genuinely
  // carried the edit and its lineage resolved against the parent share.
  check("it reconstructs the edited graph — parent's 5 plus this run's edit",
    total === 6, JSON.stringify(imported.triples));
  check("the manifest and delta_hash verified", Boolean(imported.share_id));

  // Whether those six are ACCEPTED or QUARANTINED is not a property of this
  // bundle, and asserting either would make this smoke a test of someone else's
  // open defect. `quipu import delta` opens Store::open_in_memory() and never
  // adopts the shapes the delta carries, so validation reports "no local shapes
  // loaded" and every custom class lands in off_vocabulary — which blocks
  // promotion for any share that declares one.
  //
  // The bundle does NOT have this gap, and says so in its own source:
  // Explorer::load_qpack calls load_shapes with the comment "skipping it is not
  // a silent no-op: import_share would find the pack's classes ungoverned and
  // quarantine every triple". So the SAME artifact validates clean here and
  // quarantines in the CLI. Reported upstream rather than encoded here.
  const blocked = (imported.promotion?.blockers ?? []).includes("off_vocabulary");
  console.log(`note  CLI promotion eligible=${imported.promotion?.eligible}`
    + `${blocked ? " (off_vocabulary — `import delta` loads no shapes; upstream gap)" : ""}`);
}

if (failures.length) {
  console.error(`\n${failures.length} check(s) failed: ${failures.join(", ")}`);
  process.exit(1);
}
console.log(`\nnodejs bundle smoke: all checks passed (${bundleDir})`);
