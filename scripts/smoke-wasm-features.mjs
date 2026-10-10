// Actual browser/worker feature acceptance against a native-produced pack.
import { chromium } from "../wasm/explorer/runtime/node_modules/playwright/index.mjs";
import { createServer } from "node:http";
import { readFileSync, mkdtempSync, rmSync } from "node:fs";
import { join, resolve, extname } from "node:path";
import { tmpdir } from "node:os";
import assert from "node:assert/strict";
import { featureFixture } from "./wasm-feature-fixture.mjs";

const repo = resolve(import.meta.dirname, "..");
const bundle = resolve(process.argv[2] ?? "wasm/explorer/pkg");
const runtime = resolve(process.argv[3] ?? "wasm/explorer/runtime/dist");
const work = mkdtempSync(join(tmpdir(), "quipu-wasm-features-"));
const binary = process.env.QUIPU_BIN ?? join(repo, "target/release/quipu");
const fixture = featureFixture(repo, binary, work);
const requests = [], remoteBodies = [];
const worker = `
import init, * as engine from "/pkg/quipu_wasm_explorer.js";
import { BrowserSemanticSearch, BrowserRemoteProvider, BrowserFederatedProvider } from "/runtime/browser-runtime.mjs";
const ready = init(); let ex, semantic;
onmessage = async ({data:m}) => {
 try {
  await ready; let value;
  if(m.cmd === "load") {
   ex = engine.Explorer.loadQpack(new Uint8Array(m.bytes), "feature-fixture", new Date().toISOString());
   semantic = new BrowserSemanticSearch(ex, {runtimeUrl:new URL("/runtime/transformers.mjs",location.href).href,
    modelPath:new URL("/runtime/models/",location.href).href,wasmPath:new URL("/runtime/",location.href).href});
   value = {report:JSON.parse(ex.loadReport()),version:JSON.parse(engine.explorerVersion()),semantic:semantic.capabilities()};
  } else if(m.cmd === "query") value = JSON.parse(ex.query(m.sparql));
  else if(m.cmd === "episode") value = JSON.parse(ex.episode(JSON.stringify(m.input)));
  else if(m.cmd === "set") value = JSON.parse(ex.set(m.entity,m.predicate,JSON.stringify(m.value)));
  else if(m.cmd === "retract") value = JSON.parse(ex.retract(m.entity,m.predicate,JSON.stringify(m.value)));
  else if(m.cmd === "ontology") value = JSON.parse(ex.ontology(JSON.stringify(m.input)));
  else if(m.cmd === "validate") value = JSON.parse(engine.validateShapes(m.shapes,m.data));
  else if(m.cmd === "semantic") value = await semantic.search(m.query,m.limit);
  else if(m.cmd === "remote") {
   const remote = new BrowserRemoteProvider(engine,m.options);
   value = await remote.query(m.sparql,m.graph);
  } else if(m.cmd === "federate") {
   const remotes = m.remotes.map(options=>new BrowserRemoteProvider(engine,options));
   value = await new BrowserFederatedProvider(ex,remotes,m.floor).query(m.sparql);
  } else throw new Error("unknown command");
  postMessage({id:m.id,ok:true,value});
 } catch(error) {postMessage({id:m.id,ok:false,error:String(error?.message??error),validation:error?.validation});}
};`;
const html = `<!doctype html><title>Private WASM feature acceptance</title><script>
const worker=new Worker('/worker.mjs',{type:'module'}); const pending=new Map(); let seq=0;
worker.onmessage=({data:m})=>{const p=pending.get(m.id);pending.delete(m.id);p.resolve(m)};
worker.onerror=e=>{for(const p of pending.values())p.reject(new Error(e.message))};
window.ask=msg=>new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve,reject});worker.postMessage({...msg,id})});
</script>`;
const server = createServer(async (req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  requests.push(pathname);
  if (pathname === "/") { res.setHeader("Content-Type", "text/html"); res.end(html); return; }
  if (pathname === "/worker.mjs") { res.setHeader("Content-Type", "text/javascript"); res.end(worker); return; }
  if (pathname.endsWith("/query")) {
    let bytes = ""; for await (const chunk of req) bytes += chunk;
    remoteBodies.push(JSON.parse(bytes));
    if (pathname === "/failed/query") { res.writeHead(503); res.end("unavailable"); return; }
    if (pathname === "/slow/query") { setTimeout(() => res.end("{}"), 250); return; }
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify({ variables: ["s"], rows: [{ s: "http://example.org/remote" }] })); return;
  }
  const root = pathname.startsWith("/pkg/") ? bundle : runtime;
  const relative = pathname.replace(/^\/(pkg|runtime)\//, "");
  const file = resolve(root, relative);
  if (!file.startsWith(`${root}/`)) { res.writeHead(403); res.end(); return; }
  try {
    const body = readFileSync(file);
    res.setHeader("Content-Type", [".js", ".mjs"].includes(extname(file)) ? "text/javascript" :
      extname(file) === ".wasm" ? "application/wasm" : "application/json");
    res.end(body);
  } catch { res.writeHead(404); res.end(); }
});

let browser;
try {
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  const base = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: !process.argv.includes("--headed"),
    ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}) });
  const page = await browser.newPage();
  const errors = []; page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(base);
  const ask = (input) => page.evaluate((m) => window.ask(m), input);
  const ok = async (input) => { const result = await ask(input); assert.equal(result.ok, true, result.error); return result.value; };
  const loaded = await ok({ cmd: "load", bytes: Array.from(readFileSync(fixture.pack)) });
  assert.equal(loaded.report.shacl_compiled, true);
  assert.equal(loaded.report.import.validation.conforms, true);
  assert.equal(loaded.report.promotion.outcome, "promoted");
  assert.deepEqual(loaded.version.compiled_features, { shacl: true, owl: true, reactive_reasoner: true });
  assert.equal(loaded.semantic.state, "not_loaded");
  assert.equal(requests.filter((url) => /transformers|\/models\/|ort-wasm/.test(url)).length, 0);
  console.log("PASS real pack load, compiled flags, no heavy assets before search");

  const bad = await ask({ cmd: "episode", input: { name: "invalid-widget", source: "feature-control",
    nodes: [{ name: "invalid-pattern", type: "InternalIdentifierPattern" }] } });
  assert.equal(bad.ok, false); assert.match(bad.error, /SHACL/);
  assert.ok(bad.validation?.violations > 0); assert.ok(bad.validation.messages.length > 0);
  const widgets = "SELECT ?s WHERE { ?s a <http://example.org/Widget> }";
  assert.equal((await ok({ cmd: "query", sparql: widgets })).rows.length, 2);
  const validation = await ok({ cmd: "validate", shapes: readFileSync(fixture.shapes, "utf8"),
    data: "<http://example.org/bad> a <http://example.org/Widget> ." });
  assert.equal(validation.conforms, false); assert.ok(validation.results.length > 0);
  console.log("PASS violating write refused with real feedback; existing two widgets preserved");

  const edge = (predicate, subject = "car", object = "cat") =>
    `SELECT ?s FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred> WHERE { <http://example.org/${subject}> <http://example.org/${predicate}> ?s . FILTER(?s=<http://example.org/${object}>) }`;
  assert.equal((await ok({ cmd: "query", sparql: edge("derivedBack") })).rows.length, 0);
  await ok({ cmd: "set", entity: "http://example.org/cat", predicate: "http://example.org/connects", value: { iri: "http://example.org/car" } });
  assert.equal((await ok({ cmd: "query", sparql: edge("derivedBack") })).rows.length, 1);
  await ok({ cmd: "ontology", input: { action: "load", name: "inverse-control",
    turtle: "<http://example.org/connects> <http://www.w3.org/2002/07/owl#inverseOf> <http://example.org/owlBack> ." } });
  assert.equal((await ok({ cmd: "query", sparql: edge("owlBack") })).rows.length, 1);
  await ok({ cmd: "set", entity: "http://example.org/car", predicate: "http://example.org/connects", value: { iri: "http://example.org/cat" } });
  assert.equal((await ok({ cmd: "query", sparql: edge("owlBack", "cat", "car") })).rows.length, 1);
  console.log("PASS adopted Datalog 0->1 and OWL load plus later reactive inverse write");

  const result = await ok({ cmd: "semantic", query: "a pet cat playing", limit: 2 });
  // Preserve the engine's compact term representation in search results.
  assert.equal(result[0].entity, "ex:cat"); assert.ok(result[0].score > result[1].score);
  assert.ok(requests.some((url) => url.endsWith("model_quantized.onnx")));
  console.log("PASS semantic ranking over actual ROOT data with self-hosted ONNX");

  const sparql = "SELECT ?s WHERE { ?s a <http://example.org/Widget> }";
  const remote = { name: "peer", endpoint: `${base}/peer`, label: { freshness: "fresh" } };
  const wire = await ok({ cmd: "remote", options: remote, sparql, graph: "urn:fixture:selected" });
  assert.equal(wire.declared_label.freshness, "fresh"); assert.equal(remoteBodies.at(-1).graph, "urn:fixture:selected");
  const full = await ok({ cmd: "federate", remotes: [remote], floor: {}, sparql });
  assert.equal(full.complete, true); assert.equal(full.result.rows.length, 3);
  const partial = await ok({ cmd: "federate", remotes: [remote, { name: "failed", endpoint: `${base}/failed` }], floor: {}, sparql });
  assert.equal(partial.complete, false); assert.equal(partial.providers.find((p) => p.name === "failed").ok, false);
  const count = remoteBodies.length;
  const floor = await ask({ cmd: "federate", remotes: [remote], floor: { min_freshness: "fresh" }, sparql });
  assert.equal(floor.ok, false); assert.equal(remoteBodies.length, count);
  assert.equal((await ok({ cmd: "query", sparql })).rows.length, 2);
  const timeout = await ask({ cmd: "remote", options: { name: "slow", endpoint: `${base}/slow`, timeoutMs: 10 }, sparql });
  assert.equal(timeout.ok, false);
  console.log("PASS fetch selected graph, federation completeness, native floor before network, timeout");
  assert.deepEqual(errors, []);
  console.log("WASM full browser feature acceptance: all checks passed");
} finally {
  if (browser) await browser.close();
  server.close();
  rmSync(work, { recursive: true, force: true });
}
