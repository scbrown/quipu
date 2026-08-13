// Wasm half of the quipu-ajz spike: runs scenario_bench under the memory VFS
// and OPFS, read model off and on, in a fresh page each so nothing carries
// over. Prints one JSON object per configuration. Compare against
// `examples/wasm_native_baseline.rs` at the same episode count.
//
// Usage: node bench.mjs [episodes]

import http from "node:http";
import { readFile, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const www = path.join(here, "www");
const profile = path.join(here, ".bench-profile");
const EPISODES = Number(process.argv[2] ?? 1000);

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
};

const server = http
  .createServer(async (req, res) => {
    const file = path.join(www, req.url === "/" ? "index.html" : req.url);
    try {
      const body = await readFile(file);
      res.setHeader("content-type", MIME[path.extname(file)] ?? "application/octet-stream");
      res.end(body);
    } catch {
      res.statusCode = 404;
      res.end("not found");
    }
  })
  .listen(0);
await new Promise((r) => server.once("listening", r));
const origin = `http://localhost:${server.address().port}`;

// Fresh profile per invocation: OPFS state must not leak between runs.
await rm(profile, { recursive: true, force: true });

for (const vfs of ["memory", "opfs"]) {
  for (const rm_on of [false, true]) {
    const ctx = await chromium.launchPersistentContext(profile, {
      headless: true,
      args: ["--no-sandbox"],
    });
    const page = await ctx.newPage();
    await page.goto(origin);
    if (vfs === "opfs") {
      await page.evaluate(() => window.ask({ cmd: "install_opfs" }));
    }
    const dbPath = `bench-${vfs}-${rm_on ? "rm" : "sql"}.db`;
    const result = await page.evaluate(
      ({ dbPath, n, rm_on }) =>
        window.ask({ cmd: "bench", path: dbPath, n, read_model: rm_on }),
      { dbPath, n: EPISODES, rm_on },
    );
    console.log(JSON.stringify({ vfs, read_model: rm_on, ...result }));
    await ctx.close();
  }
}

server.close();
