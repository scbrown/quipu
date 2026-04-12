/**
 * Quipu Web Components — embeddable knowledge graph widgets.
 *
 * Load this script in any HTML page to register:
 *   <quipu-graph>    — Interactive graph explorer (Sigma.js)
 *   <quipu-entity>   — Entity detail card with edges + history
 *   <quipu-sparql>   — SPARQL workbench
 *   <quipu-timeline> — Episode timeline
 *   <quipu-schema>   — Schema browser
 *
 * All components require an `endpoint` attribute pointing to a Quipu server.
 * Communication with the host page uses postMessage.
 */

(() => {
"use strict";

// ── Shared styles ─────────────────────────────────────────────────

const THEME_CSS = `
:host {
  display: block;
  font-family: system-ui, -apple-system, sans-serif;
  color: #e0e0e0;
  background: #1a1a2e;
  border: 1px solid #2a2a4a;
  border-radius: 6px;
  overflow: hidden;
}
* { margin: 0; padding: 0; box-sizing: border-box; }
a { color: #8be9fd; text-decoration: none; }
a:hover { text-decoration: underline; }
.loading { padding: 16px; color: #8892a4; font-style: italic; }
.error { padding: 16px; color: #e94560; }
.header { padding: 8px 12px; background: #16213e; border-bottom: 1px solid #2a2a4a;
  font-size: 12px; color: #8892a4; display: flex; justify-content: space-between; align-items: center; }
.header .title { font-weight: 600; color: #e0e0e0; }
.popout { background: none; border: 1px solid #2a2a4a; color: #8be9fd; cursor: pointer;
  padding: 2px 8px; border-radius: 3px; font-size: 11px; }
.popout:hover { background: #2a2a4a; }
.badge { display: inline-block; background: #2d2d44; color: #8be9fd; font-size: 11px;
  padding: 2px 6px; border-radius: 3px; margin: 0 4px 4px 0; }
.content { padding: 12px; }
table { width: 100%; border-collapse: collapse; }
td { padding: 4px 8px 4px 0; font-size: 13px; vertical-align: top; }
td:first-child { color: #8892a4; white-space: nowrap; }
`;

// ── Helpers ────────────────────────────────────────────────────────

function shortName(iri) {
  if (!iri) return "";
  iri = iri.replace(/^<|>$/g, "");
  let pos = iri.lastIndexOf("#");
  if (pos >= 0) return iri.slice(pos + 1);
  pos = iri.lastIndexOf("/");
  if (pos >= 0) return iri.slice(pos + 1);
  return iri;
}

function escapeHtml(s) {
  return String(s).replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");
}

async function quipuFetch(endpoint, path, options = {}) {
  const url = `${endpoint.replace(/\/$/, "")}${path}`;
  const resp = await fetch(url, options);
  if (!resp.ok) throw new Error(`Quipu API error: ${resp.status}`);
  return resp.json();
}

async function quipuPost(endpoint, path, body) {
  return quipuFetch(endpoint, path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

// ── Base class ────────────────────────────────────────────────────

class QuipuElement extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    style.textContent = THEME_CSS + this.constructor.extraStyles;
    this.shadowRoot.appendChild(style);
  }

  get endpoint() { return this.getAttribute("endpoint") || window.location.origin; }

  connectedCallback() {
    this._render();
    window.addEventListener("message", this._onMessage.bind(this));
  }

  disconnectedCallback() {
    window.removeEventListener("message", this._onMessage.bind(this));
  }

  _onMessage(event) {
    if (event.data && event.data.target === this.tagName.toLowerCase()) {
      this.handleMessage(event.data);
    }
  }

  handleMessage(_data) { /* Override in subclasses */ }

  _postToHost(data) {
    window.parent.postMessage({ source: this.tagName.toLowerCase(), ...data }, "*");
  }

  _showLoading(msg = "Loading...") {
    const el = document.createElement("div");
    el.className = "loading";
    el.textContent = msg;
    this.shadowRoot.appendChild(el);
    return el;
  }

  _showError(msg) {
    const el = document.createElement("div");
    el.className = "error";
    el.textContent = msg;
    this.shadowRoot.appendChild(el);
  }

  _makeHeader(title, popoutUrl) {
    const header = document.createElement("div");
    header.className = "header";
    header.innerHTML = `<span class="title">${escapeHtml(title)}</span>`;
    if (popoutUrl) {
      const btn = document.createElement("button");
      btn.className = "popout";
      btn.textContent = "Pop out";
      btn.onclick = () => window.open(popoutUrl, "_blank");
      header.appendChild(btn);
    }
    return header;
  }

  _clearContent() {
    const style = this.shadowRoot.querySelector("style");
    while (this.shadowRoot.lastChild !== style) {
      this.shadowRoot.removeChild(this.shadowRoot.lastChild);
    }
  }

  async _render() { /* Override in subclasses */ }
}

// ── <quipu-graph> ─────────────────────────────────────────────────

class QuipuGraph extends QuipuElement {
  static get observedAttributes() { return ["endpoint", "query", "focus", "depth", "types"]; }
  static extraStyles = `
    .graph-container { width: 100%; height: 100%; min-height: 300px; position: relative; }
    .graph-container canvas { width: 100% !important; height: 100% !important; }
    .graph-stats { position: absolute; bottom: 8px; right: 8px; font-size: 11px; color: #8892a4;
      background: rgba(26,26,46,0.8); padding: 2px 6px; border-radius: 3px; }
  `;

  attributeChangedCallback() { if (this.isConnected) this._render(); }

  async _render() {
    this._clearContent();
    const height = this.getAttribute("height") || "400px";
    this.style.height = height;

    const header = this._makeHeader("Graph Explorer", `${this.endpoint}/`);
    this.shadowRoot.appendChild(header);

    const container = document.createElement("div");
    container.className = "graph-container";
    container.id = "quipu-graph-" + Math.random().toString(36).slice(2, 8);
    this.shadowRoot.appendChild(container);

    const loading = this._showLoading("Loading graph...");

    try {
      const query = this.getAttribute("query") || "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
      const data = await quipuPost(this.endpoint, "/query", { sparql: query });

      loading.remove();

      if (!data.rows || data.rows.length === 0) {
        this._showError("No data returned");
        return;
      }

      // Build node/edge data.
      const nodes = new Map();
      const edges = [];
      const typeFilter = this.getAttribute("types");
      const allowedTypes = typeFilter ? typeFilter.split(",").map(t => t.trim()) : null;

      for (const row of data.rows) {
        const s = this._extractValue(row, "s");
        const p = this._extractValue(row, "p");
        const o = this._extractValue(row, "o");
        if (!s || !p || !o) continue;

        if (this._isIri(s) && !nodes.has(s)) {
          nodes.set(s, { iri: s, label: shortName(s), type: "default" });
        }
        if (p.includes("type") || p.endsWith("#type")) {
          const node = nodes.get(s);
          if (node) node.type = shortName(o);
        }
        if (this._isIri(o)) {
          if (!nodes.has(o)) nodes.set(o, { iri: o, label: shortName(o), type: "default" });
          edges.push({ source: s, target: o, label: shortName(p) });
        }
      }

      // Filter by type if specified.
      if (allowedTypes) {
        for (const [iri, node] of nodes) {
          if (!allowedTypes.includes(node.type) && node.type !== "default") {
            nodes.delete(iri);
          }
        }
      }

      // Render as a simple force-directed layout table (Sigma.js loaded separately).
      // If Sigma.js is available, use it; otherwise fall back to a node list.
      if (window.graphology && window.Sigma) {
        this._renderSigma(container, nodes, edges);
      } else {
        this._renderFallback(container, nodes, edges);
      }

      const stats = document.createElement("div");
      stats.className = "graph-stats";
      stats.textContent = `${nodes.size} nodes / ${edges.length} edges`;
      container.appendChild(stats);
    } catch (e) {
      loading.remove();
      this._showError(e.message);
    }
  }

  _renderSigma(container, nodes, edges) {
    const Graph = window.graphology.default || window.graphology;
    const graph = new Graph();

    const typeColors = {
      ProxmoxNode: "#4cc9f0", LXCContainer: "#4ecca3", SystemdService: "#4ecca3",
      CrewMember: "#f7a072", Rig: "#f7a072", Person: "#f7a072", default: "#8892a4",
    };

    for (const [iri, node] of nodes) {
      graph.addNode(iri, {
        label: node.label,
        x: Math.random() * 100,
        y: Math.random() * 100,
        size: 8,
        color: typeColors[node.type] || typeColors.default,
      });
    }

    for (const edge of edges) {
      if (graph.hasNode(edge.source) && graph.hasNode(edge.target)) {
        try {
          graph.addEdge(edge.source, edge.target, {
            label: edge.label, size: 1, color: "#444",
          });
        } catch (_) { /* multi-edge */ }
      }
    }

    const renderer = new window.Sigma(graph, container, {
      renderLabels: true, labelColor: { color: "#e0e0e0" },
      labelRenderedSizeThreshold: 6,
      defaultEdgeColor: "#444", defaultNodeColor: "#8892a4",
    });

    renderer.on("clickNode", ({ node }) => {
      this._postToHost({ action: "nodeClick", iri: node });
      this.dispatchEvent(new CustomEvent("quipu-node-click", { detail: { iri: node } }));
    });

    // Focus node if specified.
    const focus = this.getAttribute("focus");
    if (focus && graph.hasNode(focus)) {
      const attrs = graph.getNodeAttributes(focus);
      renderer.getCamera().animate({ x: attrs.x, y: attrs.y, ratio: 0.3 }, { duration: 500 });
    }

    // Run layout if ForceAtlas2 available.
    if (window.ForceAtlas2 || window.forceAtlas2) {
      const FA2 = window.ForceAtlas2 || window.forceAtlas2;
      const layout = new FA2.default(graph, { settings: { gravity: 1, scalingRatio: 2 } });
      layout.start();
      setTimeout(() => layout.stop(), 3000);
    }
  }

  _renderFallback(container, nodes, edges) {
    const wrapper = document.createElement("div");
    wrapper.style.cssText = "padding: 12px; overflow: auto; height: 100%;";

    const list = document.createElement("div");
    for (const [iri, node] of nodes) {
      const nodeEdges = edges.filter(e => e.source === iri || e.target === iri);
      const el = document.createElement("div");
      el.style.cssText = "margin-bottom: 8px; padding: 6px; background: #16213e; border-radius: 4px; cursor: pointer;";
      el.innerHTML = `<span class="badge">${escapeHtml(node.type)}</span>
        <a href="${this.endpoint}/entity/${encodeURIComponent(iri)}">${escapeHtml(node.label)}</a>
        <span style="color:#8892a4;font-size:11px"> (${nodeEdges.length} edges)</span>`;
      el.onclick = () => {
        this._postToHost({ action: "nodeClick", iri });
        this.dispatchEvent(new CustomEvent("quipu-node-click", { detail: { iri } }));
      };
      list.appendChild(el);
    }
    wrapper.appendChild(list);
    container.appendChild(wrapper);
  }

  _extractValue(row, key) {
    const v = row[key];
    if (!v) return null;
    if (typeof v === "string") return v;
    if (v.value) return v.value;
    return null;
  }

  _isIri(val) {
    return val && (val.startsWith("http://") || val.startsWith("https://") || val.startsWith("urn:"));
  }

  handleMessage(data) {
    if (data.action === "query") {
      this.setAttribute("query", data.query);
    } else if (data.action === "focus") {
      this.setAttribute("focus", data.iri);
    }
  }
}

// ── <quipu-entity> ────────────────────────────────────────────────

class QuipuEntity extends QuipuElement {
  static get observedAttributes() { return ["endpoint", "iri", "show-edges", "show-history"]; }
  static extraStyles = `
    .entity-label { font-size: 18px; font-weight: 600; margin-bottom: 4px; }
    .prop-group { margin-top: 12px; }
    .prop-group-label { font-size: 12px; font-weight: 600; color: #8892a4;
      text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 4px;
      border-bottom: 1px solid #2a2a4a; padding-bottom: 2px; }
    .prop-value { padding: 2px 0; font-size: 13px; }
    .history-entry { padding: 4px 0; font-size: 12px; border-bottom: 1px solid #1a1a2e; }
    .history-op { display: inline-block; width: 14px; }
    .assert { color: #4ecca3; }
    .retract { color: #e94560; }
  `;

  attributeChangedCallback() { if (this.isConnected) this._render(); }

  async _render() {
    this._clearContent();
    const iri = this.getAttribute("iri");
    if (!iri) { this._showError("No 'iri' attribute specified"); return; }

    const showEdges = this.getAttribute("show-edges") !== "false";
    const showHistory = this.getAttribute("show-history") === "true";
    const label = shortName(iri);

    const header = this._makeHeader(label, `${this.endpoint}/entity/${encodeURIComponent(iri)}`);
    this.shadowRoot.appendChild(header);

    const content = document.createElement("div");
    content.className = "content";
    this.shadowRoot.appendChild(content);

    const loading = this._showLoading("Loading entity...");
    content.appendChild(loading);

    try {
      // Fetch entity facts.
      const sparql = `SELECT ?p ?o WHERE { <${iri}> ?p ?o }`;
      const data = await quipuPost(this.endpoint, "/query", { sparql });

      loading.remove();

      let entityType = "Thing";
      const groups = new Map();

      for (const row of (data.rows || [])) {
        const p = this._extractValue(row, "p");
        const o = this._extractValue(row, "o");
        if (!p || o === null || o === undefined) continue;

        const pShort = shortName(p);
        if (p.includes("type") || p.endsWith("#type")) {
          entityType = shortName(String(o));
          continue;
        }
        if (!groups.has(pShort)) groups.set(pShort, []);
        groups.get(pShort).push(o);
      }

      // Entity header.
      const labelEl = document.createElement("div");
      labelEl.className = "entity-label";
      labelEl.textContent = label;
      content.appendChild(labelEl);

      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = entityType;
      content.appendChild(badge);

      // Statement groups.
      if (showEdges) {
        for (const [pred, values] of groups) {
          const group = document.createElement("div");
          group.className = "prop-group";
          group.innerHTML = `<div class="prop-group-label">${escapeHtml(pred)}</div>`;
          for (const val of values) {
            const valEl = document.createElement("div");
            valEl.className = "prop-value";
            const strVal = typeof val === "object" ? JSON.stringify(val) : String(val);
            if (this._isIri(strVal)) {
              valEl.innerHTML = `<a href="${this.endpoint}/entity/${encodeURIComponent(strVal)}"
                onclick="event.preventDefault(); this.getRootNode().host.setAttribute('iri','${escapeHtml(strVal)}')"
                >${escapeHtml(shortName(strVal))}</a>`;
            } else {
              valEl.textContent = strVal;
            }
            group.appendChild(valEl);
          }
          content.appendChild(group);
        }
      }

      // History.
      if (showHistory) {
        try {
          const history = await quipuPost(this.endpoint, "/entity_history", { iri });
          if (history.history && history.history.length > 0) {
            const hGroup = document.createElement("div");
            hGroup.className = "prop-group";
            hGroup.innerHTML = `<div class="prop-group-label">History</div>`;
            for (const entry of history.history) {
              const el = document.createElement("div");
              el.className = "history-entry";
              const opClass = entry.op === "assert" ? "assert" : "retract";
              const opSymbol = entry.op === "assert" ? "+" : "-";
              el.innerHTML = `<span class="history-op ${opClass}">${opSymbol}</span>
                <span style="color:#8892a4">${escapeHtml(shortName(entry.predicate))}</span>
                ${escapeHtml(typeof entry.value === "object" ? JSON.stringify(entry.value) : String(entry.value))}
                <span style="color:#555;font-size:11px">${escapeHtml(entry.valid_from || "")}</span>`;
              hGroup.appendChild(el);
            }
            content.appendChild(hGroup);
          }
        } catch (_) { /* History endpoint may not exist */ }
      }

      // Emit JSON-LD.
      this._postToHost({ action: "entityLoaded", iri, type: entityType, label });
    } catch (e) {
      loading.remove();
      this._showError(e.message);
    }
  }

  _extractValue(row, key) {
    const v = row[key];
    if (!v) return null;
    if (typeof v === "string") return v;
    if (v && typeof v === "object" && v.value !== undefined) return v.value;
    return v;
  }

  _isIri(val) {
    return typeof val === "string" &&
      (val.startsWith("http://") || val.startsWith("https://") || val.startsWith("urn:"));
  }

  handleMessage(data) {
    if (data.action === "show" && data.iri) {
      this.setAttribute("iri", data.iri);
    }
  }
}

// ── <quipu-sparql> ────────────────────────────────────────────────

class QuipuSparql extends QuipuElement {
  static get observedAttributes() { return ["endpoint", "query"]; }
  static extraStyles = `
    .sparql-editor { width: 100%; min-height: 120px; background: #0d1117; color: #e0e0e0;
      border: none; padding: 8px; font-family: 'SF Mono', 'Cascadia Code', monospace;
      font-size: 13px; resize: vertical; border-bottom: 1px solid #2a2a4a; }
    .toolbar { padding: 6px 12px; background: #16213e; display: flex; gap: 8px; align-items: center; }
    .run-btn { background: #4ecca3; color: #1a1a2e; border: none; padding: 4px 12px;
      border-radius: 3px; cursor: pointer; font-size: 12px; font-weight: 600; }
    .run-btn:hover { background: #3dbb92; }
    .run-btn:disabled { opacity: 0.5; cursor: default; }
    .results { overflow: auto; max-height: 400px; }
    .results table { font-size: 12px; }
    .results th { text-align: left; padding: 6px 8px; background: #16213e; color: #8892a4;
      font-weight: 600; position: sticky; top: 0; }
    .results td { padding: 4px 8px; border-bottom: 1px solid #1a1a2e; }
    .result-count { font-size: 11px; color: #8892a4; padding: 4px 12px; }
  `;

  async _render() {
    this._clearContent();
    const height = this.getAttribute("height") || "500px";
    this.style.height = height;

    const header = this._makeHeader("SPARQL Workbench", `${this.endpoint}/sparql`);
    this.shadowRoot.appendChild(header);

    const defaultQuery = this.getAttribute("query") || "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 25";

    const editor = document.createElement("textarea");
    editor.className = "sparql-editor";
    editor.value = defaultQuery;
    editor.spellcheck = false;
    this.shadowRoot.appendChild(editor);

    const toolbar = document.createElement("div");
    toolbar.className = "toolbar";
    const runBtn = document.createElement("button");
    runBtn.className = "run-btn";
    runBtn.textContent = "Run Query";
    toolbar.appendChild(runBtn);
    const countEl = document.createElement("span");
    countEl.className = "result-count";
    toolbar.appendChild(countEl);
    this.shadowRoot.appendChild(toolbar);

    const results = document.createElement("div");
    results.className = "results";
    this.shadowRoot.appendChild(results);

    const executeQuery = async () => {
      const sparql = editor.value.trim();
      if (!sparql) return;
      runBtn.disabled = true;
      runBtn.textContent = "Running...";
      results.innerHTML = "";
      countEl.textContent = "";

      try {
        const data = await quipuPost(this.endpoint, "/query", { sparql });
        const rows = data.rows || [];
        countEl.textContent = `${rows.length} results`;

        if (rows.length === 0) {
          results.innerHTML = '<div style="padding:12px;color:#8892a4">No results</div>';
          return;
        }

        // Build table from columns.
        const cols = data.columns || Object.keys(rows[0] || {});
        const table = document.createElement("table");
        const thead = document.createElement("thead");
        const headerRow = document.createElement("tr");
        for (const col of cols) {
          const th = document.createElement("th");
          th.textContent = col;
          headerRow.appendChild(th);
        }
        thead.appendChild(headerRow);
        table.appendChild(thead);

        const tbody = document.createElement("tbody");
        for (const row of rows) {
          const tr = document.createElement("tr");
          for (const col of cols) {
            const td = document.createElement("td");
            let val = row[col];
            if (val && typeof val === "object") val = val.value || JSON.stringify(val);
            val = String(val || "");
            if (val.startsWith("http://") || val.startsWith("https://")) {
              td.innerHTML = `<a href="${this.endpoint}/entity/${encodeURIComponent(val)}"
                target="_blank">${escapeHtml(shortName(val))}</a>`;
            } else {
              td.textContent = val;
            }
            tr.appendChild(td);
          }
          tbody.appendChild(tr);
        }
        table.appendChild(tbody);
        results.appendChild(table);

        this._postToHost({ action: "queryComplete", sparql, rowCount: rows.length });
      } catch (e) {
        results.innerHTML = `<div class="error">${escapeHtml(e.message)}</div>`;
      } finally {
        runBtn.disabled = false;
        runBtn.textContent = "Run Query";
      }
    };

    runBtn.onclick = executeQuery;
    editor.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
        e.preventDefault();
        executeQuery();
      }
    });

    // Auto-execute if query was provided via attribute.
    if (this.getAttribute("query")) {
      executeQuery();
    }
  }

  handleMessage(data) {
    if (data.action === "query" && data.query) {
      this.setAttribute("query", data.query);
      this._render();
    }
  }
}

// ── <quipu-timeline> ──────────────────────────────────────────────

class QuipuTimeline extends QuipuElement {
  static get observedAttributes() { return ["endpoint", "from", "to", "source-type"]; }
  static extraStyles = `
    .filters { padding: 8px 12px; display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
    .filter-btn { background: #2d2d44; color: #e0e0e0; border: 1px solid #2a2a4a;
      padding: 3px 8px; border-radius: 3px; font-size: 11px; cursor: pointer; }
    .filter-btn.active { background: #0f3460; border-color: #8be9fd; color: #8be9fd; }
    .episode { padding: 10px 12px; border-bottom: 1px solid #1a1a2e; cursor: pointer; }
    .episode:hover { background: #16213e; }
    .episode-label { font-size: 14px; font-weight: 500; }
    .episode-meta { font-size: 11px; color: #8892a4; margin-top: 2px; }
    .episode-entities { padding: 8px 12px; background: #0d1117; font-size: 12px; display: none; }
    .episode.expanded .episode-entities { display: block; }
    .source-badge { font-size: 10px; padding: 1px 5px; border-radius: 2px; }
    .source-agent { background: #2d2d44; color: #f7a072; }
    .source-ci { background: #2d2d44; color: #4ecca3; }
    .source-incident { background: #2d2d44; color: #e94560; }
    .source-manual { background: #2d2d44; color: #8892a4; }
    .source-obs { background: #2d2d44; color: #8be9fd; }
  `;

  async _render() {
    this._clearContent();
    const header = this._makeHeader("Episode Timeline", `${this.endpoint}/timeline`);
    this.shadowRoot.appendChild(header);

    const loading = this._showLoading("Loading episodes...");

    try {
      const sparql = `SELECT ?ep ?label ?source ?comment WHERE {
        ?ep <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/prov#Activity> .
        ?ep <http://www.w3.org/2000/01/rdf-schema#label> ?label .
        OPTIONAL { ?ep <http://www.w3.org/ns/prov#wasAssociatedWith> ?source }
        OPTIONAL { ?ep <http://www.w3.org/2000/01/rdf-schema#comment> ?comment }
      } ORDER BY ?ep`;
      const data = await quipuPost(this.endpoint, "/query", { sparql });

      loading.remove();
      const rows = data.rows || [];

      if (rows.length === 0) {
        this._showError("No episodes found");
        return;
      }

      // Source type filters.
      const sourceFilter = this.getAttribute("source-type");
      const sourceTypes = new Set();
      const episodes = rows.map(row => {
        const label = this._extractValue(row, "label") || "";
        const source = this._extractValue(row, "source") || "";
        const sourceType = this._guessSourceType(source, label);
        sourceTypes.add(sourceType);
        return {
          iri: this._extractValue(row, "ep") || "",
          label,
          source: shortName(source),
          sourceType,
          comment: this._extractValue(row, "comment") || "",
        };
      }).filter(ep => !sourceFilter || ep.sourceType === sourceFilter);

      // Filter bar.
      const filters = document.createElement("div");
      filters.className = "filters";
      const allBtn = document.createElement("button");
      allBtn.className = "filter-btn active";
      allBtn.textContent = `All (${episodes.length})`;
      filters.appendChild(allBtn);
      this.shadowRoot.appendChild(filters);

      // Episode list.
      const list = document.createElement("div");
      for (const ep of episodes) {
        const el = document.createElement("div");
        el.className = "episode";
        el.innerHTML = `
          <div class="episode-label">
            <span class="source-badge source-${ep.sourceType}">${ep.sourceType}</span>
            ${escapeHtml(ep.label)}
          </div>
          <div class="episode-meta">${escapeHtml(ep.source)}${ep.comment ? " — " + escapeHtml(ep.comment) : ""}</div>
          <div class="episode-entities"></div>`;
        el.onclick = async () => {
          el.classList.toggle("expanded");
          const entitiesEl = el.querySelector(".episode-entities");
          if (el.classList.contains("expanded") && !entitiesEl.dataset.loaded) {
            try {
              const eSparql = `SELECT ?s ?p ?o WHERE {
                ?s <http://www.w3.org/ns/prov#wasGeneratedBy> <${ep.iri}> . ?s ?p ?o . }`;
              const eData = await quipuPost(this.endpoint, "/query", { sparql: eSparql });
              const eRows = eData.rows || [];
              if (eRows.length === 0) {
                entitiesEl.textContent = "No entities";
              } else {
                const entities = new Set();
                for (const r of eRows) {
                  const s = this._extractValue(r, "s");
                  if (s) entities.add(s);
                }
                entitiesEl.innerHTML = [...entities].map(e =>
                  `<a href="${this.endpoint}/entity/${encodeURIComponent(e)}">${escapeHtml(shortName(e))}</a>`
                ).join(", ");
              }
              entitiesEl.dataset.loaded = "true";
            } catch (_) {
              entitiesEl.textContent = "Failed to load";
            }
          }
          this._postToHost({ action: "episodeClick", iri: ep.iri });
        };
        list.appendChild(el);
      }
      this.shadowRoot.appendChild(list);
    } catch (e) {
      loading.remove();
      this._showError(e.message);
    }
  }

  _extractValue(row, key) {
    const v = row[key];
    if (!v) return null;
    if (typeof v === "string") return v;
    if (v && v.value !== undefined) return v.value;
    return null;
  }

  _guessSourceType(source, label) {
    const combined = (source + " " + label).toLowerCase();
    if (combined.includes("incident") || combined.includes("p0") || combined.includes("p1")) return "incident";
    if (combined.includes("agent") || combined.includes("crew") || combined.includes("handoff")) return "agent";
    if (combined.includes("ci") || combined.includes("pipeline") || combined.includes("deploy")) return "ci";
    if (combined.includes("patrol") || combined.includes("observation") || combined.includes("scan")) return "obs";
    return "manual";
  }

  handleMessage(data) {
    if (data.action === "filter" && data.sourceType) {
      this.setAttribute("source-type", data.sourceType);
      this._render();
    }
  }
}

// ── <quipu-schema> ────────────────────────────────────────────────

class QuipuSchema extends QuipuElement {
  static get observedAttributes() { return ["endpoint", "shape"]; }
  static extraStyles = `
    .type-tree { padding: 8px 12px; }
    .type-node { padding: 4px 0; }
    .type-indent { padding-left: 16px; }
    .type-name { cursor: pointer; }
    .type-name:hover { color: #8be9fd; }
    .type-count { color: #8892a4; font-size: 11px; margin-left: 4px; }
    .shape-card { margin: 8px 12px; padding: 10px; background: #16213e; border-radius: 4px;
      border: 1px solid #2a2a4a; }
    .shape-name { font-weight: 600; margin-bottom: 6px; }
    .shape-prop { font-size: 12px; padding: 2px 0; }
    .tabs { display: flex; border-bottom: 1px solid #2a2a4a; }
    .tab { padding: 6px 16px; cursor: pointer; font-size: 13px; color: #8892a4;
      border-bottom: 2px solid transparent; }
    .tab.active { color: #8be9fd; border-bottom-color: #8be9fd; }
  `;

  async _render() {
    this._clearContent();
    const header = this._makeHeader("Schema Browser", `${this.endpoint}/schema`);
    this.shadowRoot.appendChild(header);

    // Tabs.
    const tabs = document.createElement("div");
    tabs.className = "tabs";
    const typesTab = document.createElement("div");
    typesTab.className = "tab active";
    typesTab.textContent = "Types";
    const shapesTab = document.createElement("div");
    shapesTab.className = "tab";
    shapesTab.textContent = "Shapes";
    tabs.appendChild(typesTab);
    tabs.appendChild(shapesTab);
    this.shadowRoot.appendChild(tabs);

    const content = document.createElement("div");
    content.className = "content";
    this.shadowRoot.appendChild(content);

    typesTab.onclick = () => { this._renderTypes(content); typesTab.classList.add("active"); shapesTab.classList.remove("active"); };
    shapesTab.onclick = () => { this._renderShapes(content); shapesTab.classList.add("active"); typesTab.classList.remove("active"); };

    await this._renderTypes(content);
  }

  async _renderTypes(container) {
    container.innerHTML = '<div class="loading">Loading types...</div>';
    try {
      const sparql = `SELECT ?type (COUNT(?s) AS ?count) WHERE {
        ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type
      } GROUP BY ?type ORDER BY DESC(?count)`;
      const data = await quipuPost(this.endpoint, "/query", { sparql });

      container.innerHTML = "";
      const tree = document.createElement("div");
      tree.className = "type-tree";

      for (const row of (data.rows || [])) {
        const typeIri = this._extractValue(row, "type");
        const count = this._extractValue(row, "count");
        if (!typeIri) continue;

        const el = document.createElement("div");
        el.className = "type-node";
        el.innerHTML = `<span class="type-name">${escapeHtml(shortName(typeIri))}</span>
          <span class="type-count">(${count || 0})</span>`;
        el.querySelector(".type-name").onclick = () => {
          this._postToHost({ action: "typeClick", iri: typeIri });
          this.dispatchEvent(new CustomEvent("quipu-type-click", { detail: { iri: typeIri } }));
        };
        tree.appendChild(el);
      }
      container.appendChild(tree);
    } catch (e) {
      container.innerHTML = `<div class="error">${escapeHtml(e.message)}</div>`;
    }
  }

  async _renderShapes(container) {
    container.innerHTML = '<div class="loading">Loading shapes...</div>';
    try {
      const data = await quipuPost(this.endpoint, "/shapes", { action: "list" });
      container.innerHTML = "";

      const shapes = data.shapes || [];
      if (shapes.length === 0) {
        container.innerHTML = '<div style="padding:12px;color:#8892a4">No SHACL shapes loaded</div>';
        return;
      }

      for (const shape of shapes) {
        const card = document.createElement("div");
        card.className = "shape-card";
        card.innerHTML = `<div class="shape-name">${escapeHtml(shape.name || "")}</div>
          <div class="shape-prop" style="color:#8892a4">Loaded: ${escapeHtml(shape.loaded_at || "")}</div>`;
        container.appendChild(card);
      }
    } catch (e) {
      container.innerHTML = `<div class="error">${escapeHtml(e.message)}</div>`;
    }
  }

  _extractValue(row, key) {
    const v = row[key];
    if (!v) return null;
    if (typeof v === "string") return v;
    if (v && v.value !== undefined) return String(v.value);
    return null;
  }

  handleMessage(data) {
    if (data.action === "showShape" && data.shape) {
      this.setAttribute("shape", data.shape);
    }
  }
}

// ── Register all components ───────────────────────────────────────

if (!customElements.get("quipu-graph"))    customElements.define("quipu-graph",    QuipuGraph);
if (!customElements.get("quipu-entity"))   customElements.define("quipu-entity",   QuipuEntity);
if (!customElements.get("quipu-sparql"))   customElements.define("quipu-sparql",   QuipuSparql);
if (!customElements.get("quipu-timeline")) customElements.define("quipu-timeline", QuipuTimeline);
if (!customElements.get("quipu-schema"))   customElements.define("quipu-schema",   QuipuSchema);

// Export for module usage.
if (typeof window !== "undefined") {
  window.QuipuComponents = { QuipuGraph, QuipuEntity, QuipuSparql, QuipuTimeline, QuipuSchema };
}

})();
