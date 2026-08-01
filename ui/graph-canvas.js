/**
 * Quipu graph canvas — node-link rendering for the explorer.
 *
 * Replaces Cytoscape + its `cose` layout. Two things made the old view lag:
 * `cose` is an O(n^2) synchronous force layout that blocks the main thread
 * until it settles, and the whole instance was destroyed and rebuilt on every
 * filter change. Here the layout is Barnes-Hut (O(n log n)) and runs one tick
 * per animation frame, so the graph is interactive from the first frame and
 * the browser never stalls.
 *
 * Data comes from POST /graph as a single payload; nothing is derived here.
 *
 * COLOUR: node type is drawn with a validated eight-slot categorical palette,
 * assigned by type PREVALENCE and never cycled — a ninth type folds into a
 * neutral "Other" slot rather than repeating a hue. Because every node shares
 * one canvas, all colour pairs are adjacent, and at that setting only three
 * hues clear the colour-vision separation floor. So colour is NEVER the only
 * channel: each slot also carries a distinct SHAPE, and the legend shows both.
 */

// Dark-surface categorical steps, validated against the #16213e chart surface
// (lightness band, chroma floor, CVD separation, normal-vision floor, and 3:1
// contrast all pass as an ordered set). Do not reorder or hand-tweak: the set
// was validated as a whole.
const SLOT_COLORS = [
  '#3987e5', // blue
  '#d95926', // orange
  '#199e70', // aqua
  '#c98500', // yellow
  '#d55181', // magenta
  '#008300', // green
  '#9085e9', // violet
  '#e66767', // red
];
// The secondary channel. Identity = (colour, shape), so the graph stays
// readable for a colour-blind reader and in greyscale print.
const SLOT_SHAPES = [
  'circle', 'square', 'diamond', 'triangle',
  'hexagon', 'triangle-down', 'plus', 'ring',
];
const OTHER_COLOR = '#8892a4';
const OTHER_SHAPE = 'circle';

const SURFACE = '#16213e';
const EDGE_COLOR = 'rgba(150, 161, 184, 0.38)';
const EDGE_COLOR_HL = '#e0e0e0';
const TEXT = '#e0e0e0';
const TEXT_MUTED = '#8892a4';

export const GRAPH_SLOT_COLORS = SLOT_COLORS;
export const GRAPH_SLOT_SHAPES = SLOT_SHAPES;
export const GRAPH_OTHER_COLOR = OTHER_COLOR;

export class GraphCanvas {
  constructor(canvas, opts = {}) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');
    this.onSelect = opts.onSelect || (() => {});
    this.nodes = [];
    this.edges = [];
    this.slotOf = new Map();     // type IRI -> palette slot (or -1 = Other)
    this.view = { x: 0, y: 0, k: 1 };
    this.hover = null;
    this.selected = null;
    this.drag = null;
    this.alpha = 0;
    this._raf = null;
    this._adj = new Map();

    this._bindEvents();
    this._resizeObserver = new ResizeObserver(() => this.resize());
    this._resizeObserver.observe(canvas.parentElement || canvas);
  }

  /** Load a /graph payload. Positions seed on a phyllotaxis spiral so the
   *  first frame is already spread out rather than a single central blob. */
  setData(payload, slotOf) {
    // Slot assignment is supplied by the caller from a STABLE GLOBAL type
    // ranking, never derived from this payload: deriving it here would mean a
    // type filter (which changes the census) repaints the surviving types, and
    // colour must follow the entity, not its rank within the current view.
    this.slotOf = slotOf || new Map();
    this.types = payload.types || [];

    const n = (payload.nodes || []).length;
    const GOLDEN = Math.PI * (3 - Math.sqrt(5));
    this.nodes = (payload.nodes || []).map((d, i) => {
      const r = 18 * Math.sqrt(i + 0.5);
      const a = i * GOLDEN;
      return {
        ...d,
        x: Math.cos(a) * r, y: Math.sin(a) * r,
        vx: 0, vy: 0,
        // Degree drives radius: hubs should be findable at a glance. sqrt so a
        // degree-100 hub is ~3x a leaf, not 100x.
        r: 4 + Math.min(9, Math.sqrt(d.deg || 0) * 1.9),
        slot: this.slotOf.has(d.type) ? this.slotOf.get(d.type) : -1,
      };
    });
    this.edges = (payload.edges || []).map(([s, t, p]) => ({ s, t, p }));

    this._adj = new Map();
    for (const e of this.edges) {
      if (!this._adj.has(e.s)) this._adj.set(e.s, new Set());
      if (!this._adj.has(e.t)) this._adj.set(e.t, new Set());
      this._adj.get(e.s).add(e.t);
      this._adj.get(e.t).add(e.s);
    }

    this.resize();
    this._fit(n);
    this._fitted = false;
    this._userMoved = false;
    this.alpha = 1;
    this._start();
  }

  /** Palette entries actually on screen, for the legend. */
  legend() {
    return (this.types || []).map((t) => {
      const slot = this.slotOf.has(t.iri) ? this.slotOf.get(t.iri) : -1;
      return {
        label: t.label, count: t.count,
        color: slot >= 0 ? SLOT_COLORS[slot] : OTHER_COLOR,
        shape: slot >= 0 ? SLOT_SHAPES[slot] : OTHER_SHAPE,
        folded: slot < 0,
      };
    }).sort((a, b) => Number(a.folded) - Number(b.folded) || b.count - a.count);
  }

  resize() {
    const dpr = window.devicePixelRatio || 1;
    const rect = this.canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    this.canvas.width = Math.round(rect.width * dpr);
    this.canvas.height = Math.round(rect.height * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.w = rect.width;
    this.h = rect.height;
    this._draw();
  }

  _fit(n) {
    // Zoom so the seeded spiral fills the viewport before the first tick.
    const spread = 18 * Math.sqrt(Math.max(n, 1)) + 60;
    const k = Math.min(this.w || 800, this.h || 600) / (spread * 2.2);
    this.view = { x: (this.w || 800) / 2, y: (this.h || 600) / 2, k: Math.max(0.15, Math.min(2, k)) };
  }

  /** Frame the settled graph: centre its bounding box and zoom to fill. */
  fitToContent(pad = 60) {
    if (!this.nodes.length || !this.w) return;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const n of this.nodes) {
      minX = Math.min(minX, n.x - n.r); maxX = Math.max(maxX, n.x + n.r);
      minY = Math.min(minY, n.y - n.r); maxY = Math.max(maxY, n.y + n.r);
    }
    const bw = Math.max(maxX - minX, 1), bh = Math.max(maxY - minY, 1);
    const k = Math.max(0.08, Math.min(2.5, Math.min((this.w - pad * 2) / bw, (this.h - pad * 2) / bh)));
    this.view.k = k;
    this.view.x = this.w / 2 - ((minX + maxX) / 2) * k;
    this.view.y = this.h / 2 - ((minY + maxY) / 2) * k;
    this._draw();
  }

  /** Re-run the layout without reloading data (e.g. after a filter). */
  reheat(a = 0.7) { this.alpha = a; this._start(); }

  destroy() {
    if (this._raf) cancelAnimationFrame(this._raf);
    this._resizeObserver.disconnect();
  }

  // ── Simulation ────────────────────────────────────────────────
  _start() {
    if (this._raf) return;
    const step = () => {
      const settled = this.alpha < 0.005;
      if (!settled) this._tick();
      // The seeded fit is a guess; once the forces settle, frame what actually
      // exists. Skipped after any manual pan/zoom — never yank the user's view.
      if (settled && !this._fitted && !this._userMoved) { this._fitted = true; this.fitToContent(); }
      this._draw();
      // Keep the frame loop alive only while there is something to animate:
      // a settled graph costs nothing until the user interacts.
      if (settled && !this.drag) { this._raf = null; return; }
      this._raf = requestAnimationFrame(step);
    };
    this._raf = requestAnimationFrame(step);
  }

  _tick() {
    const nodes = this.nodes;
    const n = nodes.length;
    if (!n) { this.alpha = 0; return; }
    this.alpha += (0 - this.alpha) * 0.022;   // decay toward rest

    // Barnes-Hut repulsion. The quadtree is what makes this O(n log n): a
    // distant cluster is approximated by its centre of mass instead of being
    // visited node by node, which is the whole difference from cose.
    const tree = buildQuadtree(nodes);
    const theta2 = 0.81;                       // (0.9)^2
    // Scaled by node count so a small graph doesn't fly apart and a big one
    // still separates instead of balling up in the middle.
    const repel = -(900 + 8 * Math.min(n, 400)) * this.alpha;
    for (const node of nodes) {
      applyRepulsion(tree, node, theta2, repel);
    }

    // Spring attraction along edges.
    const k = 0.09 * this.alpha;
    for (const e of this.edges) {
      const a = nodes[e.s], b = nodes[e.t];
      if (!a || !b) continue;
      let dx = b.x - a.x, dy = b.y - a.y;
      let d = Math.hypot(dx, dy) || 1;
      const ideal = 74 + a.r + b.r;
      const f = (d - ideal) * k;
      dx = (dx / d) * f; dy = (dy / d) * f;
      a.vx += dx; a.vy += dy;
      b.vx -= dx; b.vy -= dy;
    }

    // Weak centring so disconnected components don't drift off-canvas. Nodes
    // with no relations feel it more strongly: they carry no structure, so
    // letting repulsion fling them to the margins would spend the whole frame
    // on them and squeeze the connected core into the middle.
    const c = 0.008 * this.alpha;
    for (const node of nodes) {
      const pull = node.deg ? c : c * 6;
      node.vx -= node.x * pull;
      node.vy -= node.y * pull;
      if (node.fixed) { node.vx = 0; node.vy = 0; continue; }
      node.x += (node.vx *= 0.82);
      node.y += (node.vy *= 0.82);
    }

    this._separate(nodes);
  }

  /** Push overlapping nodes apart.
   *
   * Repulsion alone does not prevent overlap — it falls off with distance and
   * an edge pulling two hubs together beats it at close range, so nodes end up
   * piled on their neighbours and the view reads as a smear rather than a
   * graph. This is a hard positional constraint applied after integration.
   *
   * Uses a uniform grid rather than the quadtree: the interaction radius is
   * bounded by the largest node, so each node only needs its own cell and the
   * eight around it — O(n) with a small constant.
   */
  _separate(nodes) {
    let maxR = 0;
    for (const n of nodes) if (n.r > maxR) maxR = n.r;
    const pad = 5;
    const cell = (maxR + pad) * 2;
    const grid = new Map();
    const key = (cx, cy) => cx + ',' + cy;
    for (let i = 0; i < nodes.length; i++) {
      const n = nodes[i];
      const k = key(Math.floor(n.x / cell), Math.floor(n.y / cell));
      const bucket = grid.get(k);
      if (bucket) bucket.push(i); else grid.set(k, [i]);
    }
    for (let i = 0; i < nodes.length; i++) {
      const a = nodes[i];
      const cx = Math.floor(a.x / cell), cy = Math.floor(a.y / cell);
      for (let gx = cx - 1; gx <= cx + 1; gx++) {
        for (let gy = cy - 1; gy <= cy + 1; gy++) {
          const bucket = grid.get(key(gx, gy));
          if (!bucket) continue;
          for (const j of bucket) {
            if (j <= i) continue;           // each pair once
            const b = nodes[j];
            const min = a.r + b.r + pad;
            let dx = b.x - a.x, dy = b.y - a.y;
            let d2 = dx * dx + dy * dy;
            if (d2 >= min * min) continue;
            let d = Math.sqrt(d2);
            if (d < 1e-6) {                 // exactly coincident: nudge apart
              dx = (i % 2 ? 1 : -1) * 0.5; dy = 0.5; d = Math.hypot(dx, dy);
            }
            const push = (min - d) / d / 2;
            const ox = dx * push, oy = dy * push;
            if (!a.fixed) { a.x -= ox; a.y -= oy; }
            if (!b.fixed) { b.x += ox; b.y += oy; }
          }
        }
      }
    }
  }

  // ── Rendering ─────────────────────────────────────────────────
  _draw() {
    const ctx = this.ctx;
    if (!ctx || !this.w) return;
    ctx.save();
    ctx.fillStyle = SURFACE;
    ctx.fillRect(0, 0, this.w, this.h);
    ctx.translate(this.view.x, this.view.y);
    ctx.scale(this.view.k, this.view.k);

    const focus = this.hover !== null ? this.hover : this.selected;
    const near = focus !== null ? this._adj.get(focus) : null;

    // Edges first, so nodes sit on top of their own connections.
    ctx.lineWidth = 1 / this.view.k;
    ctx.strokeStyle = EDGE_COLOR;
    ctx.beginPath();
    for (const e of this.edges) {
      if (focus !== null && (e.s === focus || e.t === focus)) continue;
      const a = this.nodes[e.s], b = this.nodes[e.t];
      if (!a || !b) continue;
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
    }
    ctx.stroke();

    // Focused edges drawn separately and brighter — the hover affordance.
    if (focus !== null) {
      ctx.strokeStyle = EDGE_COLOR_HL;
      ctx.lineWidth = 1.6 / this.view.k;
      ctx.beginPath();
      for (const e of this.edges) {
        if (e.s !== focus && e.t !== focus) continue;
        const a = this.nodes[e.s], b = this.nodes[e.t];
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
      }
      ctx.stroke();
    }

    // Nodes. A 2px surface-coloured ring separates overlapping marks so a
    // dense cluster reads as distinct nodes rather than one blob.
    for (let i = 0; i < this.nodes.length; i++) {
      const nd = this.nodes[i];
      const dim = focus !== null && i !== focus && !(near && near.has(i));
      // Isolated nodes recede so the connected structure reads first.
      ctx.globalAlpha = dim ? 0.22 : (nd.deg ? 1 : 0.5);
      ctx.fillStyle = nd.slot >= 0 ? SLOT_COLORS[nd.slot] : OTHER_COLOR;
      ctx.strokeStyle = SURFACE;
      ctx.lineWidth = 2 / this.view.k;
      drawShape(ctx, nd.slot >= 0 ? SLOT_SHAPES[nd.slot] : OTHER_SHAPE, nd.x, nd.y, nd.r);
      if (i === this.selected) {
        ctx.globalAlpha = 1;
        ctx.strokeStyle = '#e94560';
        ctx.lineWidth = 2.5 / this.view.k;
        ctx.beginPath();
        ctx.arc(nd.x, nd.y, nd.r + 3.5, 0, Math.PI * 2);
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;

    // Labels are SELECTIVE — a name on every node is unreadable soup. Hubs get
    // one; so does whatever is focused and its neighbours.
    const fs = 11 / this.view.k;
    ctx.font = `${fs}px -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    const hubCut = this._hubCutoff();
    // Draw order: focused labels first so they win any collision, then hubs by
    // descending degree. A label that would overlap one already placed is
    // DROPPED — overlapping text is less readable than no text.
    const cands = [];
    for (let i = 0; i < this.nodes.length; i++) {
      const nd = this.nodes[i];
      const focused = focus !== null && (i === focus || (near && near.has(i)));
      if (!focused && !(this.view.k > 0.45 && nd.deg >= hubCut)) continue;
      cands.push({ nd, focused });
    }
    cands.sort((a, b) => Number(b.focused) - Number(a.focused) || (b.nd.deg || 0) - (a.nd.deg || 0));

    const placed = [];
    const pad = 2 / this.view.k;
    for (const { nd, focused } of cands) {
      const label = nd.label.length > 22 ? nd.label.slice(0, 21) + '…' : nd.label;
      const w = ctx.measureText(label).width;
      const x = nd.x - w / 2, y = nd.y + nd.r + 3 / this.view.k;
      const box = { x0: x - pad, y0: y - pad, x1: x + w + pad, y1: y + fs + pad };
      if (placed.some(b => !(box.x1 < b.x0 || box.x0 > b.x1 || box.y1 < b.y0 || box.y0 > b.y1))) continue;
      placed.push(box);
      ctx.lineWidth = 3 / this.view.k;
      ctx.strokeStyle = SURFACE;          // halo keeps text legible over edges
      ctx.strokeText(label, nd.x, y);
      ctx.fillStyle = focused ? TEXT : TEXT_MUTED;
      ctx.fillText(label, nd.x, y);
    }
    ctx.restore();
  }

  /** Degree at or above which a node is labelled unfocused. Adapts so a small
   *  graph labels everything and a dense one stays readable. */
  _hubCutoff() {
    if (this.nodes.length <= 40) return 0;
    const degs = this.nodes.map(n => n.deg || 0).sort((a, b) => b - a);
    return degs[Math.min(degs.length - 1, 24)] || 1;
  }

  // ── Interaction ───────────────────────────────────────────────
  _toWorld(px, py) {
    return { x: (px - this.view.x) / this.view.k, y: (py - this.view.y) / this.view.k };
  }

  _hit(px, py) {
    const p = this._toWorld(px, py);
    let best = null, bestD = Infinity;
    for (let i = 0; i < this.nodes.length; i++) {
      const nd = this.nodes[i];
      const d = Math.hypot(nd.x - p.x, nd.y - p.y);
      // Hit target is bigger than the mark so small nodes stay clickable.
      if (d < Math.max(nd.r + 6, 10) && d < bestD) { best = i; bestD = d; }
    }
    return best;
  }

  _bindEvents() {
    const c = this.canvas;

    c.addEventListener('pointerdown', (ev) => {
      c.setPointerCapture(ev.pointerId);
      const hit = this._hit(ev.offsetX, ev.offsetY);
      if (hit !== null) {
        const nd = this.nodes[hit];
        this.drag = { node: hit, moved: false };
        nd.fixed = true;
      } else {
        this.drag = { pan: true, x: ev.offsetX, y: ev.offsetY, moved: false };
      }
      this._start();
    });

    c.addEventListener('pointermove', (ev) => {
      if (this.drag) {
        this.drag.moved = true;
        if (this.drag.pan) {
          this._userMoved = true;
          this.view.x += ev.offsetX - this.drag.x;
          this.view.y += ev.offsetY - this.drag.y;
          this.drag.x = ev.offsetX; this.drag.y = ev.offsetY;
        } else {
          const p = this._toWorld(ev.offsetX, ev.offsetY);
          const nd = this.nodes[this.drag.node];
          nd.x = p.x; nd.y = p.y; nd.vx = 0; nd.vy = 0;
          this.alpha = Math.max(this.alpha, 0.25);
        }
        this._start();
        return;
      }
      const hit = this._hit(ev.offsetX, ev.offsetY);
      if (hit !== this.hover) {
        this.hover = hit;
        c.style.cursor = hit === null ? 'grab' : 'pointer';
        this._emitHover(hit, ev);
        this._start();
      }
    });

    const end = () => {
      if (this.drag && !this.drag.pan) {
        const nd = this.nodes[this.drag.node];
        nd.fixed = false;
        // A click (no movement) selects; a drag just repositions.
        if (!this.drag.moved) {
          this.selected = this.drag.node;
          this.onSelect(nd);
        }
      }
      this.drag = null;
      this._start();
    };
    c.addEventListener('pointerup', end);
    c.addEventListener('pointercancel', end);
    c.addEventListener('pointerleave', () => {
      if (this.hover !== null) { this.hover = null; this._emitHover(null); this._start(); }
    });

    c.addEventListener('wheel', (ev) => {
      ev.preventDefault();
      this._userMoved = true;
      const factor = Math.exp(-ev.deltaY * 0.0015);
      const k = Math.max(0.08, Math.min(6, this.view.k * factor));
      // Zoom toward the cursor, not the origin.
      this.view.x = ev.offsetX - (ev.offsetX - this.view.x) * (k / this.view.k);
      this.view.y = ev.offsetY - (ev.offsetY - this.view.y) * (k / this.view.k);
      this.view.k = k;
      this._start();
    }, { passive: false });
  }

  _emitHover(idx, ev) {
    if (this.onHover) this.onHover(idx === null ? null : this.nodes[idx], ev);
  }

  /** Centre the view on a node by IRI (used by search / list selection). */
  focusIri(iri) {
    const i = this.nodes.findIndex(n => n.iri === iri);
    if (i < 0) return false;
    this.selected = i;
    const nd = this.nodes[i];
    this.view.x = this.w / 2 - nd.x * this.view.k;
    this.view.y = this.h / 2 - nd.y * this.view.k;
    this._start();
    return true;
  }
}

// ── Shapes ──────────────────────────────────────────────────────
function drawShape(ctx, shape, x, y, r) {
  ctx.beginPath();
  switch (shape) {
    case 'square':
      ctx.rect(x - r, y - r, r * 2, r * 2); break;
    case 'diamond':
      ctx.moveTo(x, y - r * 1.25); ctx.lineTo(x + r * 1.25, y);
      ctx.lineTo(x, y + r * 1.25); ctx.lineTo(x - r * 1.25, y); ctx.closePath(); break;
    case 'triangle':
      ctx.moveTo(x, y - r * 1.3); ctx.lineTo(x + r * 1.2, y + r * 0.9);
      ctx.lineTo(x - r * 1.2, y + r * 0.9); ctx.closePath(); break;
    case 'triangle-down':
      ctx.moveTo(x, y + r * 1.3); ctx.lineTo(x + r * 1.2, y - r * 0.9);
      ctx.lineTo(x - r * 1.2, y - r * 0.9); ctx.closePath(); break;
    case 'hexagon':
      for (let i = 0; i < 6; i++) {
        const a = Math.PI / 6 + i * Math.PI / 3;
        const px = x + Math.cos(a) * r * 1.15, py = y + Math.sin(a) * r * 1.15;
        i ? ctx.lineTo(px, py) : ctx.moveTo(px, py);
      }
      ctx.closePath(); break;
    case 'plus': {
      // ONE 12-point outline, not two overlapping rects: two rects in a single
      // path make stroke() draw their internal edges, so the mark reads as four
      // small squares instead of a cross.
      const t = r * 0.42, a = r * 1.25;
      ctx.moveTo(x - t, y - a); ctx.lineTo(x + t, y - a); ctx.lineTo(x + t, y - t);
      ctx.lineTo(x + a, y - t); ctx.lineTo(x + a, y + t); ctx.lineTo(x + t, y + t);
      ctx.lineTo(x + t, y + a); ctx.lineTo(x - t, y + a); ctx.lineTo(x - t, y + t);
      ctx.lineTo(x - a, y + t); ctx.lineTo(x - a, y - t); ctx.lineTo(x - t, y - t);
      ctx.closePath(); break;
    }
    case 'ring':
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.arc(x, y, r * 0.45, 0, Math.PI * 2, true); break;
    default:
      ctx.arc(x, y, r, 0, Math.PI * 2);
  }
  ctx.fill();
  ctx.stroke();
}

// ── Barnes-Hut quadtree ─────────────────────────────────────────
function buildQuadtree(nodes) {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const n of nodes) {
    if (n.x < minX) minX = n.x;
    if (n.y < minY) minY = n.y;
    if (n.x > maxX) maxX = n.x;
    if (n.y > maxY) maxY = n.y;
  }
  const size = Math.max(maxX - minX, maxY - minY, 1) * 1.05;
  const root = node(minX, minY, size);
  for (const n of nodes) insert(root, n, 0);
  return root;
}

function node(x, y, s) {
  return { x, y, s, cx: 0, cy: 0, m: 0, body: null, kids: null };
}

function insert(q, p, depth) {
  q.cx = (q.cx * q.m + p.x) / (q.m + 1);
  q.cy = (q.cy * q.m + p.y) / (q.m + 1);
  q.m += 1;
  if (!q.kids && !q.body) { q.body = p; return; }
  // Depth guard: coincident points would otherwise subdivide forever.
  if (depth > 24) return;
  if (!q.kids) {
    const existing = q.body;
    q.body = null;
    q.kids = subdivide(q);
    place(q, existing, depth);
  }
  place(q, p, depth);
}

function subdivide(q) {
  const h = q.s / 2;
  return [
    node(q.x, q.y, h), node(q.x + h, q.y, h),
    node(q.x, q.y + h, h), node(q.x + h, q.y + h, h),
  ];
}

function place(q, p, depth) {
  const h = q.s / 2;
  const i = (p.x >= q.x + h ? 1 : 0) + (p.y >= q.y + h ? 2 : 0);
  insert(q.kids[i], p, depth + 1);
}

function applyRepulsion(q, p, theta2, strength) {
  if (!q || q.m === 0) return;
  const dx = q.cx - p.x, dy = q.cy - p.y;
  let d2 = dx * dx + dy * dy;
  if (d2 < 0.01) d2 = 0.01;
  // Far enough away to treat this whole cell as one mass? That test is the
  // entire performance story.
  if (!q.kids || (q.s * q.s) / d2 < theta2) {
    if (q.body === p) return;
    const f = (strength * q.m) / (d2 * Math.sqrt(d2));
    p.vx += dx * f;
    p.vy += dy * f;
    return;
  }
  for (const k of q.kids) applyRepulsion(k, p, theta2, strength);
}
