import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { GitStatus, Item, Netlist, Sim, SymbolDef, Trace, TraceData } from "./types";
import { currentLabel, defOf, symKey, vexpr, type Lib } from "./geom";

export type Tool =
  | { k: "select" }
  | { k: "wire" }
  | { k: "place"; item: Item }
  | { k: "delete" };

export type Dialog =
  /** raw: a directive as plain text, even when a form could edit it */
  | { k: "edit"; index: number; raw?: boolean; draft?: string }
  | { k: "new"; item: Item; at?: [number, number] }
  | { k: "sim" }
  /** `replace`: choose a model for that part instead of placing a new one */
  | { k: "parts"; replace?: number; kind?: string; libraries?: boolean }
  | { k: "trace" }
  | { k: "about" };

interface State {
  items: Item[];
  past: Item[][];
  future: Item[][];
  sel: number[];
  tool: Tool;
  lib: Lib;
  path: string | null;
  dirty: boolean;
  /** Live netlist (connectivity, net names, warnings), computed for `netsItems`. */
  nets: Netlist | null;
  netsItems: Item[] | null;
  /** Last simulation; `sim.netlist` is aligned with the items it ran on. */
  sim: Sim | null;
  running: boolean;
  /** Items changed since `sim` was computed. */
  stale: boolean;
  traces: Trace[];
  live: boolean;
  panel: "log" | "netlist" | "op" | "history" | null;
  dialog: Dialog | null;
  fitToken: number;
  /** Waveform x-zoom and cursors A/B; reset per document and analysis kind. */
  plot: PlotView;
  menu: { x: number; y: number; items: import("./Menu").MenuItem[] } | null;
  git: GitStatus | null;
  /** visual diff: the schematic of an older commit, overlaid on the current one */
  diff: { hash: string; items: Item[] } | null;
}

/** `kind`: the analysis these belong to; a run of another kind starts fresh. */
export interface PlotView { kind: string; zoom: [number, number] | null; a: number | null; b: number | null }
const NO_PLOT: PlotView = { kind: "", zoom: null, a: null, b: null };
export const setPlot = (p: Partial<PlotView>) => useS.setState({ plot: { ...useS.getState().plot, ...p } });
/** "tran:time", "ac:frequency" ... */
export const plotKind = (sim: Sim | null) => `${sim?.run?.plot.replace(/\d+$/, "") ?? ""}:${sim?.run?.scale ?? ""}`;

export const useS = create<State>(() => ({
  items: [], past: [], future: [], sel: [], tool: { k: "select" }, lib: {}, path: null, dirty: false,
  nets: null, netsItems: null, sim: null, running: false, stale: false, traces: [], live: false, panel: null, dialog: null,
  fitToken: 0, plot: NO_PLOT, menu: null, git: null, diff: null,
}));
const set = useS.setState;
const get = useS.getState;

/** Cursor/hover readout: a separate store so pointer moves only re-render the status bar. */
export const useStatus = create(() => ({ x: 0, y: 0, net: null as string | null, zoom: 1 }));

// ---------------------------------------------------------------- document

export const dirOf = (path: string | null) => (path ? path.replace(/[\\/][^\\/]*$/, "") : null);
/** `p` relative to the open schematic's folder when inside it (as LTspice writes includes). */
export const relToDoc = (p: string) => {
  const dir = dirOf(useS.getState().path);
  if (!dir) return p;
  // Windows paths compare without case (a dialog may return c:\ for C:\); slashes: portablePaths at commit
  const [d, q] = /^[a-z]:/i.test(dir) ? [dir.toLowerCase(), p.toLowerCase()] : [dir, p];
  return q.startsWith(`${d}/`) || q.startsWith(`${d}\\`) ? p.slice(dir.length + 1) : p;
};

/** Live netlist only if it belongs to the current items (it lags edits by a debounce + IPC). */
export const currentNets = (s: { nets: Netlist | null; netsItems: Item[] | null; items: Item[] }) =>
  s.netsItems === s.items ? s.nets : null;

let netTimer: ReturnType<typeof setTimeout> | undefined;
export function refreshNets() {
  clearTimeout(netTimer);
  netTimer = setTimeout(async () => {
    const { items, path } = get();
    const nets = await invoke<Netlist>("netlist", { items, dir: dirOf(path) });
    if (get().items === items) set({ nets, netsItems: items });
  }, 120);
}

export function commit(items: Item[], sel?: number[]) {
  const { items: prev, past } = get();
  set({ items, past: [...past.slice(-199), prev], future: [], dirty: true, stale: true, ...(sel && { sel }) });
  refreshNets();
}

export function undo() {
  const { past, items, future } = get();
  if (!past.length) return;
  set({ items: past[past.length - 1], past: past.slice(0, -1), future: [items, ...future], sel: [], dirty: true, stale: true });
  refreshNets();
}

export function redo() {
  const { past, items, future } = get();
  if (!future.length) return;
  set({ items: future[0], past: [...past, items], future: future.slice(1), sel: [], dirty: true, stale: true });
  refreshNets();
}

export async function loadSymbols(items: Item[] = []) {
  const names = [...new Set(items.flatMap((it) => (it.t === "sym" ? [it.sym] : [])))].filter((n) => !get().lib[symKey(n)]);
  if (Object.keys(get().lib).length && !names.length) return;
  const defs = await invoke<SymbolDef[]>("symbols", { names });
  set({ lib: { ...get().lib, ...Object.fromEntries(defs.map((d) => [d.name, d])) } });
}

/** Bumped on every document load; in-flight simulations of an older document are discarded. */
let docGen = 0;
/** Labelled nets are auto-plotted once per document, not every time the trace list is empty. */
let autoPlotGen = -1;

/** Git status of the open file (none for an unsaved document). */
export async function refreshGit() {
  const path = get().path;
  const git = path ? await invoke<GitStatus>("git_status", { path }) : null;
  if (get().path === path) set({ git }); // an older file's status must not land on the new one
}

/** The library sources changed: generated subckt boxes may differ now. */
export async function refreshLibrarySymbols() {
  set({ lib: Object.fromEntries(Object.entries(get().lib).filter(([k]) => !k.startsWith("subckt:"))) });
  await loadSymbols(get().items);
}

export async function loadDoc(items: Item[], path: string | null) {
  await loadSymbols(items);
  docGen++;
  set({ items, path, past: [], future: [], sel: [], dirty: false, sim: null, traces: [], stale: false, nets: null, netsItems: null, plot: NO_PLOT, panel: get().panel === "op" ? null : get().panel, diff: null, tool: { k: "select" }, fitToken: get().fitToken + 1 });
  refreshNets();
  refreshGit();
}

export function nextInst(items: Item[], prefix: string) {
  const p = prefix === "X" ? "U" : prefix;
  let n = 0;
  for (const it of items) {
    const m = it.t === "sym" ? it.attrs.InstName?.match(new RegExp(`^${p}(\\d+)$`, "i")) : null;
    if (m) n = Math.max(n, +m[1]);
  }
  return `${p}${n + 1}`;
}

/** Start placing a symbol (LTspice: R, C, L, D, F2 ...); `value` e.g. a library model name. */
export function placeSymbol(name: string, value?: string) {
  const def = get().lib[symKey(name)];
  if (!def) return;
  const attrs: Record<string, string> = { Value: value ?? def.value };
  set({ tool: { k: "place", item: { t: "sym", sym: name, x: 0, y: 0, rot: "R0", attrs } }, sel: [] });
}

/** Place a part from the library browser (a generated subckt symbol is fetched first). */
export async function placePart(symbol: string, value: string) {
  await loadSymbols([{ t: "sym", sym: symbol, x: 0, y: 0, rot: "R0", attrs: {} }]);
  placeSymbol(symbol, value);
}

// ---------------------------------------------------------------- simulation

const SLOTS = 8;
function freeSlot(traces: Trace[]) {
  const used = new Set(traces.map((t) => t.slot));
  for (let i = 0; i < SLOTS; i++) if (!used.has(i)) return i;
  return traces.length % SLOTS; // ponytail: >8 traces reuse colours; LTspice users rarely plot that many
}

async function fetchTrace(t: Trace): Promise<Trace> {
  const run = get().sim?.run;
  if (!run) return { ...t, data: undefined, error: "no simulation results: run the simulation" };
  try {
    return { ...t, data: await invoke<TraceData>("trace", { expr: t.expr, scale: run.scale, run: run.id }), error: undefined };
  } catch (e) {
    return { ...t, data: undefined, error: String(e) };
  }
}

/** Clicking the schematic plots (LTspice probing): results with node voltages / device currents.
 *  Noise plots hold spectra only (traces are added by name). */
export const probeMode = (sim: Sim | null) => !!sim?.run && !isOp(sim) && !/^(noise|sens)/.test(sim.run.plot);

/** Results without a scale (.op, .tf, .noise totals, .sens): a value table, nothing to plot or probe. */
export const isOp = (sim: Sim | null) => !!sim?.run && !sim.run.scale;
/** How a result is shown: curves, the s-plane (.pz has no scale but a view of its own), or a table only. */
export const viewOf = (sim: Sim | null): "none" | "plot" | "pz" | "table" => (!sim?.run ? "none" : sim.run.plot.startsWith("pz") ? "pz" : isOp(sim) ? "table" : "plot");
/** Identifies the loaded document; async work checks it before writing results back. */
export const docId = () => docGen;
/** Counts simulation starts: a trace fetched while a run started may belong to either circuit. */
let runSeq = 0;

export async function simulate() {
  if (get().running) return;
  const { items, path } = get();
  const gen = docGen;
  runSeq++;
  set({ running: true });
  try {
    const sim = await invoke<Sim>("simulate", { items, dir: dirOf(path) });
    if (gen !== docGen) return; // another document was opened meanwhile
    const changed = get().items !== items; // edited while ngspice ran: results are already stale
    const op = isOp(sim);
    const newKind = !!sim.run && plotKind(sim) !== get().plot.kind;
    set({
      sim, stale: changed, ...(!changed && { nets: sim.netlist, netsItems: items }),
      // another analysis: fresh zoom/cursors, and old-kind data never drawn on the new axes
      ...(newKind && { plot: { ...NO_PLOT, kind: plotKind(sim) }, traces: get().traces.map((t) => ({ ...t, data: undefined })) }),
      panel: !sim.run || sim.run.aborted ? "log" : op ? "op" : get().panel === "op" && !sim.run.op.length ? null : get().panel,
    });
    if (sim.run && !op) {
      // merge by expr: traces added/removed while refetching must survive
      const fresh = await Promise.all(get().traces.map(fetchTrace));
      if (gen === docGen) set({ traces: get().traces.map((t) => fresh.find((f) => f.expr === t.expr) ?? t) });
    }
    // first plottable run of a document: plot the labelled nets so there is something to look at
    // (.sens plots hold no nodes: they do not use up this one-time auto-plot)
    if (sim.run && !op && !sim.run.plot.startsWith("sens") && autoPlotGen !== docGen && gen === docGen) {
      autoPlotGen = docGen;
      const nets = new Set(items.flatMap((it, i) => (it.t === "flag" ? [sim.netlist.nets[i]?.[0]] : [])));
      if (get().traces.length) { /* the user already plotted something */ }
      else if (sim.run.plot.startsWith("noise")) await addTraceNow("onoise", "onoise_spectrum", "V/√Hz"); // .noise v(...): volts
      else {
        for (const n of [...nets].filter((n) => n && n !== "0").slice(0, 6)) await addTraceNow(`V(${n})`, vexpr(n), "V");
        // DC rails (vcc ...) would squash the signals on a shared axis: drop flat ones if any trace varies
        const flat = (t: Trace) => { const v = t.data?.re ?? []; const lo = Math.min(...v), hi = Math.max(...v); return hi - lo <= 1e-6 * Math.max(Math.abs(hi), Math.abs(lo), 1e-9); };
        const ts = get().traces;
        if (ts.some((t) => !flat(t))) set({ traces: ts.filter((t) => !flat(t)).slice(0, 4) });
      }
    }
  } catch (e) {
    // stale cleared so live mode does not retry a hard failure every 350 ms
    if (gen === docGen) set({ sim: { netlist: get().nets ?? { text: "", nets: [], insts: [], probes: [], warnings: [] }, simulated: "", run: null, log: [String(e)], sweepable: [] }, panel: "log", stale: false });
  } finally {
    set({ running: false });
  }
}

const idle = () =>
  new Promise<void>((resolve) => {
    if (!get().running) return resolve();
    const off = useS.subscribe((s) => !s.running && (off(), resolve()));
  });

/** Add a trace once no simulation is in flight (so it is evaluated against the final plot). */
export async function addTrace(label: string, expr: string, unit: string) {
  await idle();
  await addTraceNow(label, expr, unit);
}

async function addTraceNow(label: string, expr: string, unit: string) {
  const { traces, sim } = get();
  if (!sim?.run || isOp(sim) || traces.some((t) => t.expr === expr)) return;
  const gen = docGen;
  let seq: number, t: Trace;
  do {
    seq = runSeq;
    t = await fetchTrace({ label, expr, unit, slot: 0 });
    if (seq !== runSeq) await idle(); // a run started meanwhile: evaluate again on its result
  } while (seq !== runSeq);
  if (gen !== docGen || !get().sim?.run || isOp(get().sim)) return;
  const rest = get().traces.filter((x) => x.expr !== expr);
  set({ traces: [...rest, { ...t, slot: freeSlot(rest) }] }); // slot picked at insert time: concurrent adds differ
}

export const removeTrace = (expr: string) => set({ traces: get().traces.filter((t) => t.expr !== expr) });

/** Whether clicking `target` while probing can plot something (also picks the probe cursor). */
/** Whether `target` is something a probe can plot (structure only; see `probeMode` for the results). */
export function canProbe(target: Item | undefined) {
  const s = get();
  if (!target) return false;
  const nets = currentNets(s), i = s.items.indexOf(target);
  const net = nets?.nets[i]?.[0];
  const ground = net === "0" || (target.t === "flag" && /^(0|gnd)$/i.test(target.name));
  // parts: the backend says which have a current probe; without a fresh netlist, any known symbol
  const part = target.t === "sym" && (nets ? !!nets.probes[i] : !!defOf(s.lib, target));
  return ((target.t === "wire" || target.t === "flag") && !ground) || part;
}

/** LTspice probe: wire/label -> V(net), part -> current. Re-runs first if the results are stale,
 *  so net names always come from the netlist the results were computed with. */
export async function probe(target: Item) {
  if (!canProbe(target)) return; // only things that can be plotted are worth a re-simulation
  await idle();
  if (get().stale) await simulate();
  const { sim, items, stale } = get();
  const i = items.indexOf(target); // edits during the await may have shifted indices
  if (!sim?.run || !probeMode(sim) || stale || i < 0) return; // e.g. the re-run turned out to be .noise
  if (target.t === "sym") {
    const expr = sim.netlist.probes[i];
    if (expr) await addTraceNow(currentLabel(sim.netlist.insts[i]), expr, "A");
  } else {
    const net = sim.netlist.nets[i]?.[0];
    if (net && net !== "0") await addTraceNow(`V(${net})`, vexpr(net), "V");
  }
}
