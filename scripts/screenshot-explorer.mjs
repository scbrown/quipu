// Regenerate assets/explore-page.png — the README's screenshot of the
// "Explore this repository's graph" page (aegis-tpqccc).
//
// A script rather than a one-off, because a screenshot in a README rots
// silently: the page changes, the image does not, and nothing fails. This makes
// refreshing it one command against a locally built book, and it waits for the
// page to have actually LOADED a pack rather than photographing a spinner.
//
// usage:
//   just docs build && just explorer      # produce docs/book/book/explore/
//   node scripts/screenshot-explorer.mjs [--out PATH] [--full]
//
// Default is a VIEWPORT capture, because the README is where this image is
// used: a full-page shot of this page is ~6000px tall, so GitHub scales it to
// thumbnail width and every number on it becomes unreadable. `--full` gives the
// whole page for when you want to see the layout end to end.

import { chromium } from "playwright";
import { createServer } from "node:http";
import { readFileSync, mkdirSync } from "node:fs";
import { join, resolve, extname, dirname } from "node:path";

const repo = resolve(import.meta.dirname, "..");
const root = join(repo, "docs/book/book");
const outIdx = process.argv.indexOf("--out");
const out = outIdx > -1 ? process.argv[outIdx + 1] : join(repo, "assets/explore-page.png");
const fullPage = process.argv.includes("--full");

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css",
  ".wasm": "application/wasm", ".gz": "application/gzip", ".json": "application/json",
  ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2" };
const server = createServer((req, res) => {
  let name = decodeURIComponent(req.url.split("?")[0]);
  if (name.endsWith("/")) name += "index.html";
  try {
    const body = readFileSync(join(root, name));
    res.writeHead(200, { "content-type": MIME[extname(name)] ?? "application/octet-stream" });
    res.end(body);
  } catch { res.writeHead(404); res.end("not found"); }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;

const browser = await chromium.launch({
  ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}),
});
try {
  const page = await browser.newPage({
    viewport: { width: 1280, height: fullPage ? 1180 : 1000 },
    deviceScaleFactor: 2,
  });
  await page.goto(`${base}/explore/`);

  // Photograph a LOADED page or fail — an image of the loading state would be a
  // worse lie than no image, because it looks like the page not working.
  await page.waitForFunction(
    () => /Loaded [\d,]+ triples/.test(document.querySelector("#status").textContent),
    null, { timeout: 120_000 });
  await page.waitForFunction(
    () => document.querySelectorAll("#types .bar-row").length > 0
       && document.querySelector("#graph-count").textContent.includes("nodes"),
    null, { timeout: 60_000 });
  await page.click("#canned button");
  await page.waitForFunction(
    () => /rows? in/.test(document.querySelector("#sparql-out p")?.textContent ?? ""),
    null, { timeout: 30_000 });
  await page.waitForTimeout(700);   // let the force layout settle
  // Clicking a canned query scrolls the box into view, so a viewport capture
  // taken now would start halfway down the page. Go back to the top: the frame
  // worth showing is the lede, the load result, the provenance block and the
  // type distribution — what the page IS, rather than one panel of it.
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForTimeout(250);

  // `position: sticky` renders at its scroll offset in a full-page capture, so
  // the header would land in the middle of the image on top of the content.
  // A capture-only override — the page itself wants the sticky header.
  if (fullPage) {
    await page.addStyleTag({ content: "header { position: static !important; }" });
  }
  await page.waitForTimeout(150);

  mkdirSync(dirname(out), { recursive: true });
  await page.screenshot({ path: out, fullPage });
  const status = await page.textContent("#status");
  console.log(`wrote ${out} (${fullPage ? "full page" : "viewport"})\n  ${status.trim()}`);
} finally {
  await browser.close();
  server.close();
}
