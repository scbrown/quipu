// Playwright driver for the quipu wasm harness (quipu-qd2 acceptance).
//
// Scenario 1 (memory VFS): open, ingest, query — same page.
// Scenario 2 (OPFS): install opfs-sahpool as default VFS, ingest, query;
//   page.reload() and query again (the bead's acceptance criterion); then a
//   full browser relaunch on the same profile and query a third time.
//
// Prereqs: `wasm-bindgen --target web` output in www/pkg/ (see README.md),
// and a `playwright` resolvable from this directory (npm link or install).
//
// Usage: node run.mjs [--headed]

import http from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const www = path.join(here, "www");
const profile = path.join(here, ".profile"); // OPFS lives in the profile
const EPISODES = 50;
// scale_bench shape: 3 typed nodes per episode, service-i unique per episode.
const EXPECT = { scan: EPISODES, join: EPISODES, pointMin: 1 };

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

const headless = !process.argv.includes("--headed");
const launch = () =>
  chromium.launchPersistentContext(profile, {
    headless,
    args: ["--no-sandbox"],
  });

let failures = 0;
const check = (label, cond, detail) => {
  console.log(`${cond ? "PASS" : "FAIL"}  ${label}${detail ? ` — ${detail}` : ""}`);
  if (!cond) failures++;
};

const readChecks = (label, r) => {
  check(`${label}: point lookup ≥ ${EXPECT.pointMin}`, r.point >= EXPECT.pointMin, `got ${r.point}`);
  check(`${label}: type scan = ${EXPECT.scan}`, r.scan === EXPECT.scan, `got ${r.scan}`);
  check(`${label}: 2-hop join = ${EXPECT.join}`, r.join === EXPECT.join, `got ${r.join}`);
};

// --- Scenario 1: memory VFS ------------------------------------------------
{
  const ctx = await launch();
  const page = await ctx.newPage();
  await page.goto(origin);
  const w = await page.evaluate(
    (n) => window.ask({ cmd: "write", path: "mem-harness.db", n }),
    EPISODES,
  );
  check("memory: ingest returns triples", w.triples > 0, `${w.triples} triples`);
  const r = await page.evaluate(() => window.ask({ cmd: "read", path: "mem-harness.db" }));
  readChecks("memory", r);
  const jm = await page.evaluate(() => window.ask({ cmd: "journal_mode", path: "mem-jm.db" }));
  console.log(`INFO  memory: journal_mode=WAL request →`, JSON.stringify(jm));
  await ctx.close();
}

// --- Scenario 2: OPFS, reload, relaunch -------------------------------------
{
  const ctx = await launch();
  const page = await ctx.newPage();
  await page.goto(origin);
  await page.evaluate(() => window.ask({ cmd: "install_opfs" }));
  const w = await page.evaluate(
    (n) => window.ask({ cmd: "write", path: "opfs-harness.db", n }),
    EPISODES,
  );
  check("opfs: ingest returns triples", w.triples > 0, `${w.triples} triples`);
  const jm2 = await page.evaluate(() => window.ask({ cmd: "journal_mode", path: "opfs-jm.db" }));
  console.log(`INFO  opfs: journal_mode=WAL request →`, JSON.stringify(jm2));
  readChecks("opfs same-page", await page.evaluate(() => window.ask({ cmd: "read", path: "opfs-harness.db" })));

  await page.reload();
  await page.evaluate(() => window.ask({ cmd: "install_opfs" }));
  readChecks(
    "opfs AFTER PAGE RELOAD",
    await page.evaluate(() => window.ask({ cmd: "read", path: "opfs-harness.db" })),
  );
  await ctx.close();

  const ctx2 = await launch();
  const page2 = await ctx2.newPage();
  await page2.goto(origin);
  await page2.evaluate(() => window.ask({ cmd: "install_opfs" }));
  readChecks(
    "opfs AFTER BROWSER RELAUNCH",
    await page2.evaluate(() => window.ask({ cmd: "read", path: "opfs-harness.db" })),
  );
  await ctx2.close();
}

server.close();
console.log(failures ? `\n${failures} check(s) FAILED` : "\nall checks passed");
process.exit(failures ? 1 : 0);
