/**
 * Quipu Datalinks — a 3D rank-layout view of a prerequisite DAG.
 *
 * The 2D explorer (ui/graph-canvas.js) draws a force layout: positions are
 * emergent, so they move when the data moves and mean nothing on their own.
 * Here position is DERIVED and STABLE — height is a node's longest-path depth
 * through the graph, so two loads of the same data agree exactly, and you can
 * learn where things live. That is the whole reason this view exists; a 3D
 * force layout would just be a hairball you can get lost inside.
 *
 * Built for the SMAC datalinks graph (NeuralAmplifier's alphax.ttl: 88
 * technologies joined by requiresTech, with facilities/units/projects hanging
 * off them), but nothing here is SMAC-specific. Any graph gets ranked; the
 * aiWeight "compass" and the effectText panel light up only when those
 * predicates are present, so `just demo` still renders.
 *
 * Data comes from POST /graph, exactly as the 2D view does. Enrichment (the
 * compass, prose, rule tier) is one extra SPARQL query, and is optional.
 *
 * COLOUR: reuses the validated eight-slot palette from graph-canvas.js —
 * assigned by type prevalence, never cycled, ninth type folds into a neutral
 * "Other". Identity is (colour, shape), so the view stays readable for a
 * colour-blind reader; in 3D the shape channel becomes geometry.
 */

import * as THREE from '/vendor/three.module.min.js';
import {
  GRAPH_SLOT_COLORS as SLOT_COLORS,
  GRAPH_OTHER_COLOR as OTHER_COLOR,
} from '/graph-canvas.js';

const SURFACE = 0x16213e;

// One geometry per palette slot, in the same order as GRAPH_SLOT_SHAPES:
// circle, square, diamond, triangle, hexagon, triangle-down, plus, ring.
function slotGeometries() {
  return [
    new THREE.SphereGeometry(1, 16, 12),
    new THREE.BoxGeometry(1.7, 1.7, 1.7),
    new THREE.OctahedronGeometry(1.3),
    new THREE.ConeGeometry(1.3, 2.2, 4),
    new THREE.CylinderGeometry(1.2, 1.2, 1.4, 6),
    new THREE.ConeGeometry(1.3, 2.2, 3).rotateZ(Math.PI),
    new THREE.BoxGeometry(2.2, 0.8, 0.8),
    new THREE.TorusGeometry(1.1, 0.4, 8, 16),
  ];
}

const RANK_HEIGHT = 13;
const RING_BASE = 16;
const RING_PER_NODE = 2.6;

/** The four SMAC AI weights, as compass bearings. */
const COMPASS = [
  ['aiWeightGrowth', 0],
  ['aiWeightTech', Math.PI / 2],
  ['aiWeightWealth', Math.PI],
  ['aiWeightPower', (3 * Math.PI) / 2],
];

export class Datalinks {
  constructor(container, opts = {}) {
    this.container = container;
    this.onSelect = opts.onSelect || (() => {});
    this.nodes = [];
    this.edges = [];
    this.slotOf = new Map();
    this.meshes = [];
    this.selected = null;
    this.hover = null;
    this._weights = new Map();  // iri -> {aiWeightGrowth: n, ...}
    this._raf = null;

    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color(SURFACE);
    // Range is set from the content bounds in frameAll(); a fixed range either
    // swallows a tall lattice or does nothing at all on a small one.
    this.scene.fog = new THREE.Fog(SURFACE, 1, 1e5);

    this.camera = new THREE.PerspectiveCamera(55, 1, 0.5, 2000);
    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    container.appendChild(this.renderer.domElement);

    this.scene.add(new THREE.AmbientLight(0xffffff, 0.85));
    const key = new THREE.DirectionalLight(0xffffff, 1.15);
    key.position.set(40, 90, 60);
    this.scene.add(key);

    // Spherical camera state: orbit around a target that rides up the lattice.
    this.orbit = { theta: 0.6, phi: 1.15, radius: 150, target: new THREE.Vector3() };
    this._raycaster = new THREE.Raycaster();
    this._pointer = new THREE.Vector2();

    this._bindEvents();
    this._resizeObserver = new ResizeObserver(() => this.resize());
    this._resizeObserver.observe(container);
    this.resize();
    this._loop();
  }

  /**
   * Load a POST /graph payload. Ranks, then places, then builds meshes.
   * Positions depend only on the payload, so this is idempotent.
   */
  setData(payload) {
    this.nodes = (payload.nodes || []).map((n, i) => ({ ...n, idx: i }));
    this.edges = payload.edges || [];
    this._assignSlots(payload.types || []);
    this._rank();
    this._place();
    this._build();
    this.frameAll();
    return this;
  }

  /**
   * Attach enrichment keyed by IRI: {iri: {aiWeightGrowth, effectText, ...}}.
   * Re-places nodes so the compass takes effect. Safe to never call.
   */
  setEnrichment(byIri) {
    this._weights = byIri instanceof Map ? byIri : new Map(Object.entries(byIri || {}));
    for (const node of this.nodes) node.extra = this._weights.get(node.iri) || null;
    this._place();
    this._build();
    return this;
  }

  /** Palette slot per type IRI, by prevalence. Ninth and beyond -> Other. */
  _assignSlots(types) {
    this.slotOf.clear();
    types.forEach((t, i) => {
      this.slotOf.set(t.iri, i < SLOT_COLORS.length ? i : -1);
    });
    this.types = types;
  }

  /**
   * Longest-path depth. An edge (a -> b) reads "a requires b", so b is the
   * prerequisite and sits below: rank(a) = 1 + max(rank(b)).
   *
   * Memoised DFS with an on-stack guard, because a general graph is not a DAG
   * and a cycle would otherwise recurse forever. A node on a cycle contributes
   * 0 rather than aborting the layout — a cyclic graph still renders, it just
   * renders flat where it is cyclic.
   */
  _rank() {
    const out = new Map();
    for (const [a, b] of this.edges) {
      if (!out.has(a)) out.set(a, []);
      out.get(a).push(b);
    }
    const rank = new Array(this.nodes.length).fill(-1);
    const onStack = new Uint8Array(this.nodes.length);

    const visit = (i) => {
      if (rank[i] >= 0) return rank[i];
      if (onStack[i]) return 0;
      onStack[i] = 1;
      let best = 0;
      for (const j of out.get(i) || []) {
        if (j >= 0 && j < this.nodes.length) best = Math.max(best, visit(j) + 1);
      }
      onStack[i] = 0;
      rank[i] = best;
      return best;
    };

    for (let i = 0; i < this.nodes.length; i++) visit(i);
    this.nodes.forEach((n, i) => { n.rank = rank[i]; });
    this.maxRank = Math.max(0, ...rank);
  }

  /**
   * Place each node on the ring for its rank. Bearing comes from the aiWeight
   * compass when present — the weighted circular mean of the four axes, so
   * wealth techs sit on one side and power techs on another and position
   * carries meaning. Without weights, nodes spread evenly. Ties break on IRI
   * so the layout is identical across loads.
   */
  _place() {
    const byRank = new Map();
    for (const node of this.nodes) {
      if (!byRank.has(node.rank)) byRank.set(node.rank, []);
      byRank.get(node.rank).push(node);
    }

    for (const [rank, group] of byRank) {
      group.sort((a, b) => (a.iri < b.iri ? -1 : a.iri > b.iri ? 1 : 0));
      for (const node of group) node.bearing = this._bearing(node);
      // Stable order around the ring: by bearing, then IRI.
      group.sort((a, b) => a.bearing - b.bearing || (a.iri < b.iri ? -1 : 1));

      const radius = RING_BASE + RING_PER_NODE * Math.sqrt(group.length) * 3;
      const step = (Math.PI * 2) / group.length;
      group.forEach((node, i) => {
        // Nudge toward the even slot so dense rings cannot overlap, while a
        // sparse ring still honours the compass bearing almost exactly.
        const even = i * step;
        const angle = group.length > 3 ? even : node.bearing;
        node.pos = new THREE.Vector3(
          Math.cos(angle) * radius,
          rank * RANK_HEIGHT,
          Math.sin(angle) * radius,
        );
      });
    }
  }

  /** Weighted circular mean of the four AI weights; -1 when unavailable. */
  _bearing(node) {
    const extra = node.extra;
    if (!extra) return this._hashAngle(node.iri);
    let x = 0;
    let y = 0;
    let any = false;
    for (const [key, angle] of COMPASS) {
      const w = Number(extra[key]);
      if (!Number.isFinite(w) || w <= 0) continue;
      any = true;
      x += Math.cos(angle) * w;
      y += Math.sin(angle) * w;
    }
    if (!any) return this._hashAngle(node.iri);
    const mean = Math.atan2(y, x);
    return mean < 0 ? mean + Math.PI * 2 : mean;
  }

  /** Deterministic angle from the IRI, so unweighted layouts are stable too. */
  _hashAngle(iri) {
    let h = 2166136261;
    for (let i = 0; i < iri.length; i++) {
      h ^= iri.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return ((h >>> 0) / 4294967296) * Math.PI * 2;
  }

  /** Build one InstancedMesh per palette slot, plus the edge lines. */
  _build() {
    for (const mesh of this.meshes) {
      this.scene.remove(mesh);
      mesh.geometry.dispose();
      mesh.material.dispose();
    }
    this.meshes = [];
    if (this.lines) {
      this.scene.remove(this.lines);
      this.lines.geometry.dispose();
      this.lines.material.dispose();
      this.lines = null;
    }

    const geoms = slotGeometries();
    const bySlot = new Map();
    for (const node of this.nodes) {
      const slot = this.slotOf.has(node.type) ? this.slotOf.get(node.type) : -1;
      node.slot = slot;
      if (!bySlot.has(slot)) bySlot.set(slot, []);
      bySlot.get(slot).push(node);
    }

    const dummy = new THREE.Object3D();
    for (const [slot, group] of bySlot) {
      const geometry = slot >= 0 ? geoms[slot] : new THREE.SphereGeometry(1, 12, 8);
      const base = new THREE.Color(slot >= 0 ? SLOT_COLORS[slot] : OTHER_COLOR);
      // A little self-illumination: the palette was validated against this
      // surface as flat fill, and pure Lambert shading drops the unlit faces
      // well below that contrast. Kept low on purpose — emissive is a material
      // property, so it does NOT respond to per-instance colour, and a high
      // value would floor every node at 40% of its hue and flatten the
      // importance re-light into mud.
      const material = new THREE.MeshLambertMaterial({
        emissive: base.clone().multiplyScalar(0.16),
      });
      const mesh = new THREE.InstancedMesh(geometry, material, group.length);
      mesh.userData.nodes = group;

      group.forEach((node, i) => {
        node.mesh = mesh;
        node.instance = i;
        node.baseColor = base;
        // Degree drives size: a hub is physically bigger.
        const scale = 0.85 + Math.min(1.6, Math.sqrt(node.deg || 0) * 0.42);
        dummy.position.copy(node.pos);
        dummy.scale.setScalar(scale);
        dummy.updateMatrix();
        mesh.setMatrixAt(i, dummy.matrix);
        mesh.setColorAt(i, base);
      });
      mesh.instanceMatrix.needsUpdate = true;
      if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
      this.scene.add(mesh);
      this.meshes.push(mesh);
    }
    // Unused slot geometries would leak; only the ones handed to a mesh live on.
    geoms.forEach((g, i) => { if (!bySlot.has(i)) g.dispose(); });

    const positions = [];
    for (const [a, b] of this.edges) {
      const from = this.nodes[a];
      const to = this.nodes[b];
      if (!from || !to) continue;
      positions.push(from.pos.x, from.pos.y, from.pos.z, to.pos.x, to.pos.y, to.pos.z);
    }
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      'position',
      new THREE.Float32BufferAttribute(positions, 3),
    );
    this.lines = new THREE.LineSegments(
      geometry,
      new THREE.LineBasicMaterial({ color: 0x96a1b8, transparent: true, opacity: 0.28 }),
    );
    this.scene.add(this.lines);
  }

  /**
   * Re-light by importance. Pass a Map of iri -> score in [0,1]; every node's
   * colour lerps from dim toward its palette hue by score. This is what makes
   * personalized PageRank legible: seed on where you stand, and the lattice
   * re-illuminates by relevance-to-here. Pass null to restore flat colour.
   */
  setImportance(scores) {
    // Unreached nodes go grey, not merely dark. Hue-vs-no-hue separates far
    // more cleanly than bright-vs-dim on this surface, and it keeps the
    // untouched part of the lattice legible as structure instead of erasing it.
    const grey = new THREE.Color(0x55607a);
    for (const node of this.nodes) {
      if (!node.mesh) continue;
      const colour = node.baseColor.clone();
      if (scores) {
        const score = scores.get(node.iri) || 0;
        if (score <= 0) {
          colour.copy(grey).multiplyScalar(0.5);
        } else {
          colour.multiplyScalar(0.55 + 0.75 * score);
        }
      }
      node.mesh.setColorAt(node.instance, colour);
    }
    for (const mesh of this.meshes) {
      if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    }
  }

  /** Legend rows: [{label, color, count}], in palette order. */
  legend() {
    return (this.types || []).map((t, i) => ({
      label: t.label || t.iri,
      color: i < SLOT_COLORS.length ? SLOT_COLORS[i] : OTHER_COLOR,
      count: t.count,
    }));
  }

  /**
   * Focus a node: the camera settles on its rank and closes in, so following a
   * prerequisite reads as travel down the lattice rather than a cut. Keeps the
   * current bearing — only the height and distance change — so you never lose
   * which way you were facing.
   */
  select(iri) {
    const node = this.nodes.find(n => n.iri === iri);
    if (!node) return null;
    this.selected = node;
    this.orbit.target.set(node.pos.x * 0.35, node.pos.y, node.pos.z * 0.35);
    const reach = Math.max(60, (this.maxRank || 1) * RANK_HEIGHT * 0.75);
    this.orbit.radius = Math.min(this.orbit.radius, reach);
    this._refog();
    this.onSelect(node);
    return node;
  }

  /** Prerequisites (outgoing) and dependents (incoming) of a node. */
  linksOf(node) {
    const requires = [];
    const unlocks = [];
    for (const [a, b, pred] of this.edges) {
      if (a === node.idx && this.nodes[b]) requires.push({ node: this.nodes[b], pred });
      if (b === node.idx && this.nodes[a]) unlocks.push({ node: this.nodes[a], pred });
    }
    return { requires, unlocks };
  }

  /** Frame the whole lattice, and pull the fog in around what is on screen. */
  frameAll() {
    const height = (this.maxRank || 0) * RANK_HEIGHT;
    const centre = new THREE.Vector3(0, height / 2, 0);
    // Bounding SPHERE, not extent: the widest rank sits nearest the camera and
    // a half-height estimate frames far too close, cropping the base ring.
    let bound = 40;
    for (const node of this.nodes) bound = Math.max(bound, node.pos.distanceTo(centre));
    this.orbit.target.copy(centre);
    this.orbit.radius = bound / Math.sin(this._halfFov()) * 1.1;
    this._refog();
  }

  /** Half of the narrower field of view, so wide and tall windows both fit. */
  _halfFov() {
    const vertical = (this.camera.fov * Math.PI) / 360;
    const horizontal = Math.atan(Math.tan(vertical) * this.camera.aspect);
    return Math.max(0.05, Math.min(vertical, horizontal));
  }

  /** Depth cue only — the far plane clears the back of whatever is framed. */
  _refog() {
    this.scene.fog.near = this.orbit.radius * 0.75;
    this.scene.fog.far = this.orbit.radius * 2.6;
  }

  resize() {
    const w = this.container.clientWidth || 1;
    const h = this.container.clientHeight || 1;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  _bindEvents() {
    const el = this.renderer.domElement;
    el.style.touchAction = 'none';
    let drag = null;

    el.addEventListener('pointerdown', (e) => {
      drag = { x: e.clientX, y: e.clientY, moved: 0, pan: e.button === 2 || e.shiftKey };
      el.setPointerCapture(e.pointerId);
    });
    el.addEventListener('pointermove', (e) => {
      const rect = el.getBoundingClientRect();
      this._pointer.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      this._pointer.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
      if (!drag) return;
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      drag.moved += Math.abs(dx) + Math.abs(dy);
      drag.x = e.clientX;
      drag.y = e.clientY;
      if (drag.pan) {
        this.orbit.target.y -= dy * this.orbit.radius * 0.0016;
      } else {
        this.orbit.theta -= dx * 0.006;
        this.orbit.phi = Math.min(3.03, Math.max(0.11, this.orbit.phi - dy * 0.006));
      }
    });
    const endDrag = (e) => {
      if (drag && drag.moved < 5) this._pick();
      drag = null;
      if (el.hasPointerCapture?.(e.pointerId)) el.releasePointerCapture(e.pointerId);
    };
    el.addEventListener('pointerup', endDrag);
    el.addEventListener('pointercancel', endDrag);
    el.addEventListener('contextmenu', e => e.preventDefault());
    el.addEventListener('wheel', (e) => {
      e.preventDefault();
      const k = Math.exp(e.deltaY * 0.0012);
      this.orbit.radius = Math.min(1400, Math.max(18, this.orbit.radius * k));
    }, { passive: false });
  }

  /** Raycast the instanced meshes and select whatever is under the pointer. */
  _pick() {
    this._raycaster.setFromCamera(this._pointer, this.camera);
    const hits = this._raycaster.intersectObjects(this.meshes, false);
    if (!hits.length) return;
    const hit = hits[0];
    const group = hit.object.userData.nodes || [];
    const node = group[hit.instanceId];
    if (node) this.select(node.iri);
  }

  _loop() {
    this._raf = requestAnimationFrame(() => this._loop());
    const { theta, phi, radius, target } = this.orbit;
    this.camera.position.set(
      target.x + radius * Math.sin(phi) * Math.cos(theta),
      target.y + radius * Math.cos(phi),
      target.z + radius * Math.sin(phi) * Math.sin(theta),
    );
    this.camera.lookAt(target);
    this.renderer.render(this.scene, this.camera);
  }

  destroy() {
    if (this._raf) cancelAnimationFrame(this._raf);
    this._resizeObserver.disconnect();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }
}
