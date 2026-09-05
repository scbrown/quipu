// Acceptance for the packaged wasm bundle, run in a real browser at release
// time (aegis-tpqccc).
//
// The failure this exists to catch is a bundle that BUILDS and SHIPS and cannot
// do its job: `cargo build` proves it compiles, `wasm-bindgen` proves the glue
// generated, and neither of them ever loads the module. A version-mismatched
// glue file, a missing export, an abort at instantiation, or an import path
// that no longer works are all invisible upstream of here and all fatal on the
// book page.
//
// So this drives the packaged directory exactly as the page does — ES module,
// dedicated worker, a real .qpack produced by the native binary in the same run
// — and asserts on counts, not on the absence of an exception.
//
// usage: node scripts/smoke-wasm-explorer.mjs <packaged-bundle-dir> [--headed]

import { chromium } from "playwright";
import { createServer } from "node:http";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, rmSync, existsSync }
  from "node:fs";
import { Buffer } from "node:buffer";
import { tmpdir } from "node:os";
import { join, resolve, extname } from "node:path";

const bundleDir = process.argv[2];
if (!bundleDir) {
  console.error("usage: node scripts/smoke-wasm-explorer.mjs <bundle-dir> [--headed]");
  process.exit(2);
}
const headed = process.argv.includes("--headed");
const repo = resolve(import.meta.dirname, "..");

const failures = [];
const check = (name, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${name}${detail === undefined ? "" : `  — ${detail}`}`);
  if (!ok) failures.push(name);
};

const work = mkdtempSync(join(tmpdir(), "quipu-wasm-smoke-"));
process.on("exit", () => rmSync(work, { recursive: true, force: true }));

// ---- 1. Produce a real pack with the NATIVE binary ------------------------
//
// A fixture checked into the repo would go stale against the manifest format
// the very release this job is cutting. Minting it here means the two halves of
// the round trip are the same commit by construction.

const quipu = process.env.QUIPU_BIN ?? join(repo, "target/release/quipu");
if (!existsSync(quipu)) {
  console.error(
    `native quipu binary not found at ${quipu}\n` +
    "  This script mints a share with the NATIVE binary and imports the result back\n" +
    "  with it, so the calling job must build it first:\n" +
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
// Two typed subjects and an edge between them: enough that a count is a real
// assertion rather than a non-zero smell test.
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

const packName = "smoke.qpack.tar.gz";
execFileSync("tar", ["--sort=name", "--mtime=UTC 1970-01-01", "--owner=0", "--group=0",
  "--numeric-owner", "-C", shareDir, "-czf", join(work, packName), "."]);
const producerManifest = JSON.parse(readFileSync(join(shareDir, "manifest.json"), "utf8"));

// ---- 2. Serve the packaged bundle exactly as the book page would ----------

const site = join(work, "site");
mkdirSync(join(site, "pkg"), { recursive: true });
for (const f of ["quipu_wasm_explorer.js", "quipu_wasm_explorer_bg.wasm"]) {
  copyFileSync(join(bundleDir, f), join(site, "pkg", f));
}
copyFileSync(join(work, packName), join(site, packName));
// The worker the release README documents, and the shape the book page uses.
writeFileSync(join(site, "worker.js"), `
import init, { Explorer, explorerVersion } from "./pkg/quipu_wasm_explorer.js";
const ready = init();
let ex = null;
onmessage = async (e) => {
  const { id, cmd, bytes, sparql, entity, predicate, value: v, episode } = e.data;
  try {
    await ready;
    let value = null;
    if (cmd === "version") value = JSON.parse(explorerVersion());
    else if (cmd === "load") {
      ex = Explorer.loadQpack(new Uint8Array(bytes), "smoke", new Date().toISOString());
      value = JSON.parse(ex.loadReport());
    } else if (cmd === "query") value = JSON.parse(ex.query(sparql));
    else if (cmd === "set") value = JSON.parse(ex.set(entity, predicate, v));
    else if (cmd === "retract") value = JSON.parse(ex.retract(entity, predicate ?? "", v ?? ""));
    else if (cmd === "episode") value = JSON.parse(ex.episode(episode));
    else if (cmd === "editLog") value = JSON.parse(ex.editLog());
    else if (cmd === "exportManifest") value = JSON.parse(ex.exportManifest());
    else if (cmd === "exportPack") value = Array.from(ex.exportPack());
    else throw new Error("unknown cmd " + cmd);
    postMessage({ id, ok: true, value });
  } catch (err) { postMessage({ id, ok: false, error: String(err?.message ?? err) }); }
};
`);
writeFileSync(join(site, "index.html"), `<!doctype html><title>smoke</title><script>
const w = new Worker("./worker.js", { type: "module" });
let n = 1; const pending = new Map();
w.onmessage = (e) => { const p = pending.get(e.data.id); pending.delete(e.data.id);
  e.data.ok ? p.resolve(e.data.value) : p.reject(new Error(e.data.error)); };
w.onerror = (e) => { for (const p of pending.values()) p.reject(new Error(e.message)); };
window.ask = (msg) => new Promise((resolve, reject) => {
  const id = n++; pending.set(id, { resolve, reject }); w.postMessage({ id, ...msg }); });
</script><body>smoke</body>`);

const MIME = { ".html": "text/html", ".js": "text/javascript",
  ".wasm": "application/wasm", ".gz": "application/gzip" };
const server = createServer((req, res) => {
  const name = decodeURIComponent(req.url.split("?")[0]);
  const file = join(site, name === "/" ? "index.html" : name);
  try {
    const body = readFileSync(file);
    res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
    res.end(body);
  } catch { res.writeHead(404); res.end("not found"); }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/`;

// ---- 3. Drive it -----------------------------------------------------------

// CI uses the Playwright-managed Chromium. `CHROMIUM_PATH` is for hosts whose
// OS Playwright has no managed build for, where a system Chrome is the only
// browser available — the same escape hatch `QUIPU_BIN` gives the native half.
const browser = await chromium.launch({
  headless: !headed,
  ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}),
});
try {
  const page = await browser.newPage();
  const consoleErrors = [];
  page.on("pageerror", (e) => consoleErrors.push(String(e)));
  await page.goto(base);

  const build = await page.evaluate(() => window.ask({ cmd: "version" }));
  check("the module instantiates and reports its build",
    typeof build.version === "string" && build.version.length > 0,
    `quipu ${build.version} (${String(build.git_sha).slice(0, 12)})`);

  const report = await page.evaluate(async (url) => {
    const bytes = await (await fetch(url)).arrayBuffer();
    return window.ask({ cmd: "load", bytes });
  }, `./${packName}`);

  check("the manifest survives the round trip",
    report.manifest.share_id === producerManifest.share_id,
    report.manifest.share_id);
  check("the graph hash survives the round trip",
    report.manifest.graph_hash === producerManifest.graph_hash);
  check("the import staged rather than quarantined",
    report.import.outcome === "staged", report.import.outcome);
  check("no triple was quarantined",
    report.import.triples.quarantined === 0,
    `accepted ${report.import.triples.accepted}`);
  check("the bundled shapes were adopted, so nothing is off-vocabulary",
    report.import.validation.off_vocabulary.length === 0,
    JSON.stringify(report.import.validation.off_vocabulary));
  check("the staged graph was promoted into ROOT",
    report.promotion?.outcome === "promoted",
    JSON.stringify(report.promotion ?? null));

  // The point of the whole exercise: the graph is QUERYABLE, and the answer is
  // the data the native binary put in — not merely a non-empty result.
  const widgets = await page.evaluate(() => window.ask({ cmd: "query", sparql: `
    PREFIX ex: <http://example.org/>
    PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
    SELECT ?label WHERE { ?w a ex:Widget ; rdfs:label ?label } ORDER BY ?label` }));
  const labels = (widgets.rows ?? []).map((r) => r.label);
  check("SPARQL returns exactly the seeded subjects",
    JSON.stringify(labels) === JSON.stringify(["Alpha", "Beta"]),
    JSON.stringify(labels));

  const join2 = await page.evaluate(() => window.ask({ cmd: "query", sparql: `
    PREFIX ex: <http://example.org/>
    PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
    SELECT ?to WHERE { ex:alpha ex:connects ?o . ?o rdfs:label ?to }` }));
  check("a two-hop join resolves across the imported edge",
    (join2.rows ?? []).length === 1 && join2.rows[0].to === "Beta",
    JSON.stringify(join2.rows ?? []));

  // ---- the EDIT round trip -------------------------------------------------
  //
  // The read half above proves the browser can consume what the CLI produces.
  // This proves the other direction, which is the half that is easy to ship
  // broken: a store edited in a tab must come out as a share the NATIVE binary
  // will accept, verify and query. Everything short of that — a successful
  // `set`, a plausible-looking download — is the write reporting on itself.

  const setResult = await page.evaluate(() => window.ask({
    cmd: "set", entity: "http://example.org/alpha",
    predicate: "http://www.w3.org/2000/01/rdf-schema#comment",
    value: JSON.stringify({ str: "edited in a browser tab" }),
  }));
  check("a /set write lands and reports a transaction",
    setResult.tx_id > 0 && setResult.asserted >= 1,
    JSON.stringify(setResult));

  const readBack = await page.evaluate(() => window.ask({ cmd: "query", sparql: `
    PREFIX ex: <http://example.org/>
    PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
    SELECT ?c WHERE { ex:alpha rdfs:comment ?c }` }));
  check("the edit is visible to SPARQL in the same tab",
    (readBack.rows ?? []).length === 1 && readBack.rows[0].c === "edited in a browser tab",
    JSON.stringify(readBack.rows ?? []));

  // /set is single-valued by definition. If it appended instead of replacing,
  // the count above would be 2 and this is what would say so.
  const setAgain = await page.evaluate(() => window.ask({
    cmd: "set", entity: "http://example.org/alpha",
    predicate: "http://www.w3.org/2000/01/rdf-schema#comment",
    value: JSON.stringify({ str: "edited twice" }),
  }));
  const afterSecond = await page.evaluate(() => window.ask({ cmd: "query", sparql: `
    PREFIX ex: <http://example.org/>
    PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
    SELECT (COUNT(?c) AS ?n) WHERE { ex:alpha rdfs:comment ?c }` }));
  check("a second /set REPLACES rather than appends",
    Number(afterSecond.rows?.[0]?.n) === 1 && setAgain.retracted >= 1,
    `count=${afterSecond.rows?.[0]?.n} retracted=${setAgain.retracted}`);

  // The closed-vocabulary gate is a FEATURE of the edit surface, not an
  // obstacle to it: the sender's shapes bound what a receiver may add.
  const ungoverned = await page.evaluate(() => window.ask({
    cmd: "episode",
    episode: JSON.stringify({
      name: "smoke-ungoverned", episode_body: "should be refused", source: "smoke",
      nodes: [{ name: "nope", type: "NotAGovernedType", description: "x" }], edges: [],
    }),
  }).then(() => null, (e) => e.message));
  check("an ungoverned node type is REFUSED, not written",
    typeof ungoverned === "string" && ungoverned.length > 0, ungoverned);

  // A query result is where a UI GETS an IRI to write to, and `tool_query`
  // COMPACTS IRIs on the way out (`rdfs:label`), while the write tools resolve
  // them literally. Writing back what you read therefore fails with
  // `predicate not found` — found by clicking Retract on a real fact, and
  // invisible to every check above, all of which write hand-typed full IRIs.
  // Pinned here in both directions so the compaction cannot change under the
  // page without something going red.
  const discovered = await page.evaluate(() => window.ask({ cmd: "query", sparql: `
    PREFIX ex: <http://example.org/>
    SELECT ?p ?pIri WHERE { ex:alpha ?p ?o . BIND(STR(?p) AS ?pIri) }` }));
  const compacted = (discovered.rows ?? []).find((r) => r.p !== r.pIri);
  check("a query result really does compact IRIs (the control for the next two)",
    Boolean(compacted), JSON.stringify((discovered.rows ?? []).slice(0, 4)));

  if (compacted) {
    const withCompact = await page.evaluate((p) => window.ask({
      cmd: "retract", entity: "http://example.org/alpha", predicate: p,
    }).then(() => null, (e) => e.message), compacted.p);
    check("retracting with the COMPACTED form is refused, not silently ignored",
      typeof withCompact === "string" && withCompact.includes("not found"),
      `${compacted.p} -> ${withCompact}`);

    const withFull = await page.evaluate((p) => window.ask({
      cmd: "retract", entity: "http://example.org/alpha", predicate: p,
    }), compacted.pIri);
    check("retracting with the FULL IRI from STR() succeeds",
      withFull.retracted >= 1, `${compacted.pIri} -> retracted ${withFull.retracted}`);
  }

  const editLog = await page.evaluate(() => window.ask({ cmd: "editLog" }));
  check("the edit log records the writes that landed and not the ones that did not",
    editLog.length === 3
      && editLog.filter((e) => e.op === "set").length === 2
      && editLog.filter((e) => e.op === "retract").length === 1,
    JSON.stringify(editLog.map((e) => e.op)));

  const exportManifest = await page.evaluate(() => window.ask({ cmd: "exportManifest" }));
  check("the exported pack declares the pack it came from as its parent",
    exportManifest.parent_share === producerManifest.share_id,
    `${exportManifest.parent_share} (parent) vs ${producerManifest.share_id} (source)`);
  check("editing changed the graph hash",
    exportManifest.graph_hash !== producerManifest.graph_hash);

  // Ship the bytes out and hand them to the NATIVE binary. This is the check
  // the whole edit surface exists to pass.
  const packBytes = await page.evaluate(() => window.ask({ cmd: "exportPack" }));
  const edited = join(work, "edited.qpack.tar.gz");
  writeFileSync(edited, Buffer.from(packBytes));
  check("the exported pack is a non-trivial archive", packBytes.length > 200,
    `${packBytes.length} bytes`);

  // EXTRACT, then import the DIRECTORY — not the archive.
  //
  // `quipu import <archive>` routes through `share_transport::import_in_memory`
  // (cli_pack.rs), which builds a FRESH in-memory store and IGNORES `--db`. A
  // fresh store has no shapes, so every type in the pack is off-vocabulary and
  // the result is always `quarantined`. That is a transient verification, not
  // an import into your store, and it cannot show the edit arriving. Measured
  // both ways on one store and one pack: directory -> staged, 5 accepted;
  // archive -> quarantined, off_vocabulary [ex:Widget], "no local shapes
  // loaded". Same recipe `scripts/build-repository-share.sh` uses, and for the
  // same reason.
  const roundDb = join(work, "roundtrip.db");
  const unpacked = join(work, "unpacked");
  mkdirSync(unpacked, { recursive: true });
  execFileSync("tar", ["-C", unpacked, "-xzf", edited]);
  run(["shapes", "load", "smoke", shapes, "--db", roundDb]);
  const importJson = JSON.parse(run(["import", unpacked, "--db", roundDb]));
  check("`quipu import` accepts the browser-produced pack",
    importJson.outcome === "staged" && importJson.promotion.eligible === true,
    `${importJson.outcome}, eligible=${importJson.promotion?.eligible}, ` +
    `blockers=${JSON.stringify(importJson.promotion?.blockers)}, ` +
    `offVocab=${JSON.stringify(importJson.validation?.off_vocabulary)}`);
  check("the native importer verifies it against the browser's own manifest",
    importJson.share_id === exportManifest.share_id,
    `${importJson.share_id} vs ${exportManifest.share_id}`);
  run(["import", "promote", importJson.share_id, "--db", roundDb]);

  const nativeSees = run(["query",
    `PREFIX ex: <http://example.org/>
     PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
     SELECT ?c WHERE { ex:alpha rdfs:comment ?c }`, "--db", roundDb]);
  check("the browser's edit survives into a native store and is queryable there",
    nativeSees.includes("edited twice"),
    nativeSees.replace(/\s+/g, " ").slice(0, 160));

  // Refusing a malformed pack is part of the contract, and a bundle that
  // accepted anything would pass every check above.
  const refused = await page.evaluate(() =>
    window.ask({ cmd: "load", bytes: new Uint8Array([1, 2, 3, 4, 5]).buffer })
      .then(() => null, (e) => e.message));
  check("a malformed pack is refused rather than silently accepted",
    typeof refused === "string" && refused.length > 0, refused);

  check("no uncaught page errors", consoleErrors.length === 0, consoleErrors.join(" | "));
} finally {
  await browser.close();
  server.close();
}

if (failures.length) {
  console.error(`\n${failures.length} check(s) failed: ${failures.join(", ")}`);
  process.exit(1);
}
console.log("\nall checks passed");
