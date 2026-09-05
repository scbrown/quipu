// UI for the "Explore this repository's graph" page.
//
// Everything on screen is derived from a SPARQL query this file sends to the
// worker — there is no privileged read path. The queries are printed in the
// query box when you click a panel's "show the query" link, so a reader can
// take any view apart and re-run it themselves. That is deliberate: a demo you
// cannot reproduce is a screenshot with extra steps.

const AEGIS = "http://aegis.gastown.local/ontology/";
const PREFIXES = `PREFIX aegis: <${AEGIS}>
PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
`;

// ---------------------------------------------------------------- worker RPC

const worker = new Worker("./worker.js", { type: "module" });
let nextId = 1;
const pending = new Map();
worker.onmessage = (e) => {
  const { id, ok, value, error } = e.data;
  const p = pending.get(id);
  pending.delete(id);
  if (!p) return;
  ok ? p.resolve(value) : p.reject(new Error(error));
};
worker.onerror = (e) => {
  for (const p of pending.values()) p.reject(new Error(e.message || "worker failed"));
  pending.clear();
  fail(`The wasm worker failed to start: ${e.message || "unknown error"}`);
};
const ask = (msg) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    worker.postMessage({ id, ...msg }, msg.bytes ? [msg.bytes] : []);
  });

const query = (sparql) => ask({ cmd: "query", sparql });

// ------------------------------------------------------------------ helpers

const $ = (sel) => document.querySelector(sel);
const el = (tag, attrs = {}, ...kids) => {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else if (k === "text") node.textContent = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v);
  }
  for (const kid of kids) node.append(kid);
  return node;
};
const status = (msg) => { $("#status").textContent = msg; };
const fail = (msg) => {
  $("#status").textContent = msg;
  $("#status").classList.add("bad");
};
const fmt = (n) => Number(n).toLocaleString("en-US");
// Bobbin percent-encodes the path into the IRI; show people the path.
const short = (iri) => {
  if (typeof iri !== "string") return String(iri);
  const tail = iri.startsWith(AEGIS) ? iri.slice(AEGIS.length) : iri.replace(/^aegis:/, "");
  try { return decodeURIComponent(tail); } catch { return tail; }
};
const cell = (v) => (v && typeof v === "object" ? JSON.stringify(v) : String(v ?? ""));

// A query result's `rows` are already plain objects keyed by variable name.
const rows = (result) => (result && Array.isArray(result.rows) ? result.rows : []);

// --------------------------------------------------------------- provenance

function renderProvenance(report, source, timings) {
  const build = window.__quipuBuild ?? { version: "?", git_sha: "?" };
  const m = report.manifest;
  const imp = report.import;
  const facts = [
    ["Producer", `quipu ${m.producer.version}`],
    ["Explorer build", `quipu ${build.version} (${String(build.git_sha).slice(0, 12)})`],
    ["Pack built", m.created_at],
    ["Graph hash", m.graph_hash],
    ["Share id", m.share_id],
    ["Canonicalization", m.canonicalization ?? "(none declared)"],
    ["Tx anchor", String(m.tx_anchor)],
    ["Source", source],
    ["Import outcome", imp.outcome],
    ["Triples accepted", fmt(imp.triples.accepted)],
    ["Quarantined", fmt(imp.triples.quarantined)],
    ["Off-vocabulary types", imp.validation.off_vocabulary.length
      ? imp.validation.off_vocabulary.map(short).join(", ")
      : "none"],
    ["Promotion", report.promotion
      ? `${report.promotion.outcome} (tx ${report.promotion.tx_id}, ${fmt(report.promotion.triples)} triples)`
      : `refused — ${imp.promotion.blockers.join(", ") || "not eligible"}`],
    ["Load time", `${(timings.load / 1000).toFixed(1)} s in this tab`],
  ];
  const dl = $("#provenance");
  dl.replaceChildren();
  for (const [k, v] of facts) {
    dl.append(el("dt", { text: k }), el("dd", { class: "mono", text: v }));
  }

  // Say what this build did NOT check, rather than letting `conforms: true`
  // read as a clean bill of health it did not earn.
  $("#shacl-note").textContent = report.shacl_compiled
    ? "SHACL is compiled into this build, and the report above is a real validation result."
    : "SHACL is NOT compiled into this browser build, so “conforms” above is a default, not a "
      + "finding. The vocabulary check (off-vocabulary types) DID run — it does not need the "
      + "SHACL engine. Run `quipu import` locally for the full validation.";
}

// Release metadata is the one cross-origin call on this page: api.github.com
// sends `access-control-allow-origin: *`, while release-asset DOWNLOADS send no
// CORS header at all — which is exactly why the pack itself is served from this
// site rather than fetched from the release. Metadata only; failure is silent
// because it is a nicety, not the page.
async function reportReleaseFreshness(packVersion) {
  try {
    const r = await fetch("https://api.github.com/repos/scbrown/quipu/releases/latest");
    if (!r.ok) return;
    const latest = (await r.json()).tag_name?.replace(/^quipu-ai-v/, "");
    if (!latest) return;
    const note = $("#freshness");
    if (latest === packVersion) {
      note.textContent = `This is the current release (v${latest}).`;
    } else {
      note.textContent = `Heads up: the newest release is v${latest}; this page is serving the `
        + `pack from v${packVersion}. The site restages the pack on its next docs build.`;
      note.classList.add("warn");
    }
  } catch {
    /* offline, rate-limited, or blocked — the page works without it */
  }
}

// ---------------------------------------------------------------- the panels

const Q = {
  types: `${PREFIXES}
SELECT ?type (COUNT(?s) AS ?n)
WHERE { ?s rdf:type ?type }
GROUP BY ?type
ORDER BY DESC(?n)`,

  modules: `${PREFIXES}
SELECT ?module ?path ?language
WHERE {
  ?module rdf:type aegis:CodeModule ;
          aegis:filePath ?path .
  OPTIONAL { ?module aegis:language ?language }
}
ORDER BY ?path`,

  documents: `${PREFIXES}
SELECT ?document ?path
WHERE { ?document rdf:type aegis:Document ; aegis:filePath ?path }
ORDER BY ?path`,
};

// The three canned queries the page ships with. Each is a real question the
// artifact can answer, not a shape-of-SPARQL demo.
const CANNED = [
  {
    name: "Which modules mention a name?",
    sparql: `${PREFIXES}
# Every module holding a chunk that mentions "Store".
SELECT DISTINCT ?path
WHERE {
  ?chunk aegis:mentions "Store" ;
         aegis:inDocument ?module .
  ?module aegis:filePath ?path .
}
ORDER BY ?path
LIMIT 50`,
  },
  {
    name: "What symbols does one file define?",
    sparql: `${PREFIXES}
# The symbol table for one source file, straight out of the pack.
SELECT ?name ?kind
WHERE {
  ?symbol rdf:type aegis:CodeSymbol ;
          aegis:filePath "src/share_transport.rs" ;
          aegis:name ?name .
  OPTIONAL { ?symbol aegis:symbolKind ?kind }
}
ORDER BY ?name`,
  },
  {
    name: "What is the chunk chain of a document?",
    sparql: `${PREFIXES}
# Chunks are a linked list: follow aegis:nextChunk through one document.
SELECT ?order ?chunk ?next
WHERE {
  ?chunk aegis:inDocument <${AEGIS}doc/quipu/README.md> ;
         aegis:chunkOrder ?order .
  OPTIONAL { ?chunk aegis:nextChunk ?next }
}
ORDER BY ?order`,
  },
];

async function renderTypes() {
  const result = await query(Q.types);
  const data = rows(result).map((r) => ({ type: short(r.type), n: Number(r.n) }));
  const max = Math.max(1, ...data.map((d) => d.n));
  const box = $("#types");
  box.replaceChildren();
  for (const d of data) {
    const bar = el("div", { class: "bar" });
    bar.style.width = `${Math.max(2, (d.n / max) * 100)}%`;
    box.append(
      el("div", { class: "bar-row", title: `${d.type}: ${fmt(d.n)}` },
        el("span", { class: "bar-label mono", text: d.type }),
        el("div", { class: "bar-track" }, bar),
        el("span", { class: "bar-n mono", text: fmt(d.n) })),
    );
  }
  $("#types-total").textContent =
    `${fmt(data.reduce((a, d) => a + d.n, 0))} typed subjects across ${data.length} types`;
  wireShowQuery("#types-showq", Q.types);
}

async function renderBrowser() {
  const [modules, documents] = await Promise.all([query(Q.modules), query(Q.documents)]);
  const items = [
    ...rows(modules).map((r) => ({ iri: r.module, path: r.path, kind: r.language || "code" })),
    ...rows(documents).map((r) => ({ iri: r.document, path: r.path, kind: "doc" })),
  ].sort((a, b) => a.path.localeCompare(b.path));

  const list = $("#file-list");
  const filter = $("#file-filter");
  const draw = () => {
    const needle = filter.value.trim().toLowerCase();
    const shown = needle ? items.filter((i) => i.path.toLowerCase().includes(needle)) : items;
    list.replaceChildren();
    for (const item of shown.slice(0, 400)) {
      list.append(el("button", {
        class: "file",
        onclick: () => selectFile(item),
      }, el("span", { class: "mono", text: item.path }), el("em", { text: item.kind })));
    }
    $("#file-count").textContent = shown.length > 400
      ? `showing 400 of ${fmt(shown.length)}`
      : `${fmt(shown.length)} files`;
  };
  filter.addEventListener("input", draw);
  draw();
  wireShowQuery("#browse-showq", Q.modules);
  if (items.length) selectFile(items.find((i) => i.path === "src/share.rs") ?? items[0]);
}

function detailQuery(iri) {
  return `${PREFIXES}
SELECT ?member ?name ?kind ?order
WHERE {
  ?member aegis:inDocument <${iri}> .
  OPTIONAL { ?member aegis:name ?name }
  OPTIONAL { ?member aegis:heading ?name }
  OPTIONAL { ?member aegis:symbolKind ?kind }
  OPTIONAL { ?member aegis:chunkOrder ?order }
}
ORDER BY ?order`;
}

let selected = null;

async function selectFile(item) {
  selected = item;
  $("#detail-title").textContent = item.path;
  $("#detail-iri").textContent = item.iri;
  const body = $("#detail-body");
  body.replaceChildren(el("p", { class: "muted", text: "querying…" }));
  const sparql = detailQuery(item.iri);
  wireShowQuery("#detail-showq", sparql);
  const result = await query(sparql);
  const members = rows(result);
  body.replaceChildren();
  if (!members.length) {
    body.append(el("p", { class: "muted", text: "No chunks recorded under this file." }));
  } else {
    const table = el("table");
    table.append(el("thead", {}, el("tr", {},
      el("th", { text: "#" }), el("th", { text: "name" }), el("th", { text: "kind" }))));
    const tbody = el("tbody");
    for (const m of members) {
      tbody.append(el("tr", {},
        el("td", { class: "mono num", text: m.order ?? "" }),
        el("td", { class: "mono", text: m.name ?? short(m.member) }),
        el("td", { text: m.kind ?? "" })));
    }
    table.append(tbody);
    body.append(table);
  }
  drawNeighbourhood(item);
}

// ------------------------------------------------------------ graph panel

// A small force layout on a 2D canvas. No library on purpose: the whole page is
// already a 3 MB wasm download, and a graph of one file's neighbourhood is a
// few dozen nodes — importing a renderer to draw it would cost more than it
// draws.
async function drawNeighbourhood(item) {
  const sparql = `${PREFIXES}
SELECT ?from ?rel ?to
WHERE {
  { <${item.iri}> ?rel ?to . FILTER(isIRI(?to)) BIND(<${item.iri}> AS ?from) }
  UNION
  { ?from ?rel <${item.iri}> . FILTER(isIRI(?from)) BIND(<${item.iri}> AS ?to) }
  UNION
  { ?from aegis:inDocument <${item.iri}> . ?from ?rel ?to . FILTER(isIRI(?to)) }
}
LIMIT 300`;
  wireShowQuery("#graph-showq", sparql);
  const result = await query(sparql).catch(() => null);
  const edges = rows(result)
    .filter((e) => e.from && e.to && e.from !== e.to)
    .map((e) => ({ from: String(e.from), to: String(e.to), rel: short(String(e.rel)) }));
  layout(edges, String(item.iri));
}

function layout(edges, focus) {
  const canvas = $("#graph");
  const ctx = canvas.getContext("2d");
  const dpr = window.devicePixelRatio || 1;
  const W = canvas.clientWidth, H = canvas.clientHeight;
  canvas.width = W * dpr; canvas.height = H * dpr;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  const ids = [...new Set(edges.flatMap((e) => [e.from, e.to]))];
  $("#graph-count").textContent = ids.length
    ? `${fmt(ids.length)} nodes, ${fmt(edges.length)} edges`
    : "no linked neighbours in the pack";
  ctx.clearRect(0, 0, W, H);
  if (!ids.length) return;

  // Simulate in a SQUARE space and fit the result to the canvas afterwards.
  // Simulating directly in canvas coordinates looked like a bug: this panel is
  // ~1180x320, so a symmetric repulsion runs out of vertical room long before
  // horizontal and parks half the nodes in rows along the top and bottom edges.
  // The graph was fine; the aspect ratio was doing the layout.
  const S = 1000;
  const nodes = new Map(ids.map((id, i) => {
    const a = (i / ids.length) * Math.PI * 2;
    return [id, { id, x: S / 2 + Math.cos(a) * S * 0.35, y: S / 2 + Math.sin(a) * S * 0.35,
                  vx: 0, vy: 0 }];
  }));
  const hub = nodes.get(focus);
  if (hub) { hub.x = S / 2; hub.y = S / 2; }

  const repulsion = (S * S) / (nodes.size * 45);
  for (let step = 0; step < 320; step++) {
    for (const a of nodes.values()) {
      for (const b of nodes.values()) {
        if (a === b) continue;
        const dx = a.x - b.x, dy = a.y - b.y;
        const d2 = Math.max(64, dx * dx + dy * dy);
        const f = repulsion / d2;
        a.vx += dx * f; a.vy += dy * f;
      }
    }
    for (const e of edges) {
      const a = nodes.get(e.from), b = nodes.get(e.to);
      if (!a || !b) continue;
      const dx = b.x - a.x, dy = b.y - a.y;
      const d = Math.max(1, Math.hypot(dx, dy));
      const f = (d - 110) * 0.010;
      a.vx += dx / d * f; a.vy += dy / d * f;
      b.vx -= dx / d * f; b.vy -= dy / d * f;
    }
    for (const n of nodes.values()) {
      n.vx += (S / 2 - n.x) * 0.004; n.vy += (S / 2 - n.y) * 0.004;
      n.x += (n.vx *= 0.85); n.y += (n.vy *= 0.85);
    }
    if (hub) { hub.x = S / 2; hub.y = S / 2; hub.vx = hub.vy = 0; }
  }

  // Fit whatever shape came out into the canvas. No clamping anywhere in the
  // simulation, so no node can be pressed flat against an edge by the geometry.
  const pad = { l: 16, r: 130, t: 16, b: 16 };   // right pad leaves room for labels
  const xs = [...nodes.values()].map((n) => n.x), ys = [...nodes.values()].map((n) => n.y);
  const minX = Math.min(...xs), maxX = Math.max(...xs);
  const minY = Math.min(...ys), maxY = Math.max(...ys);
  const sx = (W - pad.l - pad.r) / Math.max(1, maxX - minX);
  const sy = (H - pad.t - pad.b) / Math.max(1, maxY - minY);
  for (const n of nodes.values()) {
    n.px = pad.l + (n.x - minX) * sx;
    n.py = pad.t + (n.y - minY) * sy;
  }

  ctx.strokeStyle = "rgba(136, 146, 164, 0.3)";
  ctx.lineWidth = 1;
  for (const e of edges) {
    const a = nodes.get(e.from), b = nodes.get(e.to);
    if (!a || !b) continue;
    ctx.beginPath(); ctx.moveTo(a.px, a.py); ctx.lineTo(b.px, b.py); ctx.stroke();
  }
  ctx.font = "10px ui-monospace, SFMono-Regular, Menlo, monospace";
  for (const n of nodes.values()) {
    const isHub = n.id === focus;
    ctx.beginPath();
    ctx.arc(n.px, n.py, isHub ? 7 : 4, 0, Math.PI * 2);
    ctx.fillStyle = isHub ? "#e94560" : "#3987e5";
    ctx.fill();
    // Label the focus always, and the rest only while there is room. Past a
    // couple of dozen nodes the labels overlap into an unreadable smear, which
    // reads as a broken render rather than as a dense graph — so the honest
    // move is to stop drawing them and say how many there are, which the
    // heading beside this canvas already does.
    if (isHub || nodes.size <= 18) {
      ctx.fillStyle = isHub ? "#e0e0e0" : "#8892a4";
      ctx.fillText(short(n.id).split("/").pop().slice(0, 26), n.px + 9, n.py + 3);
    }
  }
}

// ------------------------------------------------------------- query panel

function wireShowQuery(sel, sparql) {
  const link = $(sel);
  if (!link) return;
  link.onclick = (e) => {
    e.preventDefault();
    $("#sparql").value = sparql;
    $("#sparql").scrollIntoView({ behavior: "smooth", block: "center" });
  };
}

async function runSparql() {
  const sparql = $("#sparql").value;
  const out = $("#sparql-out");
  out.replaceChildren(el("p", { class: "muted", text: "running…" }));
  const t0 = performance.now();
  let result;
  try {
    result = await query(sparql);
  } catch (err) {
    out.replaceChildren(el("pre", { class: "bad", text: String(err.message) }));
    return;
  }
  const ms = performance.now() - t0;
  const data = rows(result);
  out.replaceChildren();
  if (result.boolean !== undefined) {
    out.append(el("p", { class: "mono", text: `ASK → ${result.boolean}` }));
  } else if (!data.length) {
    out.append(el("p", { class: "muted", text: `No rows. (${ms.toFixed(0)} ms)` }));
  } else {
    const vars = result.variables ?? Object.keys(data[0]);
    const table = el("table");
    table.append(el("thead", {}, el("tr", {}, ...vars.map((v) => el("th", { text: v })))));
    const tbody = el("tbody");
    for (const row of data.slice(0, 200)) {
      tbody.append(el("tr", {}, ...vars.map((v) =>
        el("td", { class: "mono", text: short(cell(row[v])) }))));
    }
    table.append(tbody);
    out.append(
      el("p", { class: "muted", text:
        `${fmt(data.length)} row${data.length === 1 ? "" : "s"} in ${ms.toFixed(0)} ms`
        + (data.length > 200 ? " (showing the first 200)" : "")
        + (result.truncated ? " — the engine truncated this result" : "") }),
      table,
    );
  }
}

// ------------------------------------------------------------------- boot

async function loadPack(bytes, source) {
  status(`Verifying and importing ${fmt(bytes.byteLength)} bytes from ${source}…`);
  const t0 = performance.now();
  const report = await ask({ cmd: "load", bytes, source });
  const load = performance.now() - t0;
  renderProvenance(report, source, { load });
  status(`Loaded ${fmt(report.import.triples.accepted)} triples — everything below is a live `
    + `query against this tab's copy.`);
  $("main").hidden = false;
  await renderTypes();
  await renderBrowser();
  reportReleaseFreshness(report.manifest.producer.version);
}

async function boot() {
  try {
    window.__quipuBuild = JSON.parse(await ask({ cmd: "version" }));
  } catch {
    fail("This page needs WebAssembly and module workers. Try a current Chrome, Firefox or Safari.");
    return;
  }
  $("#sparql").value = CANNED[0].sparql;
  $("#run").addEventListener("click", runSparql);
  const canned = $("#canned");
  for (const c of CANNED) {
    canned.append(el("button", {
      class: "canned", text: c.name,
      onclick: () => { $("#sparql").value = c.sparql; runSparql(); },
    }));
  }
  $("#file-input").addEventListener("change", async (e) => {
    const file = e.target.files?.[0];
    if (!file) return;
    await loadPack(await file.arrayBuffer(), file.name).catch((err) => fail(String(err.message)));
  });
  $("#url-load").addEventListener("click", async () => {
    const url = $("#url-input").value.trim();
    if (!url) return;
    status(`Fetching ${url}…`);
    try {
      const r = await fetch(url);
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      await loadPack(await r.arrayBuffer(), url);
    } catch (err) {
      fail(`Could not load ${url}: ${err.message}. If this is a GitHub release asset, the `
        + `browser refuses it — those are served without CORS headers.`);
    }
  });

  // The pack this site ships. Staged into the book by the docs workflow from
  // the newest release, so it is a real release artifact and not a fixture.
  status("Fetching this repository's knowledge pack…");
  let bytes;
  try {
    const r = await fetch("./repository.qpack.tar.gz");
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    bytes = await r.arrayBuffer();
  } catch (err) {
    fail("The repository pack is not staged on this site yet — it is attached by the docs build "
      + "from the newest GitHub release. You can still load a .qpack.tar.gz from your own disk "
      + "with the file picker above.");
    return;
  }
  await loadPack(bytes, "this site's copy of the latest release pack")
    .catch((err) => fail(String(err.message)));
}

boot();
