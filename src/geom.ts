import type { Item, Netlist, Shape, SymbolDef } from "./types";

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

/** A symbol pin's PIN line (a label in a symbol): where its name shows (LTspice's LEFT, RIGHT,
 *  TOP, BOTTOM, V…, NONE) and how far from the pin; null for a net label. */
export function pinLine(extra?: string[]) {
  const l = extra?.find((x) => x.trimStart().startsWith("PIN "));
  if (!l) return null;
  const [, , , just = "NONE", off = ""] = l.trim().split(/\s+/);
  return { just: just.toUpperCase(), off: +off || 0 }; // (none: 0, as a placed part reads it)
}

/** Label pill geometry (shared by drawing and hit-testing). */
export function labelRect(it: Extract<Item, { t: "flag" }>, left: boolean): Box {
  const w = it.name.length * 7.8 + 12, x = left ? it.x - 6 - w : it.x + 6;
  return [x, it.y - 10, x + w, it.y + 10];
}

export function box(it: Item, lib: Lib, left?: Set<string>): Box | null {
  switch (it.t) {
    case "raw": return null;
    case "draw": return boxOf(outline(it)); // (what is drawn: an arc's drawn part, not its ellipse's box nor its rays)
    case "wire": return [Math.min(it.x1, it.x2), Math.min(it.y1, it.y2), Math.max(it.x1, it.x2), Math.max(it.y1, it.y2)];
    case "flag": {
      if (pinLine(it.extra)) return [it.x - 6, it.y - 6, it.x + 6, it.y + 6]; // (a symbol's pin: its square)
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
  it.t === "wire" ? { ...it, x1: it.x1 + dx, y1: it.y1 + dy, x2: it.x2 + dx, y2: it.y2 + dy }
  : it.t === "draw" ? { ...it, p: it.p.map((v, k) => v + (k % 2 ? dy : dx)) }
  : { ...it, x: it.x + dx, y: it.y + dy };

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

const RANK = { flag: 0, text: 1, wire: 2, sym: 3, draw: 4 } as const;
/** An arc's start angle and its span, counter-clockwise on screen as GDI draws it (both rays
 *  alike: the whole ellipse); one rule for symbols' arcs and the schematic's own. */
export function arcSpan([x1, y1, x2, y2, sx, sy, ex, ey]: number[]): [number, number] {
  const cx = (x1 + x2) / 2, cy = (y1 + y2) / 2, rx = Math.abs(x2 - x1) / 2 || 1, ry = Math.abs(y2 - y1) / 2 || 1;
  const a0 = Math.atan2((sy - cy) / ry, (sx - cx) / rx);
  let span = a0 - Math.atan2((ey - cy) / ry, (ex - cx) / rx);
  if (span <= 1e-9) span += 2 * Math.PI;
  return [a0, span];
}

/** A drawing's outline as a polyline: a line, a rectangle's four sides, a circle's or an arc's
 *  drawn curve (arcSpan), in segments of a size that keeps a big one round. Kept per item. */
const OUTLINES = new WeakMap<object, Pt[]>();
export function outline(it: Extract<Item, { t: "draw" }>): Pt[] {
  const got = OUTLINES.get(it);
  if (got) return got;
  const [x1, y1, x2, y2] = it.p, k = it.kind;
  let o: Pt[];
  if (k === "LINE") o = [[x1, y1], [x2, y2]];
  else if (k === "RECTANGLE") o = [[x1, y1], [x2, y1], [x2, y2], [x1, y2], [x1, y1]];
  else {
    const cx = (x1 + x2) / 2, cy = (y1 + y2) / 2, rx = Math.abs(x2 - x1) / 2, ry = Math.abs(y2 - y1) / 2;
    const [a0, span] = k === "ARC" ? arcSpan(it.p) : [0, 2 * Math.PI];
    const n = Math.min(512, Math.max(32, Math.ceil(4 * Math.sqrt(Math.max(rx, ry))))); // (a 2000-unit radius: under half a unit off)
    o = Array.from({ length: n + 1 }, (_, j) => { const a = a0 - (span * j) / n; return [cx + rx * Math.cos(a), cy + ry * Math.sin(a)] as Pt; });
  }
  OUTLINES.set(it, o);
  return o;
}
const boxOf = (o: Pt[]): Box => [Math.min(...o.map((q) => q[0])), Math.min(...o.map((q) => q[1])), Math.max(...o.map((q) => q[0])), Math.max(...o.map((q) => q[1]))];
const inBox = (b: Box, p: Pt, tol: number) => p[0] >= b[0] - tol && p[0] <= b[2] + tol && p[1] >= b[1] - tol && p[1] <= b[3] + tol;
/** Whether p is on a drawing's outline (within tol; its box first): a frame is picked by its edge,
 *  not its inside, so the circuit it frames stays clickable. */
function onDrawing(it: Extract<Item, { t: "draw" }>, p: Pt, tol: number) {
  // (the box of its first two points first, without any trigonometry: an arc lies within its ellipse's)
  const [x1, y1, x2, y2] = it.p;
  if (!inBox([Math.min(x1, x2), Math.min(y1, y2), Math.max(x1, x2), Math.max(y1, y2)], p, tol)) return false;
  const o = outline(it);
  return o.some((a, j) => j > 0 && distSeg(p, o[j - 1][0], o[j - 1][1], a[0], a[1]) <= tol);
}
/** Topmost item under p; flags > texts > wires > symbols > drawings (`drawings` false: none of
 *  them, while probing: no target there; `only`: just the items it accepts, a differential
 *  probe's end a wire or label even under a text). */
export function hit(items: Item[], lib: Lib, p: Pt, tol: number, drawings = true, only?: (it: Item) => boolean): number {
  let best = -1, rank = 9;
  const left = leftLabels(items);
  items.forEach((it, i) => {
    if (it.t === "raw" || RANK[it.t] > rank || (it.t === "draw" && !drawings) || (only && !only(it))) return; // equal rank: later = drawn on top wins
    const ok = it.t === "wire" ? distSeg(p, it.x1, it.y1, it.x2, it.y2) <= tol
      : it.t === "draw" ? onDrawing(it, p, tol)
      : inBox(box(it, lib, left)!, p, tol);
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

/** A label's port kind (In, Out, BiDir; "" none): its IOPIN line. */
export const portOf = (extra?: string[]) => extra?.find((l) => l.startsWith("IOPIN "))?.split(/\s+/)[3] ?? "";

/** ngspice voltage expression. Always quoted: ngspice misparses 3V3/5V and reads gt/and/not as operators. */
export const vexpr = (net: string) => (net === "0" ? "0" : `v("${net}")`); // (ground: no vector)

/** `P(R1)` in a trace expression: what `power(name)` gives for that part (the voltage across it
 *  times its current); a name it gives nothing for stays as written. */
export const expandPowers = (expr: string, power: (name: string) => string) =>
  expr.replace(POWER_CALLS, (m, name: string) => ((p) => (p ? `(${p})` : m))(power(name)));
/** A part's power in a trace expression, `P(R1)` (its name: group 1). */
export const POWER_CALL = /\bP\s*\(\s*([^()\s,]+)\s*\)/i;
const POWER_CALLS = new RegExp(POWER_CALL.source, "gi");
/** P() of anything (exprUnit's watts): an expression that asks for a power. */
export const POWER_ANY = /\bP\s*\(/i;

/** `expr` with each call of `name` (any case, blanks before `(` too) replaced by what `f` makes of
 *  its arguments (split at the commas outside parentheses); null: left as written. */
function replaceCalls(expr: string, name: string, f: (args: string[]) => string | null): string {
  const re = new RegExp(`\\b${name}\\s*\\(`, "gi");
  let out = "", from = 0, m: RegExpExecArray | null;
  while ((m = re.exec(expr))) {
    let depth = 0, end = -1;
    const args: string[] = [];
    let start = m.index + m[0].length;
    for (let i = start - 1; i < expr.length; i++) {
      const c = expr[i];
      if (c === "(") depth++;
      else if (c === ")" && --depth === 0) { end = i; break; }
      else if (c === "," && depth === 1) args.push(expr.slice(start, i).trim()), (start = i + 1);
    }
    if (end < 0) break;
    args.push(expr.slice(start, end).trim());
    const r = f(args);
    if (r == null) { // (left as written, but what is called inside it is not)
      const upTo = m.index + m[0].length;
      out += expr.slice(from, upTo);
      from = re.lastIndex = upTo;
      continue;
    }
    out += expr.slice(from, m.index) + r;
    from = end + 1;
    re.lastIndex = end + 1;
  }
  return out + expr.slice(from);
}

/** The netlist's `.func f(x) {…}` in a trace expression, as LTspice's plot defs: ngspice's plot
 *  expressions know none. A call becomes its body, each parameter its argument in parentheses;
 *  calls of other functions in it too (10 rounds at most: a recursive one stops there). */
export function funcsIn(expr: string, text: string): string {
  const defs = deckFuncs(text);
  if (!defs.length) return expr;
  for (let round = 0; round < 10; round++) {
    const before = expr;
    for (const d of defs) {
      // (all parameters at once: an argument's own letters are not a parameter's; any case, as SPICE)
      // (not a function's name: a parameter v is no V(out))
      if (d.stepped) continue; // (see funcsLeft)
      const re = new RegExp(`\\b(${d.params.join("|")})\\b(?!\\s*\\()`, "gi"), low = d.params.map((p) => p.toLowerCase());
      expr = replaceCalls(expr, d.name, (given) => {
        const args = given.length === 1 && !given[0] ? [] : given; // (`vt()`: none)
        if (args.length !== d.params.length) return null;
        // (in parentheses, an expression; inside a probe call, V(a): the name as given, no parentheses)
        const arg = (p: string, bare: boolean) => (bare ? args[low.indexOf(p.toLowerCase())] : `(${args[low.indexOf(p.toLowerCase())]})`);
        const probes = d.body.match(PROBE) ?? [];
        const filled = d.params.length ? d.body.split(PROBE).reduce((o, seg, k) => o + seg.replace(re, (p) => arg(p, false)) + (probes[k]?.replace(re, (p) => arg(p, true)) ?? ""), "") : d.body;
        return `(${d.withParams(filled)})`;
      });
    }
    if (expr === before) break;
  }
  return expr;
}

/** The top level's `.func`s of a netlist (a subcircuit's are its own), parsed once per text (the
 *  traces of a run all ask). */
type FuncDef = { name: string; params: string[]; body: string; withParams: (body: string) => string; stepped?: string };
/** The top level's .func and .param definitions of a netlist (a subcircuit's are its own), parsed
 *  once per text (the traces of a run all ask). */
let DEFS: { text: string; funcs: FuncDef[]; params: Map<string, string>; stepped: Map<string, string>; lines: string[] } | null = null;
/** A probe call in an expression, `V(out)`, `I(R1)`, `Ib(Q1)`: its argument is a name, no expression. */
const PROBE = /\b(?:v|i|i[bcdegsx])\s*\([^)]*\)/gi;
function deckDefs(text: string) {
  if (DEFS?.text === text) return DEFS;
  let depth = 0;
  // (a line's `;` or `$` comment is not of its body; a body continued on + lines is one line)
  const top = text.split("\n").filter((l) => { const t = l.trim().toLowerCase(); depth += t.startsWith(".subckt") ? 1 : t.startsWith(".ends") ? -1 : 0; return depth === 0; }).map((l) => l.split(/;|(?<![^\s])\$/)[0]).join("\n").replace(/\n\s*\+\s?/g, " ");
  const probe = PROBE; // (a node's name is no parameter's; imag() is no probe)
  // (the top level's .param values too: a body's `Rs` means nothing to ngspice's plot expressions)
  const params = new Map<string, string>();
  for (const m of top.matchAll(/^\s*\.param\s+(.+)$/gim))
    for (const p of m[1].matchAll(/(\w+)\s*=\s*(\{[^}]*\}|'[^']*'|\S+)/g)) params.set(p[1].toLowerCase(), p[2].replace(/^[{']|[}']$/g, ""));
  // (a parameter .step varies, and each whose value depends on one: a value per step, which one
  // expression cannot hold; by the name it is stepped as, said so)
  const roots = new Map<string, string>();
  // (the netlist's own: a `* .step` the user commented out is none)
  for (const m of top.matchAll(/^\*\s*\.step\s+(?:(?:dec|oct|lin)\s+)?param\s+(\w+).*\(nomospice: one run per step\)\s*$/gim)) roots.set(m[1].toLowerCase(), m[1]);
  const mentions = (body: string) => (roots.size ? body.replace(probe, "").match(new RegExp(`\\b(${[...roots.keys()].join("|")})\\b(?!\\s*\\()`, "i"))?.[1] : undefined);
  for (let round = 0; round < 8; round++) {
    let more = false;
    for (const [k, v] of params) {
      const hit = roots.has(k) ? undefined : mentions(v);
      if (hit) roots.set(k, roots.get(hit.toLowerCase())!), (more = true);
    }
    if (!more) break;
  }
  const names = params.size ? new RegExp(`\\b(${[...params.keys()].join("|")})\\b(?!\\s*\\()`, "gi") : null;
  const withParams = (body: string) => {
    if (!names) return body;
    for (let round = 0; round < 8; round++) {
      const before = body;
      const ps = body.match(probe) ?? [];
      body = body.split(probe).reduce((o, seg, k) => o + seg.replace(names, (n) => `(${params.get(n.toLowerCase())})`) + (ps[k] ?? ""), "");
      if (body === before) break;
    }
    return body;
  };
  const funcs = [...top.matchAll(/^\s*\.func(?:tion)?\s+(\w+)\s*\(([^)]*)\)\s*=?\s*(.+?)\s*$/gim)].map((m): FuncDef => {
    const body = m[3].replace(/^\{(.*)\}$/, "$1").replace(/^'(.*)'$/, "$1");
    const hit = mentions(body), root = hit && roots.get(hit.toLowerCase());
    return { name: m[1], params: m[2].split(/[\s,]+/).filter(Boolean), body, withParams, stepped: hit && (root?.toLowerCase() === hit.toLowerCase() ? hit : `${hit} (of ${root})`) };
  });
  DEFS = { text, funcs, params, stepped: roots, lines: text.split("\n") };
  return DEFS;
}
const deckFuncs = (text: string) => deckDefs(text).funcs;

/** Why a `.func` call in `expr` is not written out (see funcsIn): its body uses a parameter that
 *  `.step` varies, which one expression cannot hold per step. */
export function funcsLeft(expr: string, text: string): string | null {
  const d = deckFuncs(text).find((d) => d.stepped && new RegExp(`\\b${d.name}\\s*\\(`, "i").test(expr));
  return d ? `${d.name}(): its .func uses ${d.stepped}, which .step varies; a trace cannot take a value per step` : null;
}

/** A two-port quantity of `.net` in a trace expression: S21(V1), S11(V1), Zin(V1). */
export const TWO_PORT_CALL = /\b(s11|s21|zin)\s*\(/i;

/** `.net [<out>] <V source> …` as it stands in the run's netlist: the output (`I(R1)`, `V(out, ref)`
 *  with blanks; none: a one-port), the source (a voltage source), the rest (Rin=, Rout=). */
export const NET_DIRECTIVE = /^\*\s*\.net\s+(?:([IV]\s*\([^)]*\)|\S+)\s+)?(V\S*)(.*?)\s*; nomospice: two-port\s*$/im; // (the netlist's mark: a `* .net` of the user's is none)

/** LTspice's `.net` (a comment in the run's netlist: ngspice has no such analysis) in a trace
 *  expression of an .ac result: Zin(V1), S11(V1) and S21(V1) of the two-port from source V1 to the
 *  load R1 (`.net I(R1) V1`), as the circuit terminates it: V1's Rser the input's reference
 *  impedance, R1 the output's (V1's AC amplitude the drive). One drive gives these only: S12, S22,
 *  other reference impedances (LTspice's Rin=, Rout=), V(out) without a load or a current source as
 *  the input would take more (said: left as written). */
export function twoPortIn(expr: string, nl: Netlist): string {
  if (!TWO_PORT_CALL.test(expr)) return expr;
  const m = NET_DIRECTIVE.exec(nl.text);
  if (!m || /\b(rin|rout)\s*=/i.test(m[3])) return expr;
  const [, spec, src] = m, out = (spec ?? "").replace(/\s+/g, ""); // (`.net V1`, a one-port: Zin and S11 only)
  const { lines, params, stepped } = deckDefs(nl.text);
  const line = (name: string) => lines.find((l) => l.split(/\s+/)[0]?.toLowerCase() === name.toLowerCase())?.split(/\s+/);
  // (a value `{RL}` of a .param: the value itself, where that is a number; one .step varies: not)
  const resolved = (x?: string) => ((m) => (m && !stepped.has(m[1].toLowerCase()) ? params.get(m[1].toLowerCase()) ?? x : x))(x ? /^\{\s*(\w+)\s*\}$/.exec(x) : null);
  const nets = (inst: string) => nl.nets[nl.insts.findIndex((x) => x.toLowerCase() === inst.toLowerCase())];
  const across = (p?: string[]) => (p?.length === 2 ? `(${vexpr(p[0])}-${vexpr(p[1])})` : "");
  const vin = across(nets(src));
  if (!vin) return expr;
  const z0 = resolved(line(`R${src}_rser`)?.[3]);
  // (the AC amplitude after the nodes, `AC 1`: a node named ac is none; a parameter: not here)
  const t = line(src)?.slice(3) ?? [], ac = t.findIndex((x) => /^ac$/i.test(x)), vs = ac >= 0 ? t[ac + 1] ?? "1" : "1";
  const phase = ac >= 0 ? parseFloat(t[ac + 2] ?? "0") : 0, phased = Number.isFinite(phase) && phase !== 0; // (a drive of a phase: not folded in)
  const drive = nets(src)?.[0] === "0" ? `-${vs}` : vs; // (the source drawn with its + pin at ground: its EMF the other way)
  const zin = `(${vin}/(-I(${src})))`; // (I(V1): into its + pin)
  const r = /^I\((\w+)\)$/i.exec(out);
  // (a resistor of a plain value as the load, an Rser of one as the source's: else no reference)
  const plain = (x?: string) => !!x && /^[\d.]+(e[-+]?\d+)?[a-zµ]*$/i.test(x);
  // (the load to ground: its node's voltage, whichever way the resistor is drawn)
  const loadV = (p?: string[]) => (p?.length !== 2 ? "" : p[0] === "0" ? vexpr(p[1]) : p[1] === "0" ? vexpr(p[0]) : across(p));
  const [vl, rl] = r && /^r/i.test(r[1]) && plain(resolved(line(r[1])?.[3])) ? [loadV(nets(r[1])), resolved(line(r[1])?.[3])] : ["", undefined];
  let e = expr;
  for (const f of ["zin", "s11", "s21"]) {
    e = replaceCalls(e, f, ([s]) => s?.toLowerCase() !== src.toLowerCase() ? null
      : f === "zin" ? zin
      : !plain(z0) ? null
      : f === "s11" ? `((${zin}-(${z0}))/(${zin}+(${z0})))`
      : vl && rl && plain(vs) && !phased ? `(2*${vl}/(${drive})*sqrt((${z0})/(${rl})))` : null); // (S21 alone needs the drive)
  }
  return e;
}

/** A wire or a label: what has a net (a voltage to probe, a differential probe's end). */
export const isNet = (it: Item | undefined) => it?.t === "wire" || it?.t === "flag";

/** The name of part `i`'s current, which is also its trace expression (the backend reads I(R1),
 *  Ib(Q1) …): into `pin` where that has a sensor, else the part's own (pin 1; a source's I(V1)).
 *  "": none (a subcircuit's current is per pin). */
export function partCurrent(nl: Netlist, i: number, pin?: number) {
  const named = nl.pin_currents?.[i] ?? [];
  return (pin != null && named[pin]) || (nl.probes[i] ? named[0] ?? "" : "");
}

/** Part `i`'s power if it has two pins: the voltage across it times the current into pin 1 —
 *  positive where it takes power (a resistor), negative where it gives it (a source); else "". */
export function powerExpr(nl: Netlist, i: number) {
  const pins = nl.nets[i], cur = partCurrent(nl, i);
  return cur && pins?.length === 2 ? `(${vexpr(pins[0])}-${vexpr(pins[1])})*${cur}` : "";
}

/** Each P(R1) in `expr` the product of voltage and current by netlist `nl` (a trace's P(R1)
 *  follows the part when it is rewired): the part by the name it is drawn with (its current's,
 *  I(D1)), else by its netlist's (XD1, a subcircuit U1's XU1). */
export function powersIn(expr: string, nl: Netlist) {
  const part = (n: string) => {
    const drawn = nl.insts.findIndex((_, j) => partCurrent(nl, j).toLowerCase() === `i(${n})`);
    return drawn >= 0 ? drawn : nl.insts.findIndex((x) => x.toLowerCase() === n || x.toLowerCase() === `x${n}`);
  };
  return expandPowers(expr, (n) => powerExpr(nl, part(n.toLowerCase())));
}
