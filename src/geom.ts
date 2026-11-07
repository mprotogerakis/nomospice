import type { Item, Shape, SymbolDef } from "./types";

export const GRID = 16;
export type Pt = [number, number];
export type Box = [number, number, number, number];
type Sym = Extract<Item, { t: "sym" }>;
export type Lib = Record<string, SymbolDef>;

export const snap = (v: number) => Math.round(v / GRID) * GRID;
export const key = (x: number, y: number) => `${x},${y}`;
/** `Opamps\opamp` -> `opamp`; generated `subckt:BS/MMBF170` keys keep their `/` (schematic.rs sym_key). */
export const symKey = (name: string) => (/^subckt:/i.test(name) ? name : name.split(/[\\/]/).pop()!).toLowerCase();
export const defOf = (lib: Lib, it: Sym) => lib[symKey(it.sym)] as SymbolDef | undefined;

/** LTspice orientation: rotate n*90° clockwise, `M` negates x afterwards (same as schematic.rs). */
export function xform(rot: string, x: number, y: number): Pt {
  const n = rot.slice(1);
  const p: Pt = n === "90" ? [-y, x] : n === "180" ? [-x, -y] : n === "270" ? [y, -x] : [x, y];
  return rot[0] === "M" ? [-p[0] || 0, p[1]] : p;
}
export function matrix(rot: string, x: number, y: number) {
  const [a, b] = xform(rot, 1, 0);
  const [c, d] = xform(rot, 0, 1);
  return `matrix(${a} ${b} ${c} ${d} ${x} ${y})`;
}
export const rotate = (rot: string) =>
  rot[0] === "M" ? `M${(+rot.slice(1) + 270) % 360}` : `R${(+rot.slice(1) + 90) % 360}`;
export const mirror = (rot: string) => (rot[0] === "M" ? "R" : "M") + rot.slice(1);

export function pins(it: Sym, def?: SymbolDef): Pt[] {
  return (def?.pins ?? []).map((p) => {
    const [dx, dy] = xform(it.rot, p.x, p.y);
    return [it.x + dx, it.y + dy];
  });
}

const localBoxes = new WeakMap<SymbolDef, Box>();
function localBox(def: SymbolDef): Box {
  let b = localBoxes.get(def);
  if (!b) {
    const xs: number[] = [], ys: number[] = [];
    for (const [, p] of def.shapes) for (let i = 0; i < 4; i += 2) xs.push(p[i]), ys.push(p[i + 1]);
    for (const p of def.pins) xs.push(p.x), ys.push(p.y);
    b = xs.length ? [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)] : [-16, -16, 16, 16];
    localBoxes.set(def, b);
  }
  return b;
}

export function symBox(it: Sym, def?: SymbolDef): Box {
  const [x0, y0, x1, y1] = def ? localBox(def) : [-16, -16, 16, 16];
  const a = xform(it.rot, x0, y0), b = xform(it.rot, x1, y1);
  return [it.x + Math.min(a[0], b[0]), it.y + Math.min(a[1], b[1]), it.x + Math.max(a[0], b[0]), it.y + Math.max(a[1], b[1])];
}

export const textLines = (t: string) => t.split("\n");

/** Anchors of labels drawn to the left (their wire leaves to the right), as "x,y". */
export function leftLabels(items: Item[]) {
  const out = new Set<string>();
  for (const w of items) {
    if (w.t !== "wire") continue;
    if (w.x2 > w.x1) out.add(key(w.x1, w.y1));
    if (w.x1 > w.x2) out.add(key(w.x2, w.y2));
  }
  return out;
}

/** Label pill geometry (shared by drawing and hit-testing). */
export function labelRect(it: Extract<Item, { t: "flag" }>, left: boolean): Box {
  const w = it.name.length * 7.8 + 12, x = left ? it.x - 6 - w : it.x + 6;
  return [x, it.y - 10, x + w, it.y + 10];
}

export function box(it: Item, lib: Lib, left?: Set<string>): Box | null {
  switch (it.t) {
    case "raw": return null;
    case "wire": return [Math.min(it.x1, it.x2), Math.min(it.y1, it.y2), Math.max(it.x1, it.x2), Math.max(it.y1, it.y2)];
    case "flag": {
      if (it.name === "0") return [it.x - 16, it.y, it.x + 16, it.y + 16];
      const r = labelRect(it, !!left?.has(key(it.x, it.y)));
      return [Math.min(r[0], it.x - 3), r[1], Math.max(r[2], it.x + 3), r[3]];
    }
    case "text": {
      const lines = textLines(it.text);
      return [it.x, it.y - 10, it.x + Math.max(...lines.map((l) => l.length)) * 7.6, it.y - 10 + lines.length * 17];
    }
    case "sym": return symBox(it, defOf(lib, it));
  }
}

export const union = (all: (Box | null)[]): Box | null => {
  const bs = all.filter((b): b is Box => !!b);
  return bs.length ? [Math.min(...bs.map((b) => b[0])), Math.min(...bs.map((b) => b[1])), Math.max(...bs.map((b) => b[2])), Math.max(...bs.map((b) => b[3]))] : null;
};
export const isShape = (it: Item): it is Shape => it.t !== "raw";

/** `it` moved by (dx, dy). */
export const shifted = (it: Shape, dx: number, dy: number): Shape =>
  it.t === "wire" ? { ...it, x1: it.x1 + dx, y1: it.y1 + dy, x2: it.x2 + dx, y2: it.y2 + dy } : { ...it, x: it.x + dx, y: it.y + dy };

function distSeg(p: Pt, x1: number, y1: number, x2: number, y2: number) {
  const dx = x2 - x1, dy = y2 - y1;
  const t = dx || dy ? Math.max(0, Math.min(1, ((p[0] - x1) * dx + (p[1] - y1) * dy) / (dx * dx + dy * dy))) : 0;
  return Math.hypot(p[0] - x1 - t * dx, p[1] - y1 - t * dy);
}

/** Strictly inside a wire (T-junction rule, same as schematic.rs). */
export function inside(p: Pt, w: Extract<Item, { t: "wire" }>) {
  const cross = (w.x2 - w.x1) * (p[1] - w.y1) - (w.y2 - w.y1) * (p[0] - w.x1);
  const dot = (p[0] - w.x1) * (w.x2 - w.x1) + (p[1] - w.y1) * (w.y2 - w.y1);
  return cross === 0 && dot > 0 && dot < (w.x2 - w.x1) ** 2 + (w.y2 - w.y1) ** 2;
}

const RANK = { flag: 0, text: 1, wire: 2, sym: 3 } as const;
/** Topmost item under p; flags > texts > wires > symbols. */
export function hit(items: Item[], lib: Lib, p: Pt, tol: number): number {
  let best = -1, rank = 9;
  const left = leftLabels(items);
  items.forEach((it, i) => {
    if (it.t === "raw" || RANK[it.t] > rank) return; // equal rank: later = drawn on top wins
    const b = box(it, lib, left)!;
    const ok = it.t === "wire"
      ? distSeg(p, it.x1, it.y1, it.x2, it.y2) <= tol
      : p[0] >= b[0] - tol && p[0] <= b[2] + tol && p[1] >= b[1] - tol && p[1] <= b[3] + tol;
    if (ok) (best = i), (rank = RANK[it.t]);
  });
  return best;
}

/** Junction dots, unconnected pins, and whether a point is a connection (for ending wires). */
export function connectivity(items: Item[], lib: Lib) {
  const pts = new Map<string, { p: Pt; ends: number; pins: number; flags: number }>();
  const add = (p: Pt, k: "ends" | "pins" | "flags") => {
    const e = pts.get(key(...p)) ?? { p, ends: 0, pins: 0, flags: 0 };
    e[k]++;
    pts.set(key(...p), e);
  };
  const wires = items.filter((it): it is Extract<Item, { t: "wire" }> => it.t === "wire");
  for (const it of items) {
    if (it.t === "wire") add([it.x1, it.y1], "ends"), add([it.x2, it.y2], "ends");
    else if (it.t === "flag") add([it.x, it.y], "flags");
    else if (it.t === "sym") pins(it, defOf(lib, it)).forEach((p) => add(p, "pins"));
  }
  const dots: Pt[] = [], open: Pt[] = [];
  // ponytail: O(points × wires); fine for hand-drawn schematics, add a spatial index past ~10k wires.
  for (const e of pts.values()) {
    const through = wires.filter((w) => inside(e.p, w)).length;
    const total = e.ends + e.pins + 2 * through;
    if (total >= 3 && e.ends + through > 0) dots.push(e.p);
    if (e.pins === 1 && total === 1 && !e.flags) open.push(e.p);
  }
  const isConn = (p: Pt) => pts.has(key(...p)) || wires.some((w) => inside(p, w));
  return { dots, open, isConn };
}

/** Rubber-band drag: selected items move, wire ends / flags touching them follow, and no
 *  connection is lost (LTspice keeps them too):
 *  - a wire whose both ends move (it joins two moving parts) moves whole, like a selected one;
 *  - an unselected wire ending inside a wholly moving wire (a T-leg) follows it;
 *  - a wire that stretches (one end, or a point inside it, moves) is first cut at the connection
 *    points inside it (T-junctions, pins, labels): only the piece at the moving point stretches,
 *    so a junction further along does not slide off the slanted wire.
 *  The cut pieces are appended (the items' indices, and so the selection, stay valid). */
export function dragPlan(items: Item[], lib: Lib, sel: Set<number>) {
  type Wire = Extract<Item, { t: "wire" }>;
  const conn: Pt[] = [];
  const moving = new Set<string>(), movingPts: Pt[] = [];
  const move = (p: Pt) => moving.has(key(...p)) || (moving.add(key(...p)), movingPts.push(p));
  // pins of parts that stay: a wire must not take a connection away from them (a selected wire's
  // end on one stays there: the wire stretches; with both ends on such pins it moves whole and is
  // bridged back to them, below)
  const fixed = new Set(items.flatMap((it, i) => (it.t === "sym" && !sel.has(i) ? pins(it, defOf(lib, it)).map((p) => key(...p)) : [])));
  items.forEach((it, i) => {
    const ps: Pt[] = it.t === "wire" ? [[it.x1, it.y1], [it.x2, it.y2]] : it.t === "flag" ? [[it.x, it.y]] : it.t === "sym" ? pins(it, defOf(lib, it)) : [];
    conn.push(...ps);
    if (sel.has(i)) ps.forEach((p) => (it.t !== "wire" || !fixed.has(key(...p))) && move(p));
  });
  const ends = (w: Wire): Pt[] => [[w.x1, w.y1], [w.x2, w.y2]];
  const pinned = (w: Wire) => conn.some((p) => inside(p, w) && fixed.has(key(...p))); // (cheap test first)
  // wires that move whole, and what hangs on them: a worklist, each wire once. A wire whose ends
  // both move joins (one with a resting part's pin on it stretches instead: the pin keeps its wires)
  const whole = new Set<number>(), work: number[] = [];
  const atEnd = new Map<string, number[]>(); // wires by end point
  items.forEach((it, i) => it.t === "wire" && ends(it).forEach((p) => atEnd.set(key(...p), [...(atEnd.get(key(...p)) ?? []), i])));
  const join = (i: number) => {
    const it = items[i];
    // both ends moving (selected or not; cheap test first), and no resting part's pin on it: else
    // it stretches around the pin
    if (whole.has(i) || it.t !== "wire" || !ends(it).every((p) => moving.has(key(...p))) || pinned(it)) return;
    whole.add(i), work.push(i);
  };
  items.forEach((_, i) => join(i));
  // a selected wire between two resting pins: it moves whole (T-legs on it follow, below), and new
  // wires keep those pins on it — its ends' and any resting pin inside it
  const bridged = items.flatMap((it, i): [Wire, Pt[]][] =>
    it.t === "wire" && sel.has(i) && ends(it).every((p) => fixed.has(key(...p)))
      ? (whole.add(i), work.push(i), [[it, [...ends(it), ...conn.filter((p) => fixed.has(key(...p)) && inside(p, it))]]])
      : []);
  // what can follow a whole wire: unselected wire ends and labels on it
  const followers = items.flatMap((o, j): Pt[] => (sel.has(j) ? [] : o.t === "wire" ? ends(o) : o.t === "flag" ? [[o.x, o.y]] : []));
  while (work.length) {
    const w = items[work.pop()!] as Wire;
    for (const p of followers) {
      if (!inside(p, w) || moving.has(key(...p)) || fixed.has(key(...p))) continue;
      move(p); // a T-leg follows
      (atEnd.get(key(...p)) ?? []).forEach(join); // its wire may now move whole too
    }
  }
  const all: Item[] = [], pieces: Item[] = [], groups: number[][] = []; // groups: a cut wire's pieces, in order
  items.forEach((it, i) => {
    if (it.t !== "wire" || whole.has(i) || !(ends(it).some((p) => moving.has(key(...p))) || movingPts.some((p) => inside(p, it)))) return void all.push(it);
    // cut where it matters only: at a moving point inside it, and at the nearest connection on
    // each side of every moving point (further junctions stay on an unmoved piece: no fragments)
    const d = (p: Pt) => Math.abs(p[0] - it.x1) + Math.abs(p[1] - it.y1); // along the wire
    const inner = [...new Map(conn.filter((p) => inside(p, it)).map((p) => [key(...p), p])).values()].sort((p, q) => d(p) - d(q));
    const marks = [...ends(it), ...inner].filter((p) => moving.has(key(...p))).map(d);
    const cuts = inner.filter((p, k) => moving.has(key(...p)) || marks.some((m) =>
      (d(p) > m && (k === 0 || d(inner[k - 1]) <= m)) || (d(p) < m && (k === inner.length - 1 || d(inner[k + 1]) >= m))));
    const pts: Pt[] = [[it.x1, it.y1], ...cuts, [it.x2, it.y2]];
    if (cuts.length) groups.push([all.length, ...cuts.map((_, k) => items.length + pieces.length + k)]);
    pts.slice(1).forEach(([x2, y2], k) => (k ? pieces : all).push({ ...it, x1: pts[k][0], y1: pts[k][1], x2, y2 }));
  });
  all.push(...pieces);
  // pieces that end up in line again (not stretched, or dragged back) become one wire again: a
  // junction inside a wire stays connected (LTspice's rule), so nothing is lost, and nothing piles up
  const rejoin = (out: Item[]): Item[] => {
    const gone = new Set<number>();
    for (const g of groups) {
      let cur = g[0];
      for (const k of g.slice(1)) {
        const [a, b] = [out[cur], out[k]] as Wire[];
        const [ux, uy, vx, vy] = [a.x2 - a.x1, a.y2 - a.y1, b.x2 - b.x1, b.y2 - b.y1];
        if (a.x2 === b.x1 && a.y2 === b.y1 && ux * vy === uy * vx && ux * vx + uy * vy > 0) (out[cur] = { ...a, x2: b.x2, y2: b.y2 }), gone.add(k);
        else cur = k;
      }
    }
    return gone.size ? out.filter((_, i) => !gone.has(i)) : out; // only appended pieces go: indices below stay
  };
  const bridges = (dx: number, dy: number): Item[] =>
    bridged.flatMap(([w, at]) => [...new Map(at.map((p) => [key(...p), p])).values()].map(([x, y]) => ({ ...w, x1: x, y1: y, x2: x + dx, y2: y + dy })));
  return (dx: number, dy: number): Item[] => [
    ...rejoin(all.map((it, i) => {
      if (it.t === "raw") return it;
      if (i < items.length && (whole.has(i) || (sel.has(i) && items[i].t !== "wire"))) {
        return shifted(it, dx, dy);
      }
      if (it.t === "wire") {
        const a = moving.has(key(it.x1, it.y1)), b = moving.has(key(it.x2, it.y2));
        return a || b ? { ...it, x1: it.x1 + (a ? dx : 0), y1: it.y1 + (a ? dy : 0), x2: it.x2 + (b ? dx : 0), y2: it.y2 + (b ? dy : 0) } : it;
      }
      if (it.t === "flag" && moving.has(key(it.x, it.y))) return { ...it, x: it.x + dx, y: it.y + dy };
      return it;
    })),
    ...bridges(dx, dy), // (zero-length while not moved: dropped on commit)
  ];
}

const zeroWire = (it: Item) => it.t === "wire" && it.x1 === it.x2 && it.y1 === it.y2;
export const dropZeroWires = (items: Item[]) => items.filter((it) => !zeroWire(it));
/** Drop zero-length wires and carry a selection (item indices) across the removal. */
export function dropZeroWiresKeepSel(items: Item[], sel: number[]) {
  const map = new Map<number, number>();
  const out = items.filter((it, i) => !zeroWire(it) && (map.set(i, map.size), true));
  return { items: out, sel: sel.flatMap((i) => (map.has(i) ? [map.get(i)!] : [])) };
}

/** Rotate a symbol in place: keep its bbox centre on the grid. */
export function rotateInPlace(it: Sym, lib: Lib, f: (r: string) => string): Sym {
  const def = defOf(lib, it);
  const [a, b, c, d] = symBox(it, def);
  const next = { ...it, rot: f(it.rot) };
  const [e, g, h, k] = symBox(next, def);
  return { ...next, x: it.x + snap((a + c - e - h) / 2), y: it.y + snap((b + d - g - k) / 2) };
}

/** A part's SPICE prefix letter: its own Prefix attribute (LTspice: X calls a subcircuit) over the
 *  symbol's, as the netlister has it. */
export const prefixOf = (it: Sym, def?: SymbolDef) => ((it.attrs.Prefix?.trim() || def?.prefix || "")[0] ?? "").toUpperCase();

/** ngspice voltage expression. Always quoted: ngspice misparses 3V3/5V and reads gt/and/not as operators. */
export const vexpr = (net: string) => (net === "0" ? "0" : `v("${net}")`); // (ground: no vector)


