// Browser acceptance for the actual book output and its released Quipu runtime.
// Usage: node scripts/smoke-datalinks.cjs [http://host/datalinks/]
// Omit the URL to serve docs/book/book on an ephemeral local port.
const { chromium } = require('playwright');
const { createServer } = require('node:http');
const { readFileSync } = require('node:fs');
const { resolve, extname, sep } = require('node:path');
const { createHash } = require('node:crypto');
const assert = require('node:assert/strict');

(async () => {
  let server, browser;
  try {
    let url = process.argv[2];
    if (!url) {
      const root = resolve('docs/book/book');
      server = createServer((req, res) => {
        const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
        const path = resolve(root, '.' + pathname + (pathname.endsWith('/') ? 'index.html' : ''));
        if (!path.startsWith(root + sep)) { res.writeHead(403); return res.end(); }
        try {
          const bytes = readFileSync(path);
          const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.gz': 'application/gzip' };
          res.setHeader('Content-Type', mime[extname(path)] || 'application/octet-stream');
          res.end(bytes);
        } catch { res.writeHead(404); res.end(); }
      });
      await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
      url = `http://127.0.0.1:${server.address().port}/datalinks/`;
    }
    browser = await chromium.launch({ headless: process.env.CI === 'true' });
    const page = await browser.newPage();
    const requests = [], errors = [];
    page.on('request', req => requests.push(req.url()));
    page.on('pageerror', err => errors.push(err.message));
    await page.goto(url);
    await page.waitForFunction(() => !document.querySelector('#loading') || document.querySelector('#loading').textContent.startsWith('Could not load:'), null, { timeout: 60000 });
    assert.equal(await page.locator('#loading').count(), 0, 'demo must finish loading');
    assert.equal(await page.locator('#counts').textContent(), '374 nodes · 362 edges · 17 ranks');
    assert.equal(await page.locator('canvas').count(), 1);
    assert(requests.some(r => r.endsWith('/demo.qpack.tar.gz')), 'pack fetched');
    assert(requests.some(r => r.endsWith('/quipu_wasm_explorer_bg.wasm')), 'real Quipu wasm loaded');
    assert(!requests.some(r => r.endsWith('/graph.json')), 'no JSON fallback');
    assert.deepEqual(errors, []);
    const result = await page.evaluate(async () => {
      const { loadDemo } = await import('./load-demo.js');
      const { graph, enrichment, report } = await loadDemo();
      const facts = graph.nodes.map(n => ['node', n.iri, n.label, n.type, n.deg]);
      for (const [a, b, p] of graph.edges) facts.push(['edge', graph.nodes[a].iri, graph.nodes[b].iri, p]);
      for (const [s, props] of Object.entries(enrichment))
        for (const [p, o] of Object.entries(props)) facts.push(['extra', s, p, String(o)]);
      return { facts: facts.map(f => JSON.stringify(f)).sort().join('\n'), report };
    });
    // Digest of the original display facts: labels/types/degrees, every edge,
    // and all enrichment. Index order/layout may change; the demo data may not.
    assert.equal(createHash('sha256').update(result.facts).digest('hex'),
      '05d2a16fff06816b30a506e8cf103aabbb780abae38cfaab58338a0bc4f084e0');
    assert(result.report.promotion, 'canonical import was promoted');
    console.log('PASS canonical qpack: exact demo data, 374 nodes / 362 edges / 17 ranks');
    if (process.env.SCREENSHOT) await page.screenshot({ path: process.env.SCREENSHOT });

    // A damaged artifact must produce a visible error, never the old JSON view.
    await page.route('**/demo.qpack.tar.gz', route => route.fulfill({ body: 'damaged pack' }));
    await page.reload();
    await page.waitForFunction(() => document.querySelector('#loading')?.textContent.startsWith('Could not load:'), null, { timeout: 60000 });
    assert.equal(await page.locator('canvas').count(), 0);
    assert.equal(await page.locator('#counts').textContent(), '');
    console.log('PASS damaged qpack: visible refusal, no rendered fallback');
  } finally {
    await browser?.close();
    if (server) await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
