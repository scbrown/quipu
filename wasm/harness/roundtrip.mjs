// quipu-2l5 acceptance driver: the .db file as interchange format, in every
// direction the design doc claims (wasm-support.md §6).
//
//   browser → native:  scenario_export bytes → file → opens in the quipu CLI
//                      (and sqlite3, when present) unchanged.
//   browser pack:      scenario_pack bytes → file → attach_pack_check
//                      (respace + attach + hash verify + query).
//   native → browser:  wasm_native_baseline produces a .db → bytes →
//                      scenario_import reads it in the tab.
//
// Native binaries are invoked through cargo (debug quipu CLI carries shacl —
// the default features; the examples build --no-default-features). Override
// with QUIPU_CLI / CARGO_PROFILE_FLAGS if a CI cache has them prebuilt.
//
// Usage: node roundtrip.mjs

import http from "node:http";
import { readFile, writeFile, rm, mkdtemp } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../..");
const www = path.join(here, "www");
const EPISODES = 25;

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

const work = await mkdtemp(path.join(os.tmpdir(), "quipu-roundtrip-"));
let failures = 0;
const check = (label, cond, detail) => {
  console.log(`${cond ? "PASS" : "FAIL"}  ${label}${detail ? ` — ${detail}` : ""}`);
  if (!cond) failures++;
};
const cargo = (args, opts = {}) =>
  execFileSync("cargo", args, { cwd: repo, encoding: "utf8", ...opts });

// Native producer for the import leg, and the example the attach leg needs —
// built up front so browser work isn't interleaved with compiles.
cargo(["build", "--release", "--no-default-features", "--examples"], { stdio: "inherit" });
const nativeDb = path.join(work, "native-produced.db");
cargo(["run", "--release", "--no-default-features", "--example", "wasm_native_baseline", "--",
  String(EPISODES), nativeDb], { stdio: "ignore" });

const ctx = await chromium.launchPersistentContext(path.join(work, "profile"), {
  headless: true,
  args: ["--no-sandbox"],
});
const page = await ctx.newPage();
await page.goto(origin);

// --- browser → native: export ------------------------------------------------
await page.evaluate(
  (n) => window.ask({ cmd: "write", path: "export-src.db", n }),
  EPISODES,
);
const exported = await page.evaluate(() => window.ask({ cmd: "export", path: "export-src.db" }));
const exportedDb = path.join(work, "wasm-export.db");
await writeFile(exportedDb, Buffer.from(exported));
check("export: bytes look like a SQLite db", exported.length > 4096,
  `${exported.length} bytes, header "${Buffer.from(exported.slice(0, 15)).toString()}"`);

// Opens in the quipu CLI unchanged (debug build carries default features).
const cli = cargo(["run", "--quiet", "--bin", "quipu", "--", "read",
  "SELECT ?s WHERE { ?s a <http://gastown.example/Service> } LIMIT 100",
  "--db", exportedDb]);
const cliRows = cli.trim().split("\n").filter((l) => l.includes("service-")).length;
check("export: opens in the quipu CLI, type scan intact", cliRows === EPISODES, `${cliRows} rows`);

// Opens in sqlite3 unchanged — when the CLI is present (CI has it).
try {
  const ic = execFileSync("sqlite3", [exportedDb, "PRAGMA integrity_check;"], { encoding: "utf8" });
  check("export: sqlite3 integrity_check", ic.trim() === "ok", ic.trim());
} catch (e) {
  if (e.code === "ENOENT") console.log("SKIP  export: sqlite3 CLI not installed here");
  else { check("export: sqlite3 integrity_check", false, String(e)); }
}

// --- browser pack → native attach ---------------------------------------------
const packBytes = await page.evaluate(() => window.ask({ cmd: "pack", path: "pack-src.db" }));
const packDb = path.join(work, "wasm-pack.db");
await writeFile(packDb, Buffer.from(packBytes));
try {
  const out = cargo(["run", "--release", "--no-default-features", "--example",
    "attach_pack_check", "--", packDb, "urn:g:browser-pack"]);
  check("pack: browser-produced pack attaches natively", out.includes("attach_pack_check: ok"), out.trim());
} catch (e) {
  check("pack: browser-produced pack attaches natively", false, String(e.stdout || e));
}

// --- native → browser: import --------------------------------------------------
const nativeBytes = await readFile(nativeDb);
const imported = await page.evaluate(
  (bytes) => window.ask({ cmd: "import", bytes }),
  Array.from(nativeBytes),
);
check("import: native .db opens in the tab, type scan intact",
  imported.scan === EPISODES, `${imported.scan} rows`);

await ctx.close();
server.close();
await rm(work, { recursive: true, force: true });
console.log(failures ? `\n${failures} check(s) FAILED` : "\nall round-trips passed");
process.exit(failures ? 1 : 0);
