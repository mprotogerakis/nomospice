import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { useShallow } from "zustand/react/shallow";
import { Maximize, Minus, Plus } from "lucide-react";
import type { Item, Shape, SymbolDef } from "./types";
import {
  box, connectivity, defOf, pinLine, portOf, arcSpan, dragPlan, mirror, outline, pins, prefixOf, rotate, dropZeroWires, dropZeroWiresKeepSel, GRID, hit, isShape, labelRect, leftLabels, matrix, shifted, snap, symBox,
  textLines, union, xform, type Box, type Lib, type Pt,
} from "./geom";
import { canProbe, commit, currentNets, isSymbolDoc, nextInst, partCurrent, powerMode, powerName, probe, probeMode, simulate, useS, useStatus } from "./store";
import { copySelection, deleteSelection, duplicate, newItem, pasteClipboard, transform } from "./actions";
import { ANALYSIS, ANY_ANALYSIS, analysisOptions, continuation } from "./spice";
import { openMenu, type MenuItem } from "./Menu";
import { useSize } from "./lib/useSize";
import { cn } from "@/lib/utils";
import { inlineStyles, saveSvg, withLightTheme } from "./svgExport";

type View = { x: number; y: number; k: number };
type Drag =
  | { k: "pan"; sx: number; sy: number; vx: number; vy: number; moved: boolean }
  | { k: "move"; start: Pt; plan: (dx: number, dy: number) => Item[]; moved: boolean; index: number; at: Pt; base: Item[]; pin?: number }
  | { k: "box"; start: Pt; end: Pt };

// SwiftFlow viewport constants (Models/Viewport.swift, FitViewOptions.swift)
const MIN_ZOOM = 0.1, MAX_ZOOM = 6, FIT_PADDING = 80, FIT_MAX = 2.5, ZOOM_STEP = 1.2;
const clampZoom = (k: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, k));

const cursorSvg = (body: string) =>
  `url("data:image/svg+xml,${encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'>${body}</svg>`)}") 1 23, crosshair`;
const PROBE_V = cursorSvg("<path d='M2 22 L9 15' stroke='%23e0573a' stroke-width='2'/><path d='M8 16 L18 6 L21 9 L11 19 Z' fill='%23e0573a' stroke='white' stroke-width='1'/>".replaceAll("%23", "#"));
const PROBE_I = cursorSvg("<circle cx='13' cy='11' r='7' fill='none' stroke='%23d9a441' stroke-width='3'/><path d='M2 22 L8 16' stroke='%23d9a441' stroke-width='2'/><path d='M13 6 v10 M10 13 l3 3 l3 -3' stroke='white' fill='none' stroke-width='1.5'/>".replaceAll("%23", "#"));

function arcPath(p: number[]) {
  const [x1, y1, x2, y2] = p;
  const cx = (x1 + x2) / 2, cy = (y1 + y2) / 2, rx = Math.abs(x2 - x1) / 2 || 1, ry = Math.abs(y2 - y1) / 2 || 1;
  const [a0, d] = arcSpan(p), a1 = a0 - d;
  const at = (a: number) => `${cx + rx * Math.cos(a)} ${cy + ry * Math.sin(a)}`;
  // (the whole ellipse: one SVG arc cannot end where it starts, so two halves)
  if (d > 2 * Math.PI - 1e-9) return `M${at(a0)} A${rx} ${ry} 0 1 0 ${at(a0 + Math.PI)} A${rx} ${ry} 0 1 0 ${at(a0)}`;
  return `M${at(a0)} A${rx} ${ry} 0 ${d > Math.PI ? 1 : 0} 0 ${at(a1)}`;
}

/** One <g> per library symbol in <defs>; instances are <use>. Attributes must sit on the
 *  referenced element itself: <use> clones inherit from the <use>, not from <defs>. */
/** LTspice line styles after the coordinates: 1 dash, 2 dot, 3 dash-dot, 4 dash-dot-dot (screen px). */
const DASH: Record<number, string> = { 1: "5 4", 2: "1 3", 3: "5 3 1 3", 4: "5 3 1 3 1 3" };

/** A symbol's shape: line, rect, circle or arc, then its style. */
function shapeEl(k: string, p: number[], key: number | string) {
  const s = { vectorEffect: "non-scaling-stroke", strokeDasharray: DASH[p[k === "arc" ? 8 : 4]] } as const;
  return k === "line" ? <line key={key} {...s} x1={p[0]} y1={p[1]} x2={p[2]} y2={p[3]} />
  : k === "rect" ? <rect key={key} {...s} x={Math.min(p[0], p[2])} y={Math.min(p[1], p[3])} width={Math.abs(p[2] - p[0])} height={Math.abs(p[3] - p[1])} />
  : k === "circle" ? <ellipse key={key} {...s} cx={(p[0] + p[2]) / 2} cy={(p[1] + p[3]) / 2} rx={Math.abs(p[2] - p[0]) / 2} ry={Math.abs(p[3] - p[1]) / 2} />
  : <path key={key} {...s} d={arcPath(p)} />;
}

/** A schematic's own drawing: the very outline it is picked and boxed by (geom's outline: a
 *  polyline, a circle's or an arc's curve in 64 segments), so what is seen is what is hit. */
const drawEl = (it: Extract<Item, { t: "draw" }>) => (
  <polyline points={outline(it).map((q) => q.join(",")).join(" ")} vectorEffect="non-scaling-stroke" strokeDasharray={DASH[it.style ?? 0]} strokeLinejoin="round" />
);

function SymbolGraphics({ def }: { def: SymbolDef }) {
  return (
    <g id={`sym-${def.name}`} fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round">
      {def.shapes.map(([k, p], i) => shapeEl(k, p, i))}
    </g>
  );
}


/** InstName/Value labels: window anchors for upright symbols, above/below for rotated ones. */
function SymLabels({ it, def }: { it: Extract<Item, { t: "sym" }>; def?: SymbolDef }) {
  const inst = it.attrs.InstName ?? "", value = it.attrs.Value ?? def?.value ?? "";
  const extra = [it.attrs.Value2, it.attrs.SpiceLine].filter(Boolean).join(" ");
  const [dx, dy] = xform(it.rot, 1, 0);
  const bb = symBox(it, def);
  const cls = "fill-[var(--sch-label)] text-[13px]";
  if (dy !== 0 || !def) {
    const cx = (bb[0] + bb[2]) / 2;
    return (
      <g className={cls} textAnchor="middle">
        <text x={cx} y={bb[1] - 7}>{inst}</text>
        <text x={cx} y={bb[3] + 16}>{value}</text>
        {extra && <text x={cx} y={bb[3] + 32} className="text-[11px] fill-[var(--text-tertiary)]">{extra}</text>}
      </g>
    );
  }
  const at = (w: string, fy: number): Pt => {
    const win = def.windows[w];
    if (!win) return [dx > 0 ? bb[2] + 6 : bb[0] - 6, fy];
    const [wx, wy] = xform(it.rot, win[0], win[1]);
    return [it.x + wx, it.y + wy];
  };
  const a = at("0", (bb[1] + bb[3]) / 2 - 8), b = at("3", (bb[1] + bb[3]) / 2 + 8);
  return (
    <g className={cls} textAnchor={dx > 0 ? "start" : "end"} dominantBaseline="middle">
      <text x={a[0]} y={a[1]}>{inst}</text>
      <text x={b[0]} y={b[1]}>{value}</text>
      {extra && <text x={b[0]} y={b[1] + 15} className="text-[11px] fill-[var(--text-tertiary)]">{extra}</text>}
    </g>
  );
}

/** Where a pin's name goes from it (its LTspice justification), the text upright: LEFT starts
 *  right of the pin, TOP below it …; the vertical kinds as their direction says. */
const JUST: Record<string, Pt> = { LEFT: [1, 0], RIGHT: [-1, 0], TOP: [0, 1], BOTTOM: [0, -1], VLEFT: [0, -1], VRIGHT: [0, 1], VTOP: [1, 0], VBOTTOM: [-1, 0] };

/** Pin names: inside a generated subcircuit box, or where the symbol puts them (a block's ports,
 *  an IC's pins), kept upright whatever the rotation. */
function PinLabels({ it, def }: { it: Extract<Item, { t: "sym" }>; def: SymbolDef }) {
  const bb = symBox(it, def), cx = (bb[0] + bb[2]) / 2, cy = (bb[1] + bb[3]) / 2;
  return (
    <g stroke="none" className="fill-[var(--text-secondary)] font-mono text-[10px]">
      {def.pins.map((p, k) => {
        const [dx, dy] = xform(it.rot, p.x, p.y);
        const [x, y] = [it.x + dx, it.y + dy];
        if (!def.pin_labels) {
          const dir = JUST[p.just ?? ""];
          if (!dir || !p.name) return null;
          const [ux, uy] = xform(it.rot, dir[0], dir[1]), off = p.off || 8;
          return (
            <text key={k} x={x + ux * off} y={y + uy * off} dominantBaseline={uy > 0 ? "hanging" : uy < 0 ? "auto" : "central"}
              textAnchor={ux > 0 ? "start" : ux < 0 ? "end" : "middle"}>{p.name}</text>
          );
        }
        const ux = Math.sign(cx - x), uy = Math.sign(cy - y), horiz = Math.abs(cx - x) >= Math.abs(cy - y);
        return (
          <text key={k} x={x + (horiz ? ux * 36 : 0)} y={y + (horiz ? 0 : uy * 40)} dominantBaseline="central"
            textAnchor={horiz ? (ux > 0 ? "start" : "end") : "middle"}>{p.name}</text>
        );
      })}
    </g>
  );
}

/** A symbol's pin (a label with its PIN line): a square, and its name where the placed part shows it
 *  (faint where it does not). */
function Pin({ it, pin }: { it: Extract<Item, { t: "flag" }>; pin: NonNullable<ReturnType<typeof pinLine>> }) {
  const dir = JUST[pin.just], o = dir ? pin.off : 8; // (a hidden one's name, here only: beside its square)
  const [ux, uy] = dir ?? [1, -1];
  return (
    <g>
      <rect x={it.x - 3} y={it.y - 3} width={6} height={6} fill="none" />
      <text x={it.x + ux * o} y={it.y + uy * o} fill="currentColor" stroke="none" opacity={dir ? 1 : 0.55} className="font-mono text-[11px]"
        dominantBaseline={uy > 0 ? "hanging" : uy < 0 ? "auto" : "central"} textAnchor={ux > 0 ? "start" : ux < 0 ? "end" : "middle"}>{it.name}</text>
    </g>
  );
}

function Flag({ it, leftward }: { it: Extract<Item, { t: "flag" }>; leftward: boolean }) {
  const pin = pinLine(it.extra);
  if (pin) return <Pin it={it} pin={pin} />;
  if (it.name === "0") return <path d={`M${it.x - 16} ${it.y} H${it.x + 16} L${it.x} ${it.y + 16} Z`} fill="none" />;
  const [x, , x2] = labelRect(it, leftward), w = x2 - x;
  return (
    <g>
      <circle cx={it.x} cy={it.y} r={2.5} fill="currentColor" />
      {/* (a port, a pin of the schematic's block symbol: pointed at both ends) */}
      {portOf(it.extra) ? (
        <polygon points={`${x},${it.y} ${x + 6},${it.y - 10} ${x + w - 6},${it.y - 10} ${x + w},${it.y} ${x + w - 6},${it.y + 10} ${x + 6},${it.y + 10}`} fill="var(--surface-panel)" fillOpacity={0.85} />
      ) : (
        <rect x={x} y={it.y - 10} width={w} height={20} rx={3} fill="var(--surface-panel)" fillOpacity={0.85} />
      )}
      <text x={x + w / 2} y={it.y} textAnchor="middle" dominantBaseline="central" fill="currentColor" stroke="none" className="font-mono text-[12px]">{it.name}</text>
    </g>
  );
}

const ItemView = memo(function ItemView({ it, lib, leftward }: { it: Shape; lib: Lib; leftward?: boolean }) {
  switch (it.t) {
    case "wire": return <line x1={it.x1} y1={it.y1} x2={it.x2} y2={it.y2} />;
    // (a Wide pen: heavier than the 1.6 of the group around it)
    case "draw": return <g strokeWidth={/^wide$/i.test(it.width) ? 3 : undefined}>{drawEl(it)}</g>;
    case "flag": return <Flag it={it} leftward={!!leftward} />;
    case "text":
      return (
        <text x={it.x} y={it.y} fill="currentColor" stroke="none" className={cn("text-[13px]", it.directive ? "font-mono font-medium" : "italic")}>
          {textLines(it.text).map((l, i) => <tspan key={i} x={it.x} y={it.y + 17 * i}>{l}</tspan>)/* absolute y: some SVG renderers ignore tspan dy */}
        </text>
      );
    case "sym": {
      const def = defOf(lib, it);
      return (
        <g>
          {def ? <use href={`#sym-${def.name}`} transform={matrix(it.rot, it.x, it.y)} />
            : <rect x={it.x - 16} y={it.y - 16} width={32} height={32} strokeDasharray="4 3" />}
          {def && (def.pin_labels || def.pins.some((p) => p.just)) && <PinLabels it={it} def={def} />}
          <g stroke="none"><SymLabels it={it} def={def} /></g>
        </g>
      );
    }
  }
});

const COLOR: Record<Shape["t"], string> = { wire: "var(--sch-wire)", flag: "var(--sch-label)", sym: "var(--sch-symbol)", text: "var(--sch-comment)", draw: "var(--sch-drawing)" };
/** The schematic as it would print: no grid, selection, hover or open-pin marks. */
function ExportSvg({ items, lib }: { items: Item[]; lib: Record<string, SymbolDef> }) {
  const left = leftLabels(items);
  const conn = connectivity(items, lib);
  const defs = [...new Set(items.flatMap((it) => (it.t === "sym" ? [defOf(lib, it)] : [])).filter((d): d is SymbolDef => !!d))];
  return (
    <svg xmlns="http://www.w3.org/2000/svg">
      <defs>{defs.map((d) => <SymbolGraphics key={d.name} def={d} />)}</defs>
      <g data-content="" fill="none" strokeLinecap="round">
        {items.map((it, i) => isShape(it) && (
          <g key={i} color={colorOf(it)} stroke="currentColor" strokeWidth={it.t === "wire" ? 2 : 1.6}>
            <ItemView it={it} lib={lib} leftward={it.t === "flag" && left.has(`${it.x},${it.y}`)} />
          </g>
        ))}
        {conn.dots.map(([x, y]) => <circle key={`d${x},${y}`} cx={x} cy={y} r={4} fill="var(--sch-wire)" />)}
      </g>
    </svg>
  );
}

/** Standalone SVG of the whole schematic in the light theme, cropped to its content. */
export function schematicSvg(items: Item[], lib: Record<string, SymbolDef>): string {
  const host = document.createElement("div");
  host.style.cssText = "position:fixed;left:-100000px;top:0";
  document.body.append(host);
  const root = createRoot(host);
  try {
    flushSync(() => root.render(<ExportSvg items={items} lib={lib} />));
    const svg = host.querySelector("svg")!;
    // <use> → copies of the symbol graphics: not every program follows href into <defs>
    for (const use of [...svg.querySelectorAll("use")]) {
      const ref = svg.querySelector(use.getAttribute("href")!);
      if (!ref) continue;
      const g = ref.cloneNode(true) as SVGGElement;
      g.removeAttribute("id");
      g.setAttribute("transform", use.getAttribute("transform") ?? "");
      use.replaceWith(g);
    }
    svg.querySelector("defs")?.remove();
    return withLightTheme(() => {
      inlineStyles(svg);
      // one <text> per line: macOS's own SVG renderer (Preview, Keynote) ignores tspan positions
      for (const t of [...svg.querySelectorAll("text")].filter((t) => t.querySelectorAll("tspan").length > 1)) {
        for (const span of t.querySelectorAll("tspan")) {
          const line = t.cloneNode(false) as SVGTextElement;
          line.setAttribute("y", span.getAttribute("y")!);
          line.textContent = span.textContent;
          t.before(line);
        }
        t.remove();
      }
      const b = svg.querySelector<SVGGElement>("g[data-content]")!.getBBox(), m = 16;
      svg.setAttribute("viewBox", `${b.x - m} ${b.y - m} ${b.width + 2 * m} ${b.height + 2 * m}`);
      svg.setAttribute("width", String(Math.ceil(b.width + 2 * m)));
      svg.setAttribute("height", String(Math.ceil(b.height + 2 * m)));
      svg.querySelector("g[data-content]")!.removeAttribute("data-content");
      return `<?xml version="1.0" encoding="UTF-8"?>\n${new XMLSerializer().serializeToString(svg)}\n`;
    });
  } finally {
    root.unmount();
    host.remove();
  }
}

const colorOf = (it: Shape) => (it.t === "text" && it.directive ? "var(--sch-directive)" : COLOR[it.t]);

export const Schematic = memo(function Schematic() {
  const { items, lib, sel, tool, nets, sim, fitToken, stale, diff, symbolDoc } = useS(
    useShallow((s) => ({ items: s.items, lib: s.lib, sel: s.sel, tool: s.tool, nets: currentNets(s), sim: s.sim, fitToken: s.fitToken, stale: s.stale, diff: s.diff, symbolDoc: isSymbolDoc(s.path) })),
  );
  // git comparison: items only in the old revision (removed) and only in the current one (added)
  const diffSets = useMemo(() => {
    if (!diff) return null;
    // canonical form: the backend fills defaults (text align, empty extra) and sorts attributes
    const k = (it: Item) => JSON.stringify(
      it.t === "text" ? { ...it, align: it.align || "Left 2" }
      : it.t === "sym" ? { ...it, attrs: Object.fromEntries(Object.entries(it.attrs).sort(([a], [b]) => (a < b ? -1 : 1))), extra: it.extra ?? [] }
      : it.t === "flag" ? { ...it, extra: it.extra ?? [] } : it,
    );
    const now = new Set(items.map(k)), then = new Set(diff.items.map(k));
    return { removed: diff.items.filter((it) => !now.has(k(it))), added: new Set(items.flatMap((it, i) => (then.has(k(it)) ? [] : [i]))) };
  }, [diff, items]);
  const svgRef = useRef<SVGSVGElement>(null);
  const [view, setView] = useState<View>({ x: 120, y: 120, k: 1.5 });
  const [cursor, setCursor] = useState<Pt>([0, 0]);
  const [wireStart, setWireStart] = useState<Pt | null>(null);
  const [hFirst, setHFirst] = useState<boolean | null>(null);
  const [hover, setHover] = useState(-1);
  const [hoverPin, setHoverPin] = useState<number>(); // probing: the part's pin under the pointer
  const pastedAt = useRef({ t: 0, x: 0, y: 0 });
  const [preview, setPreview] = useState<Item[] | null>(null);
  const [boxSel, setBoxSel] = useState<[Pt, Pt] | null>(null);
  const drag = useRef<Drag | null>(null);
  const mods = useRef(false);
  /** True while the view is the automatic fit; then layout changes refit instead of cropping. */
  const autoFit = useRef(true);
  const shown = preview ?? items;
  const selSet = useMemo(() => new Set(sel), [sel]);
  const conn = useMemo(() => connectivity(shown, lib), [shown, lib]);
  // the select tool and the move tool (dragging parts off their wires) select alike
  const selecting = tool.k === "select" || tool.k === "move";
  const [, ended] = useState(0);
  const probing = probeMode(sim) && selecting;
  const defs = useMemo(() => Object.values(lib), [lib]);

  // net of the hovered item (for net highlight + status bar)
  const hoverNet = hover >= 0 && nets && items[hover]?.t !== "sym" ? nets.nets[hover]?.[0] ?? null : null;
  /** The pin of part `it` near `p` (probing: its own current, Ib(Q1)); none: the part's body. */
  const pinAt = (it: Item | undefined, p: Pt) => {
    if (it?.t !== "sym") return undefined;
    // (8 px on screen, but at most a grid step: zoomed out, the body stays the body)
    const tol = Math.min(GRID, Math.max(GRID * 0.5, 8 / view.k));
    let best: number | undefined, d0 = tol * tol;
    pins(it, defOf(lib, it)).forEach(([x, y], k) => {
      const d = (x - p[0]) ** 2 + (y - p[1]) ** 2;
      if (d <= d0) (best = k), (d0 = d);
    });
    return best;
  };
  // probing a part: the status bar names the current a click plots
  const hoverCurrent = probing && hover >= 0 && items[hover]?.t === "sym" && nets ? partCurrent(nets, hover, hoverPin) : "";
  useEffect(() => {
    useStatus.setState({ probe: hoverCurrent || null });
    return () => useStatus.setState({ probe: null });
  }, [hoverCurrent]);

  const size = useSize(svgRef);

  /** `its` in the middle, at the zoom that fits them, at most `cap`; nothing to show: the origin. */
  const frame = (its: Item[], cap = FIT_MAX) => {
    const left = leftLabels(items);
    const b = union(its.map((it) => box(it, lib, left)));
    const { w, h } = size;
    if (!b) return setView({ x: w / 2, y: h / 2, k: 1.5 });
    const pad = Math.min(FIT_PADDING, w / 8, h / 8); // a low canvas (plot + panel open) must not shrink the circuit to nothing
    const k = clampZoom(Math.min((w - 2 * pad) / Math.max(b[2] - b[0], 1), (h - 2 * pad) / Math.max(b[3] - b[1], 1), cap));
    setView({ k, x: w / 2 - ((b[0] + b[2]) / 2) * k, y: h / 2 - ((b[1] + b[3]) / 2) * k });
  };
  const fit = () => {
    autoFit.current = true;
    frame(items);
  };
  // refit on document load, and on layout changes (waveform pane opening) while still auto-fitted
  useEffect(fit, [fitToken, size.w === 0]);
  const prevSize = useRef(size);
  useEffect(() => {
    const p = prevSize.current;
    prevSize.current = size;
    if (!p.w || !size.w) return;
    if (autoFit.current) fit();
    else setView((v) => ({ ...v, x: v.x + (size.w - p.w) / 2, y: v.y + (size.h - p.h) / 2 }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [size]);
  useEffect(() => {
    const onFit = () => fit();
    // (found items: in the middle; zoomed out as far as they need, in to at least 1:1 (a part
    // found while zoomed far out is no dot), never further than a fit)
    const onShow = (e: Event) => {
      const its = ((e as CustomEvent<number[]>).detail ?? []).map((i) => items[i]).filter(Boolean);
      if (!its.length || !size.w || !size.h) return;
      autoFit.current = false;
      frame(its, Math.max(view.k, 1)); // (zoomed in further already: kept, if they fit)
    };
    // (the menu's Zoom In / Out: about the middle, as the corner's buttons)
    const onZoom = (e: Event) => zoomAt(view.k * ZOOM_STEP ** (e as CustomEvent<number>).detail, size.w / 2, size.h / 2);
    window.addEventListener("nomospice:fit", onFit);
    window.addEventListener("nomospice:show", onShow);
    window.addEventListener("nomospice:zoom", onZoom);
    return () => (window.removeEventListener("nomospice:fit", onFit), window.removeEventListener("nomospice:show", onShow), window.removeEventListener("nomospice:zoom", onZoom));
  });
  useLayoutEffect(() => {
    if (tool.k !== "wire") setWireStart(null);
    // the snapped cursor is not tracked in select mode: start the ghost where the mouse is (before paint)
    const { x, y } = useStatus.getState();
    setCursor([x, y]);
  }, [tool.k]);
  useEffect(() => {
    let op = "";
    // stale results: auto net names (N001 ...) may now denote other nodes, so no value
    // (a .step run: the net's value per step, `out [R=1k]`, in order)
    const own = (n: string) => !!hoverNet && (n.toLowerCase() === hoverNet.toLowerCase() || n.toLowerCase().startsWith(`${hoverNet.toLowerCase()} [`));
    const vs = hoverNet && !stale && sim?.run?.plot.startsWith("op") ? sim.run.op.filter(([n]) => own(n)) : [];
    if (vs.length) op = ` = ${vs.map((v) => v[1].toPrecision(5)).join(" / ")} V`;
    useStatus.setState({ net: hoverNet ? hoverNet + op : null, zoom: view.k });
  }, [hoverNet, view.k, sim, stale]);
  useEffect(() => {
    const cancel = () => (wireStart ? setWireStart(null) : useS.setState({ tool: { k: "select" } }));
    window.addEventListener("nomospice:cancel", cancel);
    return () => window.removeEventListener("nomospice:cancel", cancel);
  }, [wireStart]);

  const centerAt = useCallback(([x, y]: Pt) => {
    autoFit.current = false;
    setView((v) => ({ ...v, x: size.w / 2 - x * v.k, y: size.h / 2 - y * v.k }));
  }, [size]);

  const toWorld = (e: { clientX: number; clientY: number }): Pt => {
    const r = svgRef.current!.getBoundingClientRect();
    return [(e.clientX - r.left - view.x) / view.k, (e.clientY - r.top - view.y) / view.k];
  };
  const zoomAt = (k: number, sx: number, sy: number) => {
    autoFit.current = false;
    setView((v) => {
      const nk = clampZoom(k);
      const fx = (sx - v.x) / v.k, fy = (sy - v.y) / v.k;
      return { k: nk, x: sx - fx * nk, y: sy - fy * nk };
    });
  };

  const placed = (it: Item, [x, y]: Pt): Item => {
    if (it.t === "wire" || it.t === "raw") return it;
    if (it.t === "draw") return ((b) => shifted(it, x - snap(b[0]), y - snap(b[1])))(box(it, lib)!); // (what is drawn at the cursor, as a paste, by whole grid steps: the core keeps integers)
    if (it.t === "sym" && !it.attrs.InstName) {
      const def = defOf(lib, it);
      return { ...it, x, y, attrs: { InstName: nextInst(items, prefixOf(it, def) || "X"), ...it.attrs } };
    }
    return { ...it, x, y };
  };
  /** Pasted items at `at`: a part whose name is taken (or pasted twice) renumbered, as LTspice does
   *  (the preview shows the names the click gives). */
  const pasted = (its: Shape[]): Shape[] => {
    const out: Shape[] = [], taken = new Set(items.flatMap((it) => (it.t === "sym" && it.attrs.InstName ? [it.attrs.InstName.toUpperCase()] : [])));
    // (an analysis pasted next to the schematic's own: an alternative, as "+" on the result tabs adds them)
    const runs = (it: Item) => it.t === "text" && it.directive && it.text.split("\n").some((l) => ANY_ANALYSIS.test(l.trim()));
    let active = items.some(runs); // (also once one pasted here stays active: the others become alternatives)
    for (const it of its) {
      let s = it;
      if (s.t === "text" && s.directive && active) {
        // its analysis lines that read as an alternative (the rule result tabs use): a comment of their
        // own below what stays active (a .param beside them); others stay as they are
        const lines = s.text.split("\n"), alt = new Set<number>();
        lines.forEach((l, i) => ANALYSIS.test(l.trim()) && analysisOptions([{ text: [i, ...continuation(lines, i)].map((j) => lines[j]).join("\n"), directive: false }]).length && [i, ...continuation(lines, i)].forEach((j) => alt.add(j)));
        if (alt.size) {
          const rest = lines.filter((_, j) => !alt.has(j));
          if (rest.some((l) => l.trim())) out.push({ ...s, text: rest.join("\n") });
          out.push({ ...s, y: s.y + (rest.some((l) => l.trim()) ? 32 * rest.length : 0), text: lines.filter((_, j) => alt.has(j)).join("\n"), directive: false });
          continue;
        }
      }
      if (runs(s)) active = true;
      if (s.t === "sym") {
        // (its symbol not loaded yet: the prefix its own name has, R1 -> R2, not U1)
        const prefix = prefixOf(s, defOf(lib, s)) || /^[a-z]/i.exec(s.attrs.InstName ?? "")?.[0].toUpperCase() || "X";
        // (numbered past the pasted parts' own names too: R1 R2 into a document with R1 -> R3 R2)
        if (!s.attrs.InstName || taken.has(s.attrs.InstName.toUpperCase())) s = { ...s, attrs: { ...s.attrs, InstName: nextInst([...items, ...its, ...out], prefix) } };
        taken.add(s.attrs.InstName.toUpperCase());
      }
      out.push(s);
    }
    return out;
  };
  // named once per paste (not per mouse move): the preview follows the cursor by shifting these
  const pasteNamed = useMemo(() => (tool.k === "paste" ? pasted(tool.items) : []), [tool, items, lib]); // eslint-disable-line react-hooks/exhaustive-deps
  const wirePath = (a: Pt, b: Pt): Item[] => {
    // (a symbol's line: a drawing, straight at any angle)
    if (symbolDoc) return a[0] === b[0] && a[1] === b[1] ? [] : [{ t: "draw", kind: "LINE", width: "Normal", p: [a[0], a[1], b[0], b[1]] }];
    const corner: Pt = hFirst ?? true ? [b[0], a[1]] : [a[0], b[1]];
    return dropZeroWires([
      { t: "wire", x1: a[0], y1: a[1], x2: corner[0], y2: corner[1] },
      { t: "wire", x1: corner[0], y1: corner[1], x2: b[0], y2: b[1] },
    ]);
  };

  const onPointerDown = (e: React.PointerEvent) => {
    window.getSelection()?.removeAllRanges(); // back on the canvas: ⌘A/Delete act on the schematic again
    mods.current = e.shiftKey || e.metaKey || e.ctrlKey || e.altKey;
    const p = toWorld(e), g: Pt = [snap(p[0]), snap(p[1])];
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    if (e.button === 1 || (e.button === 0 && e.altKey)) {
      autoFit.current = false;
      drag.current = { k: "pan", sx: e.clientX, sy: e.clientY, vx: view.x, vy: view.y, moved: true };
      return;
    }
    if (e.button !== 0) return;
    if (tool.k === "wire") {
      if (!wireStart) return setWireStart(g), setHFirst(null);
      const ends = g[0] !== wireStart[0] || g[1] !== wireStart[1];
      // (a symbol's line ends on another's end or a pin: a closed outline is done)
      const at = (x: number, y: number) => x === g[0] && y === g[1];
      const done = ends && (symbolDoc ? items.some((it) => (it.t === "draw" && it.kind === "LINE" && (at(it.p[0], it.p[1]) || at(it.p[2], it.p[3]))) || (it.t === "flag" && at(it.x, it.y))) : conn.isConn(g));
      if (ends) commit([...items, ...wirePath(wireStart, g)]);
      setWireStart(done ? null : g);
      setHFirst(null);
    } else if (tool.k === "place") {
      commit([...items, placed(tool.item, g)]);
    } else if (tool.k === "paste") {
      const out = pasteNamed.map((it) => shifted(it, g[0], g[1])); // (here, selected)
      commit([...items, ...out], out.map((_, k) => items.length + k));
      useS.setState({ tool: { k: "select" } });
      pastedAt.current = { t: Date.now(), x: e.clientX, y: e.clientY };
    } else if (tool.k === "delete") {
      const i = hit(items, lib, p, 6 / view.k);
      if (i >= 0) commit(items.filter((_, j) => j !== i), []), setHover(-1);
    } else {
      const i = hit(items, lib, p, 6 / view.k, !probing); // (probing: a drawing is no target; its edge plots or pans)
      if (i >= 0) {
        let next = sel;
        if (e.shiftKey || e.metaKey || e.ctrlKey) next = selSet.has(i) ? sel.filter((j) => j !== i) : [...sel, i];
        else if (!selSet.has(i)) next = [i];
        useS.setState({ sel: next });
        // the plan is built on the first move (a click or a probe never needs it); the move tool:
        // the selection alone, its wires left where they are
        let built: ReturnType<typeof dragPlan> | null = null;
        const chosen = new Set(next);
        // (a symbol's lines are no wires: what is moved moves alone)
        const plan = tool.k === "move"
          ? (dx: number, dy: number) => items.map((it, j) => (chosen.has(j) && isShape(it) ? shifted(it, dx, dy) : it))
          : (dx: number, dy: number) => (built ??= dragPlan(items, lib, chosen))(dx, dy);
        drag.current = { k: "move", start: g, plan, moved: false, index: i, at: [0, 0], base: items, pin: probing ? pinAt(items[i], p) : undefined };
      } else if (e.shiftKey) {
        drag.current = { k: "box", start: p, end: p };
      } else {
        drag.current = { k: "pan", sx: e.clientX, sy: e.clientY, vx: view.x, vy: view.y, moved: false };
      }
    }
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const p = toWorld(e), g: Pt = [snap(p[0]), snap(p[1])];
    if (g[0] !== useStatus.getState().x || g[1] !== useStatus.getState().y) useStatus.setState({ x: g[0], y: g[1] });
    // the snapped cursor is state only while a ghost / wire preview follows it (select mode: no re-render)
    if (!selecting && (g[0] !== cursor[0] || g[1] !== cursor[1])) setCursor(g);
    if (wireStart && hFirst === null && (g[0] !== wireStart[0] || g[1] !== wireStart[1]))
      setHFirst(Math.abs(g[0] - wireStart[0]) >= Math.abs(g[1] - wireStart[1]));
    const d = drag.current;
    if (!d) {
      const h = tool.k === "place" || tool.k === "paste" || tool.k === "wire" ? -1 : hit(items, lib, p, 6 / view.k, !probing);
      setHover(h);
      return setHoverPin(probing && h >= 0 ? pinAt(items[h], p) : undefined);
    }
    if (d.k === "pan") {
      if (Math.hypot(e.clientX - d.sx, e.clientY - d.sy) > 3) d.moved = true; // SwiftFlow minimumDistance: 3
      if (d.moved) autoFit.current = false, setView((v) => ({ ...v, x: d.vx + e.clientX - d.sx, y: d.vy + e.clientY - d.sy }));
    } else if (d.k === "move") {
      const dx = g[0] - d.start[0], dy = g[1] - d.start[1];
      if (dx || dy) d.moved = true;
      d.at = [dx, dy];
      if (d.moved) setPreview(d.plan(dx, dy));
    } else (d.end = p), setBoxSel([d.start, p]);
  };

  const onPointerUp = () => {
    const d = drag.current;
    drag.current = null;
    if (d?.k === "pan") ended((n) => n + 1); // (the grabbing hand comes from the ref: a render puts it back)
    if (d?.k === "move") {
      // dragged back to the start: nothing changed; edited by keyboard meanwhile (⌃R, Delete, ⌘Z):
      // that edit wins, the drag (planned on the old items) is dropped
      if (d.moved && (d.at[0] || d.at[1]) && useS.getState().items === d.base) {
        // from the plan, not `preview` state: the last pointermove may not have rendered yet
        const next = dropZeroWiresKeepSel(d.plan(d.at[0], d.at[1]), sel);
        commit(next.items, next.sel);
      }
      // LTspice: a plain click probes at once; modifier clicks only change the selection
      else if (!d.moved && probing && !mods.current && useS.getState().items === d.base) probe(d.base[d.index], false, d.pin);
    } else if (d?.k === "pan" && !d.moved) useS.setState({ sel: [] });
    else if (d?.k === "box") {
      const [a, b] = [d.start, d.end]; // from the ref: the last pointermove may not have rendered
      const r: Box = [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])];
      // SwiftFlow selection mode "partial": anything the rectangle touches; shift adds to the selection,
      // and a shift-click that did not drag keeps it untouched
      if (r[2] - r[0] > 3 / view.k || r[3] - r[1] > 3 / view.k) {
        const left = leftLabels(items);
        // (a drawing only when wholly inside: a band around parts within a frame does not take the frame)
        const within = (q: Box) => q[0] >= r[0] && q[2] <= r[2] && q[1] >= r[1] && q[3] <= r[3];
        const hits = items.flatMap((it, i) => { const q = box(it, lib, left); return q && (it.t === "draw" ? within(q) : q[0] <= r[2] && q[2] >= r[0] && q[1] <= r[3] && q[3] >= r[1]) ? [i] : []; });
        useS.setState({ sel: [...new Set([...sel, ...hits])] });
      }
    }
    setPreview(null);
    setBoxSel(null);
  };

  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    if (tool.k === "wire" && wireStart) return setWireStart(null);
    if (!selecting) return useS.setState({ tool: { k: "select" } });
    const p = toWorld(e), g: Pt = [snap(p[0]), snap(p[1])];
    const i = hit(items, lib, p, 6 / view.k, !probing); // (as hover and click: no drawing while probing)
    // right-click acts on what is under the pointer: it joins/sets the selection first
    if (i >= 0 && !selSet.has(i)) useS.setState({ sel: [i] });
    openMenu(e.clientX, e.clientY, menuFor(i, g, i >= 0 && selSet.has(i) ? sel.length : 1));
  };

  const menuFor = (i: number, g: Pt, count: number): MenuItem[] => {
    const it = items[i];
    const set = useS.setState;
    const net = i >= 0 ? nets?.nets[i]?.[0] : undefined;
    const selectNet = () => set({ sel: items.flatMap((x, j) => ((x.t === "wire" || x.t === "flag") && nets?.nets[j]?.[0] === net ? [j] : [])) });
    const edit = () => set({ sel: [i], dialog: { k: "edit", index: i } });
    // a part: its current, and each pin's where it has its own (a transistor's Ib, a subcircuit's pins)
    const own = probing && it?.t === "sym" && nets ? partCurrent(nets, i) : "";
    const plot: MenuItem[] = !probing || !it ? [] : it.t !== "sym" ? (canProbe(it) ? [{ label: `Plot V(${net ?? "?"})`, run: () => probe(it) }] : [])
      : !nets ? (canProbe(it) ? [{ label: "Plot its current", run: () => probe(it) }] : []) // (no fresh netlist yet: the probe re-runs first)
      : [...(own ? [{ label: `Plot ${own}`, run: () => probe(it) }] : []),
        // (a big IC: its first pins; any pin by clicking near it)
        ...(nets?.pin_currents?.[i] ?? []).flatMap((c, k) => (c && c !== own ? [{ label: `Plot ${c}`, run: () => probe(it, false, k) }] : [])).slice(0, 8)];
    // two-pin parts: their power too (LTspice: alt-click)
    if (own && powerMode(sim) && it?.t === "sym" && nets?.nets[i]?.length === 2) plot.push({ label: `Plot power ${powerName(own)}`, run: () => probe(it, true) });
    const arrange: MenuItem[] = [
      { label: "Rotate", hint: "⌃R", run: () => transform(rotate) },
      { label: "Mirror", hint: "⌃E", run: () => transform(mirror) },
    ];
    const del: MenuItem = { label: count > 1 ? `Delete ${count} items` : "Delete", hint: "⌫", danger: true, run: deleteSelection };
    const clip: MenuItem[] = [{ label: "Copy", hint: "⌘C", run: () => copySelection() }, { label: "Cut", hint: "⌘X", run: () => copySelection(true) }];
    const back: MenuItem[] = useS.getState().parents.length ? [{ label: "Back to the schematic above", run: () => window.dispatchEvent(new Event("nomospice:up")) }] : [];
    if (!it && symbolDoc) return [
      { label: "Line", hint: "W", run: () => set({ tool: { k: "wire" } }) },
      { label: "Pin here…", hint: "N", run: () => newItem({ t: "flag", x: 0, y: 0, name: "" }, g) },
      { label: "Text here…", hint: "T", run: () => newItem({ t: "text", x: 0, y: 0, text: "", directive: false }, g) },
      "-",
      { label: "Symbol attributes…", run: () => set({ dialog: { k: "symattrs" } }) },
      "-",
      { label: "Paste", hint: "⌘V", run: pasteClipboard },
      { label: "Select all", hint: "⌘A", run: () => set({ sel: items.flatMap((x, j) => (x.t === "raw" ? [] : [j])) }) },
      { label: "Fit view", hint: "Space", run: fit },
      ...back,
    ];
    if (!it) return [
      { label: "Place component…", hint: "P", run: () => set({ dialog: { k: "parts" } }) },
      { label: "Wire", hint: "W", run: () => set({ tool: { k: "wire" } }) },
      { label: "Net label here…", hint: "N", run: () => newItem({ t: "flag", x: 0, y: 0, name: "" }, g) },
      { label: "Ground here", hint: "G", run: () => commit([...items, { t: "flag", x: g[0], y: g[1], name: "0" }]) },
      { label: "Comment here…", hint: "T", run: () => newItem({ t: "text", x: 0, y: 0, text: "", directive: false }, g) },
      { label: "SPICE directive here…", hint: "S", run: () => newItem({ t: "text", x: 0, y: 0, text: "", directive: true }, g) },
      "-",
      { label: "Edit simulation command…", run: () => set({ dialog: { k: "sim" } }) },
      { label: "Run", hint: "⌘↵", run: simulate },
      "-",
      { label: "Paste", hint: "⌘V", run: pasteClipboard },
      { label: "Select all", hint: "⌘A", run: () => set({ sel: items.flatMap((x, j) => (x.t === "raw" ? [] : [j])) }) },
      { label: "Find…", hint: "⌘F", run: () => set({ dialog: { k: "find" } }) },
      { label: "Fit view", hint: "Space", run: fit },
      "-",
      { label: "Export schematic as SVG…", disabled: !items.some(isShape), run: () => void saveSvg(schematicSvg(items, lib), "") },
      // (hierarchy: this schematic as a block of others in its folder; back to the one it was opened from)
      { label: "Create block symbol…", run: () => window.dispatchEvent(new Event("nomospice:block-symbol")) },
      { label: "New symbol", run: () => window.dispatchEvent(new Event("nomospice:new-symbol")) },
      ...back,
    ];
    if (count > 1) return [...(symbolDoc ? [] : [...arrange, "-" as const]), ...clip, del]; // (a symbol's drawing is not rotated)
    switch (it.t) {
      case "sym": {
        const prefix = prefixOf(it, defOf(lib, it)); // the part's own Prefix wins, as in the netlist
        const kind = ({ D: "D", Q: "NPN|PNP", M: "NMOS|PMOS|VDMOS", J: "NJF|PJF", X: "SUBCKT" } as Record<string, string>)[prefix];
        // its schematic opens (to fix one the netlist refuses, too), unless it is this one (a test
        // bench of its part); a model is chosen for a part the netlist runs as no block
        const { sheet, file } = defOf(lib, it) ?? {}, here = sheet === useS.getState().path;
        const block = nets ? !!nets.sheets?.[i] : !!sheet && !here;
        return [
          { label: "Edit…", hint: probing ? undefined : "dbl-click", run: edit },
          ...(sheet && !here ? [{ label: "Open its schematic", run: () => window.dispatchEvent(new CustomEvent("nomospice:sheet", { detail: sheet })) }] : []),
          // (a symbol of the schematic's folder: drawn there, edited there)
          ...(file ? [{ label: "Edit its symbol", run: () => window.dispatchEvent(new CustomEvent("nomospice:sheet", { detail: file })) }] : []),
          ...(kind && !block ? [{ label: "Choose model from library…", run: () => set({ dialog: { k: "parts", replace: i, kind } }) }] : []),
          ...plot,
          "-", ...arrange,
          { label: "Duplicate", hint: "⌘D", run: duplicate },
          "-", ...clip, del,
        ];
      }
      case "wire":
        return [...plot, { label: "Net label here…", run: () => newItem({ t: "flag", x: 0, y: 0, name: "" }, g) }, { label: "Select net", disabled: !net, run: selectNet }, "-", ...clip, del];
      case "flag":
        return [{ label: it.name === "0" && !symbolDoc ? "Ground" : "Rename…", disabled: it.name === "0" && !symbolDoc, run: edit }, ...plot, { label: "Select net", disabled: !net, run: selectNet }, "-", ...(it.name === "0" || symbolDoc ? [] : [{ label: "Make ground", run: () => commit(items.map((x) => (x === it ? { ...it, name: "0" } : x))) }]), ...clip, del];
      case "draw":
        return [{ label: "Duplicate", hint: "⌘D", run: duplicate }, "-", ...clip, del]; // (not rotated: parts only)
      case "text":
        return [
          { label: "Edit…", run: edit },
          ...(symbolDoc ? [] : [{ label: it.directive ? "Turn into comment" : "Turn into SPICE directive", run: () => commit(items.map((x) => (x === it ? { ...it, directive: !it.directive } : x))) }]),
          "-", ...clip, del,
        ];
      default:
        return [];
    }
  };


  /** Open the edit dialog for the item under the pointer (wires have nothing to edit). */
  const editAt = (e: React.MouseEvent) => {
    const i = hit(items, lib, toWorld(e), 6 / view.k);
    if (i >= 0 && items[i].t !== "wire" && items[i].t !== "draw") useS.setState({ sel: [i], dialog: { k: "edit", index: i } }); // (a drawing: nothing to edit)
  };

  const onDoubleClick = (e: React.MouseEvent) => {
    if (tool.k === "wire") return setWireStart(null);
    // (the click that placed a paste, doubled — at its spot, within any double-click time: not an edit)
    const pa = pastedAt.current;
    if (Date.now() - pa.t < 2000 && Math.hypot(e.clientX - pa.x, e.clientY - pa.y) < 6) return;
    // while probing, clicks plot (LTspice); editing is right-click there
    if (!selecting || probing) return;
    editAt(e);
  };

  useEffect(() => {
    const el = svgRef.current!;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el.getBoundingClientRect();
      // OpenPCB camera: trackpad scroll pans, pinch (ctrl) and mouse wheel zoom to cursor
      const trackpad = !e.ctrlKey && e.deltaMode === 0 && (e.deltaX !== 0 || !Number.isInteger(e.deltaY) || Math.abs(e.deltaY) < 16);
      autoFit.current = false;
      if (trackpad) setView((v) => ({ ...v, x: v.x - e.deltaX, y: v.y - e.deltaY }));
      else setView((v) => {
        const nk = clampZoom(v.k * Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015) * (e.deltaMode === 1 ? 16 : 1)));
        const sx = e.clientX - r.left, sy = e.clientY - r.top;
        return { k: nk, x: sx - ((sx - v.x) / v.k) * nk, y: sy - ((sy - v.y) / v.k) * nk };
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // labels whose wire leaves to the right are drawn to the left of their anchor
  const leftward = useMemo(() => leftLabels(shown), [shown]);
  const hoverIsSym = hover >= 0 && items[hover]?.t === "sym";
  const hoverProbe = probing && hover >= 0 && canProbe(items[hover], hoverPin);
  const cursorStyle =
    tool.k === "delete" ? "not-allowed"
    : !selecting ? "crosshair"
    : drag.current?.k === "pan" ? "grabbing"
    : hoverProbe ? (hoverIsSym ? PROBE_I : PROBE_V)
    : hover >= 0 ? (tool.k === "move" ? "move" : "pointer") : "default";
  const gap = GRID * view.k;
  const ghosts: Shape[] = tool.k === "paste" ? pasteNamed.map((it) => shifted(it, cursor[0], cursor[1])) : tool.k === "place" ? [placed(tool.item, cursor)].filter(isShape) : [];

  return (
    <div className="relative h-full w-full overflow-hidden bg-surface-schematic-canvas">
      <svg
        ref={svgRef}
        className="h-full w-full select-none touch-none"
        style={{ cursor: cursorStyle }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerLeave={() => setHover(-1)}
        onContextMenu={onContextMenu}
        onDoubleClick={onDoubleClick}
      >
        <defs>
          {/* SwiftFlow Background "dots": hidden when spacing <= 2px */}
          <pattern id="grid" width={gap} height={gap} patternUnits="userSpaceOnUse" x={view.x % gap} y={view.y % gap}>
            <circle cx={0.75} cy={0.75} r={Math.min(1.2, 0.6 * view.k)} fill="var(--sch-grid)" />
          </pattern>
          {defs.map((d) => <SymbolGraphics key={d.name} def={d} />)}
        </defs>
        {gap > 4 && <rect width="100%" height="100%" fill="url(#grid)" />}
        <g transform={`translate(${view.x} ${view.y}) scale(${view.k})`} fill="none" strokeLinecap="round">
          {shown.map((it, i) => {
            if (!isShape(it)) return null;
            const selected = selSet.has(i);
            const netHi = probing && hoverNet && it.t === "wire" && nets?.nets[i]?.[0] === hoverNet;
            return (
              <g
                key={i}
                // (a symbol's drawing and pins: as a part's are)
                color={selected ? "var(--selection)" : diffSets?.added.has(i) ? "var(--series-3)" : symbolDoc && (it.t === "draw" || it.t === "flag") ? "var(--sch-symbol)" : colorOf(it)}
                stroke="currentColor"
                strokeWidth={it.t === "wire" ? (netHi ? 4 : 2) : 1.6}
                style={{ vectorEffect: "non-scaling-stroke", opacity: i === hover && !selected ? 0.75 : 1 }}
                className="[&_*]:[vector-effect:non-scaling-stroke]"
              >
                <ItemView it={it} lib={lib} leftward={it.t === "flag" && leftward.has(`${it.x},${it.y}`)} />
              </g>
            );
          })}
          {diffSets?.removed.map((it, j) => isShape(it) && (
            <g key={`rm${j}`} color="var(--series-8)" stroke="currentColor" strokeWidth={1.6} strokeDasharray="5 4" opacity={0.75} className="[&_*]:[vector-effect:non-scaling-stroke]">
              <ItemView it={it} lib={lib} />
            </g>
          ))}
          {/* (a symbol's origin: where it is placed) */}
          {conn.dots.map(([x, y]) => <circle key={`d${x},${y}`} cx={x} cy={y} r={4} fill="var(--sch-wire)" />)}
          {symbolDoc && <path d="M-12 0H12M0 -12V12" stroke="var(--text-tertiary)" strokeWidth={1} vectorEffect="non-scaling-stroke" />}
          {conn.open.map(([x, y]) => <rect key={`o${x},${y}`} x={x - 3} y={y - 3} width={6} height={6} stroke="var(--sch-pin)" strokeWidth={1} vectorEffect="non-scaling-stroke" />)}
          {!!ghosts.length && (
            <g color="var(--selection)" stroke="currentColor" strokeWidth={1.6} opacity={0.7} className="[&_*]:[vector-effect:non-scaling-stroke]">
              {ghosts.map((it, k) => <ItemView key={k} it={it} lib={lib} />)}
            </g>
          )}
          {wireStart && (
            <g stroke="var(--selection)" strokeWidth={2} strokeDasharray="8 6" opacity={0.8} className="[&_*]:[vector-effect:non-scaling-stroke]">
              {wirePath(wireStart, cursor).map((w, i) => (w.t === "wire" ? <line key={i} x1={w.x1} y1={w.y1} x2={w.x2} y2={w.y2} /> : w.t === "draw" && <line key={i} x1={w.p[0]} y1={w.p[1]} x2={w.p[2]} y2={w.p[3]} />))}
            </g>
          )}
          {!selecting && (
            <circle cx={cursor[0]} cy={cursor[1]} r={3} fill="none" stroke="var(--selection)" strokeWidth={1} vectorEffect="non-scaling-stroke" />
          )}
          {boxSel && (
            <rect
              x={Math.min(boxSel[0][0], boxSel[1][0])} y={Math.min(boxSel[0][1], boxSel[1][1])}
              width={Math.abs(boxSel[1][0] - boxSel[0][0])} height={Math.abs(boxSel[1][1] - boxSel[0][1])}
              fill="var(--selection-soft)" stroke="var(--selection)" strokeWidth={1} vectorEffect="non-scaling-stroke"
            />
          )}
        </g>
      </svg>
      {stale && sim && (
        <div className="pointer-events-none absolute top-2 left-1/2 -translate-x-1/2 rounded-float border border-status-warning-border bg-status-warning-soft px-2 py-0.5 text-2xs text-status-warning">
          Schematic changed since last run
        </div>
      )}
      <Controls view={view} onZoom={(f) => zoomAt(view.k * f, size.w / 2, size.h / 2)} onReset={() => zoomAt(1, size.w / 2, size.h / 2)} onFit={fit} />
      <MiniMap items={items} lib={lib} view={view} size={size} onCenter={centerAt} />
    </div>
  );
});

/** SwiftFlow Controls: + / zoom% / − / fit, bottom-left, 12px margin. */
function Controls({ view, onZoom, onReset, onFit }: { view: View; onZoom: (f: number) => void; onReset: () => void; onFit: () => void }) {
  const btn = "flex h-6 w-8 items-center justify-center text-text-secondary hover:bg-surface-hover hover:text-text-strong [&_svg]:h-3.5 [&_svg]:w-3.5";
  return (
    <div role="group" aria-label="Graph controls" className="absolute bottom-3 left-3 flex flex-col divide-y divide-border rounded-float border border-border bg-surface-panel/90 shadow-float backdrop-blur">
      <button className={btn} aria-label="Zoom in" title="Zoom in" onClick={() => onZoom(ZOOM_STEP)}><Plus /></button>
      <button className={cn(btn, "font-mono text-2xs")} title="Reset to 100%" onClick={onReset}>{Math.round(view.k * 100)}%</button>
      <button className={btn} aria-label="Zoom out" title="Zoom out" onClick={() => onZoom(1 / ZOOM_STEP)}><Minus /></button>
      <button className={btn} aria-label="Fit view" title="Fit view (Space)" onClick={onFit}><Maximize /></button>
    </div>
  );
}

/** SwiftFlow MiniMap: 150×100, bottom-right; maps items ∪ viewport; click/drag recentres. */
const MiniMap = memo(function MiniMap({ items, lib, view, size, onCenter }: { items: Item[]; lib: Lib; view: View; size: { w: number; h: number }; onCenter: (p: Pt) => void }) {
  const ref = useRef<SVGSVGElement>(null);
  const vis: Box = [-view.x / view.k, -view.y / view.k, (size.w - view.x) / view.k, (size.h - view.y) / view.k];
  const boxes = useMemo(() => { const left = leftLabels(items); return items.map((it) => box(it, lib, left)); }, [items, lib]);
  const all = union([...boxes, vis])!;
  const pad = Math.max(all[2] - all[0], all[3] - all[1]) * 0.05;
  const vb = [all[0] - pad, all[1] - pad, all[2] - all[0] + 2 * pad, all[3] - all[1] + 2 * pad];
  const center = (e: React.PointerEvent) => {
    if (!(e.buttons & 1)) return;
    const m = ref.current!.getScreenCTM()!.inverse();
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m);
    onCenter([p.x, p.y]);
  };
  if (!items.some(isShape)) return null;
  return (
    <svg
      ref={ref}
      aria-label="Graph minimap"
      viewBox={vb.join(" ")}
      className="absolute right-3 bottom-3 h-[100px] w-[150px] cursor-pointer rounded-float border border-border bg-surface-panel/90 shadow-float"
      onPointerDown={(e) => { (e.target as Element).setPointerCapture(e.pointerId); center(e); }}
      onPointerMove={center}
    >
      {items.map((it, i) => {
        if (it.t === "wire") return <line key={i} x1={it.x1} y1={it.y1} x2={it.x2} y2={it.y2} stroke="var(--sch-wire)" strokeOpacity={0.5} strokeWidth={1} vectorEffect="non-scaling-stroke" />;
        if (it.t === "draw") return <g key={i} stroke="var(--sch-drawing)" strokeOpacity={0.5} strokeWidth={1} fill="none">{drawEl(it)}</g>; // (a frame: an outline, not a block over the circuit)
        const b = boxes[i];
        return b && <rect key={i} x={b[0]} y={b[1]} width={Math.max(b[2] - b[0], 4)} height={Math.max(b[3] - b[1], 3)} rx={1} fill="var(--text-tertiary)" fillOpacity={0.5} />;
      })}
      <path
        fillRule="evenodd" fill="var(--text-strong)" fillOpacity={0.06}
        d={`M${vb[0]} ${vb[1]}h${vb[2]}v${vb[3]}h${-vb[2]}Z M${vis[0]} ${vis[1]}H${vis[2]}V${vis[3]}H${vis[0]}Z`}
      />
      <rect x={vis[0]} y={vis[1]} width={vis[2] - vis[0]} height={vis[3] - vis[1]} fill="none" stroke="var(--selection)" strokeOpacity={0.6} strokeWidth={1.5} rx={2} vectorEffect="non-scaling-stroke" />
    </svg>
  );
});
