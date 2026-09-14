// Read the committed demo share through the same Quipu worker as /explore/.
// The worker verifies its manifest, adopts its shapes, imports and promotes it.
// Only the renderer's node indices and degree counts are assembled here; the
// facts, labels, types and enrichment all come from Quipu's SPARQL engine.
export async function loadDemo() {
  const worker = new Worker(new URL('../explore/worker.js', import.meta.url), { type: 'module' });
  const pending = new Map();
  let nextId = 0;
  worker.onmessage = ({ data: { id, ok, result, error } }) => {
    const request = pending.get(id);
    if (!request) return;
    pending.delete(id);
    ok ? request.resolve(result) : request.reject(new Error(error));
  };
  worker.onerror = (event) => {
    for (const request of pending.values()) request.reject(new Error(event.message || 'Quipu worker failed'));
    pending.clear();
  };
  const ask = (msg) => new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    worker.postMessage({ id, ...msg }, msg.bytes ? [msg.bytes] : []);
  });
  try {
    const source = new URL('./demo.qpack.tar.gz', import.meta.url).href;
    const response = await fetch(source);
    if (!response.ok) throw new Error(`Demo pack: HTTP ${response.status}`);
    const report = await ask({ cmd: 'load', source, bytes: await response.arrayBuffer() });
    if (!report.promotion) throw new Error('Quipu refused to promote the demo pack');
    const result = await ask({ cmd: 'query', sparql: 'SELECT ?s ?p ?o WHERE { ?s ?p ?o }' });
    if (!Array.isArray(result.rows) || result.truncated) throw new Error('Incomplete demo query');
    return { ...projectDemo(result.rows), report };
  } finally {
    worker.terminate();
  }
}

// A rendering adapter, not a stored projection: every load queries the pack.
export function projectDemo(rows) {
  const type = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type';
  const label = 'http://www.w3.org/2000/01/rdf-schema#label';
  const short = (iri) => iri.split(/[/#]/).pop();
  const byIri = new Map();
  for (const { s, p, o } of rows) {
    if (p !== type && p !== label) continue;
    if (!byIri.has(s)) byIri.set(s, { iri: s, deg: 0 });
    byIri.get(s)[p === type ? 'type' : 'label'] = o;
  }
  const nodes = [...byIri.values()].filter(n => n.type && n.label)
    .sort((a, b) => a.iri.localeCompare(b.iri));
  const index = new Map(nodes.map((node, i) => [node.iri, i]));
  const edges = [], enrichment = {}, counts = new Map();
  for (const node of nodes) counts.set(node.type, (counts.get(node.type) || 0) + 1);
  for (const { s, p, o } of rows) {
    if (!index.has(s) || p === type || p === label) continue;
    if (index.has(o)) {
      const a = index.get(s), b = index.get(o);
      edges.push([a, b, short(p)]);
      nodes[a].deg++;
      nodes[b].deg++;
    } else {
      (enrichment[s] ??= {})[short(p)] = o;
    }
  }
  const types = [...counts].map(([iri, count]) => ({ iri, count }))
    .sort((a, b) => b.count - a.count || a.iri.localeCompare(b.iri));
  return { graph: { nodes, edges, types, stats: { nodes: nodes.length, edges: edges.length } }, enrichment };
}
