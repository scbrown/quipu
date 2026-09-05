// Stable, topic-clustered graph. Geometry is computed once per graph load;
// interaction moves a camera, never a running force simulation.
export function createConstellation({ query, prefixes, short, inspect }) {
  const K = "https://quipu.dev/knowledge/";
  const $ = (id) => document.getElementById(id);
  const svg = $("graph"), layer = $("graph-targets");
  const NS = "http://www.w3.org/2000/svg";
  const groups = [
    { name: "Vision & book", color: "#f4c86a" },
    { name: "Decisions & rules", color: "#ac9bff" },
    { name: "Episodes", color: "#72d8ba" },
    { name: "Code", color: "#7cbbff" },
    { name: "Reading trail", color: "#f3a6c8" },
  ];
  let nodes = new Map(), edges = [], active = null, camera = { x: 0, y: 0, z: 1 };
  let width = 600, height = 460, generation = 0, overview = false;
  const shape = (tag, attrs, text) => {
    const e = document.createElementNS(NS, tag);
    Object.entries(attrs).forEach(([k, v]) => e.setAttribute(k, v));
    if (text) e.textContent = text;
    svg.append(e);
    return e;
  };
  const button = (title, action) => {
    const b = document.createElement("button");
    b.textContent = title; b.onclick = action;
    return b;
  };
  const groupOf = (type) => /Vision|Book$/.test(type) ? 0
    : /Decision|Directive/.test(type) ? 1 : /Episode/.test(type) ? 2
    : /Code/.test(type) ? 3 : 4;
  const pos = (n) => ({ x: (n.x - camera.x) * camera.z + width / 2,
    y: (n.y - camera.y) * camera.z + height / 2 });
  const arrange = () => {
    groups.forEach((g, index) => {
      const members = [...nodes.values()].filter((n) => n.group === index)
        .sort((a, b) => a.id.localeCompare(b.id));
      g.x = (index % 3) * 510; g.y = Math.floor(index / 3) * 680;
      g.w = 460; g.h = Math.max(170, Math.ceil(members.length / 3) * 100 + 80);
      members.forEach((n, i) => {
        n.x = g.x + 80 + (i % 3) * 150; n.y = g.y + 90 + Math.floor(i / 3) * 100;
      });
      g.count = members.length;
    });
  };
  const draw = () => {
    width = svg.clientWidth; height = svg.clientHeight;
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.replaceChildren(); layer.replaceChildren();
    if (overview) {
      const visible = groups.filter((g) => g.count);
      const points = visible.map((g, i) => ({g, x: width * (i % 2 ? .65 : .35), y: 34 + i * (height - 90) / Math.max(1, visible.length - 1)}));
      points.slice(1).forEach((p, i) => shape("line", {x1:points[i].x, y1:points[i].y, x2:p.x, y2:p.y, stroke:"#7186af", "stroke-opacity":.45}));
      for (const {g, x, y} of points) {
        shape("ellipse", {cx:x, cy:y, rx:48, ry:24, fill:g.color, "fill-opacity":.1, stroke:g.color, "stroke-opacity":.4});
        shape("circle", {cx:x, cy:y, r:7, fill:g.color});
        const b = button(g.name + " · " + g.count, () => {
          overview = false; camera = {x:g.x + g.w / 2, y:g.y + 150, z:.7}; draw();
        });
        b.className="cluster-target"; b.style.left=Math.max(0, Math.min(width - 185, x - 85)) + "px";
        b.style.top=y + 14 + "px"; b.style.color=g.color; layer.append(b);
      }
      $("graph-zoom-level").textContent="Map";
      return;
    }
    for (const g of groups.filter((g) => g.count)) {
      const p = pos(g);
      shape("rect", { x: p.x, y: p.y, width: g.w * camera.z, height: g.h * camera.z,
        rx: 22, fill: g.color, "fill-opacity": .045, stroke: g.color, "stroke-opacity": .3 });
      const b = button(`${g.name} · ${g.count}`, () => {
        camera = { x: g.x + g.w / 2, y: g.y + 170, z: Math.max(.65, Math.min(1, width / 490)) }; draw();
      });
      b.className = "cluster-target"; b.style.left = `${p.x + 10}px`; b.style.top = `${p.y}px`;
      b.style.color = g.color; layer.append(b);
    }
    const adjacent = new Set(edges.filter((e) => e.from === active || e.to === active)
      .flatMap((e) => [e.from, e.to]));
    for (const e of edges) {
      const a = nodes.get(e.from), b = nodes.get(e.to);
      if (!a || !b) continue;
      const p = pos(a), q = pos(b), selected = e.from === active || e.to === active;
      shape("line", { x1: p.x, y1: p.y, x2: q.x, y2: q.y,
        stroke: selected ? "#d7deef" : "#8a9cbd", "stroke-opacity": selected ? .7 : .13,
        "stroke-width": selected ? 1.5 : 1 });
    }
    for (const n of nodes.values()) {
      const p = pos(n), color = groups[n.group].color;
      const lit = n.id === active || adjacent.has(n.id);
      shape("circle", { cx: p.x, cy: p.y, r: n.id === active ? 9 : 5, fill: color,
        opacity: !active || lit ? 1 : .5 });
      // At overview scale the cluster controls are the targets. Zooming into a
      // cluster reveals non-overlapping 44px node buttons, independent of zoom.
      if (camera.z < .5 || p.x < 22 || p.x > width - 22 || p.y < 44 || p.y > height - 40) continue;
      const b = button("", () => open(n.id));
      b.className = "node-target"; b.setAttribute("aria-label", n.label);
      b.title = n.label; b.dataset.node = n.id;
      b.style.left = `${p.x - 22}px`; b.style.top = `${p.y - 22}px`;
      b.style.setProperty("--node-color", color);
      layer.append(b);
      shape("text", { x: p.x, y: p.y + 34, fill: color, "text-anchor": "middle", "font-size": 11 },
        n.label.length > 20 ? n.label.slice(0, 18) + "…" : n.label);
    }
    $("graph-count").textContent = `${nodes.size} nodes · ${edges.length} links`;
    $("graph-zoom-level").textContent = `${Math.round(camera.z * 100)}%`;
  };
  const addText = (parent, tag, value) => {
    const e = document.createElement(tag); e.textContent = value; parent.append(e); return e;
  };
  function open(id, center = false) {
    const n = nodes.get(id); if (!n) return;
    active = id; overview = false;
    if (center) camera = { x: n.x, y: n.y, z: Math.max(.7, camera.z) };
    const card = $("node-card"); card.replaceChildren();
    addText(card, "p", groups[n.group].name).className = "eyebrow";
    addText(card, "h3", n.label);
    if (n.description) addText(card, "p", n.description);
    if (n.excerpt) addText(card, "blockquote", n.excerpt);
    if (n.source && /^https:\/\/github\.com\/scbrown\/quipu\/blob\//.test(n.source)) {
      const a = addText(card, "a", "Read the source ↗"); a.href = n.source;
      a.target = "_blank"; a.rel = "noopener noreferrer";
    }
    const links = document.createElement("div"); links.className = "node-links";
    for (const e of edges.filter((e) => e.from === id || e.to === id)) {
      const target = nodes.get(e.from === id ? e.to : e.from);
      if (!target) continue;
      const rel = short(e.rel).split(/[\/#]/).pop();
      links.append(button(`${e.from === id ? "→" : "←"} ${rel}: ${target.label}`, () => open(target.id, true)));
    }
    card.append(links, button("Inspect facts & edit", () => inspect({ iri: n.id, path: n.label })));
    draw();
  }
  const zoom = (factor) => { overview = false; camera.z = Math.max(.18, Math.min(2.5, camera.z * factor)); draw(); };
  $("graph-zoom-in").onclick = () => zoom(1.3);
  $("graph-zoom-out").onclick = () => zoom(1 / 1.3);
  $("graph-home").onclick = () => {
    const first = nodes.get(K + "vision") ?? nodes.values().next().value;
    if (first) { camera.z = .8; open(first.id, true); }
  };
  $("graph-fit").onclick = () => { overview = true; draw(); };
  $("graph-search").oninput = () => {
    const text = $("graph-search").value.trim().toLowerCase();
    const results = $("graph-results"); results.replaceChildren();
    if (!text) return;
    const matches = [...nodes.values()].filter((n) => `${n.label} ${n.description ?? ""}`.toLowerCase().includes(text));
    for (const n of matches.slice(0, 12)) results.append(button(n.label, () => {
      open(n.id, true); results.replaceChildren();
    }));
    if (!matches.length) addText(results, "p", "No matching nodes in this view.");
  };
  // Pointer capture supports touch panning; two pointers zoom about their midpoint.
  const pointers = new Map(); let gesture = null, moved = false;
  const viewport = $("graph-viewport");
  const snapshot = () => {
    const p = [...pointers.values()];
    return { x: p.reduce((s, a) => s + a.x, 0) / p.length,
      y: p.reduce((s, a) => s + a.y, 0) / p.length,
      d: p.length > 1 ? Math.hypot(p[0].x - p[1].x, p[0].y - p[1].y) : 0 };
  };
  viewport.onpointerdown = (e) => {
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY }); gesture = snapshot(); moved = false;
    // Leave taps on buttons intact. Capture after a drag actually starts.
  };
  viewport.onpointermove = (e) => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const next = snapshot(), dx = next.x - gesture.x, dy = next.y - gesture.y;
    if (!moved && Math.hypot(dx, dy) < 4 && pointers.size === 1) return;
    moved = true; viewport.setPointerCapture(e.pointerId);
    camera.x -= dx / camera.z; camera.y -= dy / camera.z;
    if (next.d && gesture.d) camera.z = Math.max(.18, Math.min(2.5, camera.z * next.d / gesture.d));
    gesture = next; draw();
  };
  viewport.onpointerup = viewport.onpointercancel = (e) => {
    pointers.delete(e.pointerId); gesture = pointers.size ? snapshot() : null;
  };
  viewport.addEventListener("click", (e) => { if (moved) { e.preventDefault(); e.stopPropagation(); moved = false; } }, true);
  viewport.addEventListener("wheel", (e) => { e.preventDefault(); zoom(e.deltaY < 0 ? 1.12 : 1 / 1.12); }, { passive: false });
  viewport.onkeydown = (e) => {
    const moves = { ArrowLeft: [-70, 0], ArrowRight: [70, 0], ArrowUp: [0, -70], ArrowDown: [0, 70] };
    if (moves[e.key]) { e.preventDefault(); camera.x += moves[e.key][0] / camera.z;
      camera.y += moves[e.key][1] / camera.z; draw(); }
  };
  new ResizeObserver(draw).observe(svg);
  for (const g of groups) {
    const item = addText($("graph-legend"), "span", "● " + g.name); item.style.color = g.color;
  }
  return {
    async load() {
      const ticket = ++generation;
      const sparql = `${prefixes}
SELECT ?subject ?predicate ?object WHERE {
  ?s ?p ?o . FILTER(STRSTARTS(STR(?s), "${K}"))
  BIND(STR(?s) AS ?subject) BIND(STR(?p) AS ?predicate) BIND(STR(?o) AS ?object)
} LIMIT 2000`;
      $("graph-showq").onclick = (e) => { e.preventDefault(); $("sparql").value = sparql; $("sparql").scrollIntoView(); };
      const result = await query(sparql);
      if (ticket !== generation) return;
      nodes = new Map(); edges = []; active = null;
      for (const row of result.rows ?? []) {
        const r = { s: row.subject, p: row.predicate, o: row.object };
        if (!nodes.has(r.s)) nodes.set(r.s, { id: r.s, label: short(r.s).split("/").pop(), group: 4 });
        const n = nodes.get(r.s);
        if (r.p.endsWith("#type") || r.p === "rdf:type") n.group = groupOf(r.o);
        else if (r.p.endsWith("#label") || r.p === "rdfs:label") n.label = r.o;
        else if (r.p.endsWith("#comment") || r.p === "rdfs:comment") n.description = r.o;
        else if (r.p === K + "excerpt") n.excerpt = r.o;
        else if (r.p.endsWith("#wasDerivedFrom")) n.source = r.o;
        else if (r.p.startsWith(K) && /^https?:/.test(r.o)) edges.push({ from: r.s, to: r.o, rel: r.p });
      }
      // Join to the real indexed code/doc nodes, including their own type and
      // label. Missing targets never become fabricated code nodes in this view.
      const files = await query(`${prefixes} SELECT ?subject ?kind ?path WHERE {
        ?s rdf:type ?type ; aegis:filePath ?path .
        FILTER(?type = aegis:CodeModule || ?type = aegis:Document)
        BIND(STR(?s) AS ?subject) BIND(STR(?type) AS ?kind)
      } ORDER BY ?path LIMIT 2000`);
      if (ticket !== generation) return;
      const targets = new Set(edges.map((e) => e.to));
      const fallback = !nodes.size;
      for (const row of files.rows ?? []) {
        const r = { s: row.subject, type: row.kind, path: row.path };
        if (targets.has(r.s) || (fallback && nodes.size < 60)) nodes.set(r.s,
          { id: r.s, label: r.path, group: groupOf(r.type), source: `https://github.com/scbrown/quipu/blob/main/${r.path}` });
      }
      edges = edges.filter((e) => nodes.has(e.from) && nodes.has(e.to));
      arrange();
      $("graph-search").value = ""; $("graph-results").replaceChildren();
      $("node-card").textContent = fallback
        ? "This pack has no contributor stories. Browse its files, or load a newer repository pack." : "Select a star to follow its story.";
      $("graph-home").click(); draw();
    },
    focus(item) { if (nodes.has(item.iri)) open(item.iri, true); },
    hasVision() { return nodes.has(K + "vision"); },
  };
}
