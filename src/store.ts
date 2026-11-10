import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { GitStatus, Item, Netlist, Shape, Sim, SymbolDef, Trace, TraceData } from "./types";
import { defOf, pinLine, prefixOf, symKey, vexpr, type Lib } from "./geom";
import { ANALYSIS, ANY_ANALYSIS, analysisLineIndex, analysisOptions, continuation, parseAnalysis, stripAnalyses, type Analysis } from "./spice";

export type Tool =
  | { k: "select" }
  | { k: "wire" }
  | { k: "place"; item: Item }
  /** pasted items following the cursor (positions relative to it), placed with a click */
  | { k: "paste"; items: Shape[] }
  | { k: "delete" }
  /** like select, but a drag moves the parts alone: their wires stay (LTspice's Move, F7) */
  | { k: "move" };

export type Dialog =
  /** raw: a directive as plain text, even when a form could edit it */
  | { k: "edit"; index: number; raw?: boolean; draft?: string }
  | { k: "new"; item: Item; at?: [number, number] }
  /** `add`: a new analysis of that kind (its own result tab) */
  | { k: "sim"; add?: Analysis }
  /** `replace`: choose a model for that part instead of placing a new one */
  | { k: "parts"; replace?: number; kind?: string; libraries?: boolean }
  | { k: "trace" }
  /** find parts, labels and texts by name */
  | { k: "find" }
  /** a pane's fixed y range (`pane`: its key in PlotView.y) */
  | { k: "yrange"; pane: string; unit: string; log: boolean }
  | { k: "about" }
  /** the quick reference: drawing, simulating, plotting, files */
  | { k: "help" }
  /** a symbol's attributes (Prefix, Value …) and type */
  | { k: "symattrs" };

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
  /** the document's path while no file is there yet (an opened netlist's .asc): its first save asks */
  unwritten: string | null;
  /** the schematics above this one, outermost first, when it was opened as a block's sheet */
  parents: string[];
  /** Waveform x-zoom and cursors A/B; reset per document and analysis kind. */
  plot: PlotView;
  menu: { x: number; y: number; items: import("./Menu").MenuItem[] } | null;
  git: GitStatus | null;
  /** visual diff: the schematic of an older commit, overlaid on the current one */
  diff: { hash: string; items: Item[] } | null;
  /** The result tab the user chose (an analysis kind); null: the schematic's active analysis. */
  tab: Analysis | null;
  /** The other tabs' results, kept while another tab is shown. */
  tabs: Partial<Record<Analysis, TabState>>;
  /** The tab `sim`, `traces`, `plot` belong to (what was last run or brought back). */
  shown: Analysis | null;
}

/** A result tab in the background: what `sim`, `traces`, `plot`, `stale` hold for the shown one;
 *  `at`: the items when it was put away (edited since: stale). */
interface TabState { sim: Sim | null; traces: Trace[]; plot: PlotView; stale: boolean; at: Item[] }

/** `kind`: the analysis these belong to; a run of another kind starts fresh. */
export interface PlotView {
  kind: string;
  zoom: [number, number] | null;
  a: number | null;
  b: number | null;
  /** the spectrum of lo..hi instead of the transient (zoom and cursors then in Hz; the time's
   *  kept to go back to) */
  fft: { lo: number; hi: number; time: { zoom: [number, number] | null; a: number | null; b: number | null } } | null;
  /** per pane (by its unit; `fft:` in the spectrum): log scale, fixed range */
  y: Record<string, { log?: boolean; lo?: number; hi?: number }>;
  /** an .ac result as a Nyquist or Nichols plot instead of the Bode panes */
  polar?: "nyquist" | "nichols";
}
const NO_PLOT: PlotView = { kind: "", zoom: null, a: null, b: null, fft: null, y: {} };
export const setPlot = (p: Partial<PlotView>) => useS.setState({ plot: { ...useS.getState().plot, ...p } });
/** Show the spectrum of lo..hi (zoom and cursors start afresh, in Hz), or the transient again. */
export function showFft(range: [number, number] | null) {
  const p = useS.getState().plot;
  if (range) setPlot({ fft: { lo: range[0], hi: range[1], time: p.fft?.time ?? { zoom: p.zoom, a: p.a, b: p.b } }, zoom: null, a: null, b: null });
  else if (p.fft) setPlot({ ...p.fft.time, fft: null });
}
export const setYAxis = (key: string, y: { log?: boolean; lo?: number; hi?: number }) => setPlot({ y: { ...useS.getState().plot.y, [key]: y } });
/** "tran:time", "ac:frequency" ... */
export const plotKind = (sim: Sim | null) => `${sim?.run?.plot.replace(/\d+$/, "") ?? ""}:${sim?.run?.scale ?? ""}`;

export const useS = create<State>(() => ({
  items: [], past: [], future: [], sel: [], tool: { k: "select" }, lib: {}, path: null, dirty: false,
  nets: null, netsItems: null, sim: null, running: false, stale: false, traces: [], live: false, panel: null, dialog: null,
  fitToken: 0, unwritten: null, parents: [], plot: NO_PLOT, menu: null, git: null, diff: null, tab: null, tabs: {}, shown: null,
}));
const set = useS.setState;
const get = useS.getState;

/** Cursor/hover readout: a separate store so pointer moves only re-render the status bar. */
export const useStatus = create(() => ({ x: 0, y: 0, net: null as string | null, probe: null as string | null, zoom: 1 }));

// ---------------------------------------------------------------- document

/** The document's file name ("untitled.asc" before its first save). */
export const fileName = (path: string | null | undefined) => path?.split(/[\\/]/).pop() || "untitled.asc";
export const dirOf = (path: string | null) => {
  if (!path) return null;
  const d = path.replace(/[\\/][^\\/]*$/, "");
  if (d === path) return null; // (a bare file name, a new symbol's: no folder yet)
  // a file at a root: "/" and "C:\" (a bare "C:" would be the current directory on that drive)
  return d === "" ? path[0] : /^[a-z]:$/i.test(d) ? `${d}\\` : d;
};
/** `p` relative to the open schematic's folder when inside it (as LTspice writes includes). */
export const relToDoc = (p: string) => {
  const dir = dirOf(useS.getState().path)?.replace(/[\\/]+$/, ""); // (a root's own separator off)
  if (dir == null) return p;
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
  if (isSymbolDoc(get().path)) return set({ nets: null, netsItems: null }); // (a symbol: no nets)
  netTimer = setTimeout(async () => {
    const { items, path } = get();
    const nets = await invoke<Netlist>("netlist", { items, dir: dirOf(path) });
    if (get().items === items) set({ nets, netsItems: items });
  }, 120);
}

/** An item as this document takes it: in a symbol a label is a pin (where its name shows, its
 *  number of its own when saved), a wire a line, and there are no parts or directives; in a
 *  schematic a label is a net label (its port kind, a name of one word). `copy`: a copy of a pin
 *  still there, which gets a number of its own (a cut pin pasted keeps its place). */
export function forDoc(it: Item, copy = false): Item | null {
  const symbol = isSymbolDoc(get().path);
  if (symbol && it.t === "sym") return null;
  if (symbol && it.t === "text" && it.directive) return { ...it, directive: false };
  if (symbol && it.t === "wire") return { t: "draw", kind: "LINE", width: "Normal", p: [it.x1, it.y1, it.x2, it.y2] };
  if (it.t !== "flag") return it;
  const extra = (it.extra ?? []).map((l) => l.trimStart())
    .filter((l) => (symbol ? l.startsWith("PIN ") || (l.startsWith("PINATTR ") && !(copy && l.startsWith("PINATTR SpiceOrder "))) : l.startsWith("IOPIN ")));
  if (symbol) return { ...it, extra: pinLine(extra) ? extra : [`PIN ${it.x} ${it.y} NONE 8`, ...extra] };
  // (a pin into a schematic: a net label, named as one)
  const name = it.name.trim().replace(/\s+/g, "_");
  return name ? { ...it, name, extra } : null;
}

/** A symbol's items as it takes them (`forDoc`), whatever put them there (the same objects where
 *  nothing changes). */
const symbolItems = (items: Item[]) =>
  items.flatMap((it) => {
    const x = forDoc(it);
    const same = x === it || (x?.t === "flag" && it.t === "flag" && x.name === it.name && x.extra?.join("\n") === it.extra?.join("\n"));
    return !x ? [] : same ? [it] : [x];
  });

export function commit(items: Item[], sel?: number[]) {
  const { items: prev, past } = get();
  if (isSymbolDoc(get().path)) {
    // (one place for every way in: placed, pasted, converted, edited)
    const own = symbolItems(items);
    if (own.length !== items.length) sel = [];
    items = own;
  }
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

/** The folder whose symbols the library holds (the schematic's: its .asy are listed too), and the
 *  requests in order (each answer replaces the set: one at a time, none superseded unanswered). */
let symDir: string | null | undefined, symQueue: Promise<unknown> = Promise.resolve();
/** `dir`: the schematic's folder when the request runs, unless given (a document being loaded);
 *  `fresh`: a document is being loaded (the current one's symbols are not needed any more). */
export function loadSymbols(items: Item[] = [], dir?: string | null, opts: { force?: boolean; fresh?: boolean } = {}) {
  const job = symQueue.catch(() => {}).then(() => loadSymbolsNow(items, dir === undefined ? dirOf(get().path) : dir, opts));
  symQueue = job;
  return job;
}

async function loadSymbolsNow(items: Item[], dir: string | null, { force = false, fresh = false }) {
  const moved = dir !== symDir || force; // another folder (or other libraries): ask again for all
  const want = (its: Item[]) => its.flatMap((it) => (it.t === "sym" ? [it.sym] : []));
  if (Object.keys(get().lib).length && !moved && want(items).every((n) => get().lib[symKey(n)])) return;
  // the answer replaces ours, and the backend may have dropped symbols it had cached (switched-off
  // libraries): ask for everything in use — the document, its undo history, a diff on show
  const s = get(), names = new Set(want(items));
  const placing = s.tool.k === "place" ? [s.tool.item] : s.tool.k === "paste" ? s.tool.items : []; // parts being placed, too
  if (!fresh) for (const its of [s.items, ...s.past, ...s.future, s.diff?.items ?? [], placing]) for (const it of its) if (it.t === "sym") names.add(it.sym);
  const defs = await invoke<SymbolDef[]>("symbols", { names: [...names], dir });
  symDir = dir; // only once answered: a failed request is asked again next time
  set({ lib: Object.fromEntries(defs.map((d) => [d.name, d])) }); // the backend's whole set
}

/** Bumped on every document load; in-flight simulations of an older document are discarded. */
let docGen = 0;
/** Labelled nets are auto-plotted once per document and analysis, not every time the list is empty. */
let autoPlotted = new Set<string>();

/** Git status of the open file (none for an unsaved document). */
export async function refreshGit() {
  const path = get().path;
  // (a document in no folder yet: none)
  const git = path && dirOf(path) ? await invoke<GitStatus>("git_status", { path }) : null;
  if (get().path === path) set({ git }); // an older file's status must not land on the new one
}

/** The library sources changed: generated subckt boxes may differ now. */
export async function refreshLibrarySymbols() {
  await loadSymbols(get().items, dirOf(get().path), { force: true }); // the backend's set replaces ours
}

/** `unwritten`: the path is that of no file yet (its first save asks where). */
export async function loadDoc(items: Item[], path: string | null, unwritten = false) {
  await loadSymbols(items, dirOf(path), { fresh: true, force: true }); // reopening reads edited symbols
  docGen++;
  autoPlotted = new Set();
  set({ dialog: null, menu: null, tab: null, tabs: {}, shown: null, items, path, unwritten: unwritten ? path : null, parents: path && path === get().path ? get().parents : [], past: [], future: [], sel: [], dirty: false, sim: null, traces: [], stale: false, nets: null, netsItems: null, plot: NO_PLOT, panel: get().panel === "op" ? null : get().panel, diff: null, tool: { k: "select" }, fitToken: get().fitToken + 1 });
  refreshNets();
  refreshGit();
}

export function nextInst(items: Item[], prefix: string) {
  const p = prefix === "X" ? "U" : prefix;
  const re = new RegExp(`^${p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(\\d+)$`, "i"); // (a Prefix attribute is any text: as text)
  let n = 0;
  for (const it of items) {
    const m = it.t === "sym" ? it.attrs.InstName?.match(re) : null;
    if (m) n = Math.max(n, +m[1]);
  }
  return `${p}${n + 1}`;
}

/** Start placing a symbol (LTspice: R, C, L, D, F2 ...); `value` e.g. a library model name. */
export function placeSymbol(name: string, value?: string) {
  const def = get().lib[symKey(name)];
  if (!def || isSymbolDoc(get().path)) return; // (a symbol holds no parts)
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
  if (t.power && !powerMode(get().sim)) return { ...t, data: undefined, error: "power: only in .tran and .dc results" };
  try {
    return { ...t, data: await invoke<TraceData>("trace", { expr: t.expr, scale: run.scale, run: run.id }), error: undefined };
  } catch (e) {
    return { ...t, data: undefined, error: String(e) };
  }
}

/** Clicking the schematic plots (LTspice probing): results with node voltages / device currents.
 *  Noise plots hold spectra only (traces are added by name). */
export const probeMode = (sim: Sim | null) => !!sim?.run && !isOp(sim) && !/^(noise|sens)/.test(sim.run.plot);
/** Power V·I is real power only in time and DC sweeps (in .ac it would be a complex product). */
export const powerMode = (sim: Sim | null) => /^(tran|dc)/.test(sim?.run?.plot ?? "");

/** Results without a scale (.op, .tf, .noise totals, .sens): a value table, nothing to plot or probe. */
export const isOp = (sim: Sim | null) => !!sim?.run && !sim.run.scale;
/** How a result is shown: curves, the s-plane (.pz has no scale but a view of its own), or a table only. */
export const viewOf = (sim: Sim | null): "none" | "plot" | "pz" | "table" => (!sim?.run ? "none" : sim.run.plot.startsWith("pz") ? "pz" : isOp(sim) ? "table" : "plot");
/** Identifies the loaded document; async work checks it before writing results back. */
export const docId = () => docGen;
/** Counts simulation starts: a trace fetched while a run started may belong to either circuit. */
let runSeq = 0;

// ---------------------------------------------------------------- result tabs

/** The analyses of the schematic, one per kind: the result tabs (see `analysisOptions`); parsed
 *  once per items array (the tab bar, the store and the dialogs all ask). */
const analysesCache = new WeakMap<Item[], ReturnType<typeof analysisOptions>>();
export const analysesOf = (items: Item[]) => {
  let o = analysesCache.get(items);
  if (!o) analysesCache.set(items, (o = analysisOptions(items.flatMap((it) => (it.t === "text" ? [it] : [])))));
  return o;
};

/** The tab shown: the chosen one while the schematic still has it, else its active analysis. */
export function currentTab(s: { tab: Analysis | null; items: Item[] }, o = analysesOf(s.items)): Analysis | null {
  if (s.tab && o.some((x) => x.kind === s.tab)) return s.tab;
  // only commented alternatives (LTspice's `;.tran …`): the first of them, so Run has one to run —
  // unless ngspice has an analysis of its own (.pss alone): that runs, under no tab's name
  return (o.find((x) => x.active) ?? (activeLines(s.items, ANY_ANALYSIS) ? undefined : o[0]))?.kind ?? null;
}

/** How many active analysis lines (of `re`) the schematic's directives hold. */
const activeLines = (items: Item[], re: RegExp) =>
  items.reduce((n, it) => n + (it.t === "text" && it.directive ? it.text.split("\n").filter((l) => re.test(l.trim())).length : 0), 0);

/** Index of the text item holding the analysis of `kind` (the active one first). */
export function analysisItem(items: Item[], kind: Analysis | null) {
  const holds = (it: Item) => it.t === "text" && analysisOptions([it]).some((o) => o.kind === kind);
  const active = items.findIndex((it) => it.t === "text" && it.directive && holds(it));
  return active >= 0 ? active : items.findIndex(holds);
}

/** What a run for tab `tab` gets: a chosen tab runs its analysis instead of the schematic's. The
 *  items keep their indices (the netlist's arrays are aligned with them), the analyses are taken
 *  out, the tab's line goes last. Simulation and root locus use it. */
export function simDeck(items: Item[], tab: Analysis | null, opts = analysesOf(items)) {
  // several active analyses (ngspice would run them all and show the last): the shown tab's alone;
  // the schematic's only active analysis runs as it is (the live netlist stays the schematic's)
  // (the form's kinds: an extra .pss beside one .op runs along as written, as before)
  // (none of ngspice's at all: only commented alternatives run the shown one; an active .pss alone
  // runs as written, and is not replaced by an alternative)
  const actives = activeLines(items, ANALYSIS), any = activeLines(items, ANY_ANALYSIS);
  // (a chosen tab whose analysis is gone: the shown one, as currentTab says)
  const kind = tab && opts.some((o) => o.kind === tab) ? tab : actives > 1 || !any ? currentTab({ tab, items }, opts) : null;
  const pick = kind ? opts.find((o) => o.kind === kind) : undefined;
  if (!pick || (pick.active && actives === 1)) return items;
  return [...items.map((it) => (it.t === "text" && it.directive ? { ...it, text: stripAnalyses(it.text) } : it)), { t: "text", x: 0, y: 0, text: pick.line, directive: true } as Item];
}

/** Show another result tab: the shown one is put away, the other's results come back (a tab
 *  that never ran runs now). Not while a simulation runs: its result belongs to the shown tab. */
export async function selectTab(kind: Analysis) {
  await idle(); // a run in progress belongs to the shown tab ("+" during a live run)
  const s = get();
  if (!analysesOf(s.items).some((o) => o.kind === kind)) return; // no such tab: the shown results stay
  if (s.shown === kind) {
    if (s.tab !== kind) set({ tab: kind });
    if (!s.sim) void simulate(); // shown already (an edit made it so) but never run: it runs now
    return;
  }
  set({ tab: kind, ...swapTo(s, kind) });
  if (!get().sim) void simulate();
}

/** `items` without the analysis of `kind` (the active one, or the commented alternative), and the
 *  index of the text it stood in; a text left empty goes. null: none such. */
function withoutAnalysis(items: Item[], kind: Analysis, active: boolean): [Item[], number] | null {
  const k = items.findIndex((it) => it.t === "text" && !!it.directive === active && analysisOptions([it]).some((o) => o.kind === kind));
  const it = items[k];
  if (it?.t !== "text") return null;
  const lines = it.text.split("\n"), li = analysisLineIndex(lines, kind, active);
  const drop = new Set([li, ...continuation(lines, li)]);
  const rest = lines.filter((_, j) => !drop.has(j));
  return [rest.some((l) => l.trim()) ? items.map((x, j) => (j === k ? { ...it, text: rest.join("\n") } : x)) : items.filter((_, j) => j !== k), k];
}

/** "×" on a result tab: its analysis leaves the schematic, the active line and commented ones of
 *  its kind alike (⌘Z brings them back, with the results). The schematic's own analysis closed:
 *  the first alternative left takes its place, so the file still has one to run (also in other
 *  simulators). The last tab stays. */
export function closeTab(kind: Analysis) {
  const { items, running } = get();
  const opts = analysesOf(items), o = opts.find((x) => x.kind === kind);
  if (running || !o || opts.length < 2) return;
  let next = items, at: Extract<Item, { t: "text" }> | null = null, kept = 0;
  for (let cut; (cut = withoutAnalysis(next, kind, true) ?? withoutAnalysis(next, kind, false)); next = cut[0]) {
    const was = next[cut[1]] as Extract<Item, { t: "text" }>;
    if (was.directive && !at) {
      // where the active one stood (below what stays of its text)
      at = was;
      kept = cut[0].length === next.length ? (cut[0][cut[1]] as Extract<Item, { t: "text" }>).text.split("\n").length : 0;
    }
  }
  // the first alternative as written in the file (by line, also within one comment)
  const first = next.flatMap((it) => {
    if (it.t !== "text" || it.directive || !analysisOptions([it]).length) return [];
    const lines = it.text.split("\n"), li = analysisLineIndex(lines, null, false), kind = parseAnalysis(lines[li].trim()).kind;
    return analysisOptions([it]).filter((o) => o.kind === kind);
  })[0];
  // (an active analysis left, also ngspice's own .pss: it runs as it is, none is added beside it)
  if (at && first && !activeLines(next, ANY_ANALYSIS)) {
    const moved = withoutAnalysis(next, first.kind, false);
    if (moved) next = [...moved[0], { t: "text", x: at.x, y: at.y + 32 * kept, text: first.line, directive: true }];
  }
  commit(next, []); // (a text may have gone: indices shift)
}

/** The shown results put away under their tab (if the schematic still has it), `kind`'s back. */
function swapTo(s: State, kind: Analysis | null): Partial<State> {
  const tabs = { ...s.tabs };
  // kept also when its analysis is gone: an undo brings the analysis back, and with it these
  if (s.shown) tabs[s.shown] = { sim: s.sim, traces: s.traces, plot: s.plot, stale: s.stale, at: s.items };
  const next = kind ? tabs[kind] : undefined;
  if (kind) delete tabs[kind];
  const sim = next?.sim ?? null, view = viewOf(sim);
  return {
    tabs, shown: kind, sim, traces: next?.traces ?? [], plot: next?.plot ?? NO_PLOT,
    stale: !sim || next!.stale || next!.at !== s.items, // no results yet: a run is due (live mode runs it)
    panel: view === "table" ? "op" : s.panel === "op" ? null : s.panel,
  };
}

// An edit deleted the shown results' analysis or made it another kind: the tab now shown gets
// its own results (none yet: Run), never the other analysis' under its name.
useS.subscribe((s, prev) => {
  // after an edit, or when a run ends (an edit during it was skipped: its results are filed meanwhile)
  if ((s.items === prev.items && !(prev.running && !s.running)) || s.running) return;
  const opts = analysesOf(s.items), cur = currentTab(s, opts);
  if (cur === s.shown || (!s.shown && !(cur && s.tabs[cur]))) return; // (nothing shown, nothing to bring back)
  const keep = !!s.tab && opts.some((o) => o.kind === s.tab);
  set({ tab: keep ? s.tab : null, ...swapTo(s, cur) });
});

/** A symbol (.asy) is open: drawn on the canvas, nothing to simulate. */
export const isSymbolDoc = (path: string | null) => !!path && /\.asy$/i.test(path);

/** Run the simulation; once more if another window's run took its results before they were read
 *  (all windows share the one ngspice, which holds the latest run's results only). */
export async function simulate() {
  if (await runOnce()) await runOnce();
}

/** One run; true: its results were gone before its traces were read. */
async function runOnce(): Promise<boolean> {
  if (get().running || isSymbolDoc(get().path)) return false;
  const { items, path, tab } = get();
  const opts = analysesOf(items), deck = simDeck(items, tab, opts), shown = currentTab(get(), opts); // the tab these results are for
  const gen = docGen;
  runSeq++;
  set({ running: true });
  let gone = false;
  try {
    const sim = await invoke<Sim>("simulate", { items: deck, dir: dirOf(path) });
    if (gen !== docGen) return false; // another document was opened meanwhile
    // edited next to the schematic: reloaded for the folder current when the request runs (a
    // document loaded meanwhile must not get the old folder back)
    if (sim.symbols) void loadSymbols(get().items, undefined, { force: true }).catch(() => {});
    const changed = get().items !== items; // edited while ngspice ran: results are already stale
    const op = isOp(sim);
    const newKind = !!sim.run && plotKind(sim) !== get().plot.kind;
    set({
      // the live netlist only from a run of the schematic as it is (a tab's run swaps its analysis)
      sim, shown, stale: changed, ...(!changed && deck === items && { nets: sim.netlist, netsItems: items }),
      // another analysis: fresh zoom/cursors, and old-kind data never drawn on the new axes
      ...(newKind && { plot: { ...NO_PLOT, kind: plotKind(sim) }, traces: get().traces.map((t) => ({ ...t, data: undefined })) }),
      panel: !sim.run || sim.run.aborted ? "log" : op ? "op" : get().panel === "op" && !sim.run.op.length ? null : get().panel,
    });
    if (sim.run && !op) {
      // merge by expr: traces added/removed while refetching must survive
      const fresh = await Promise.all(get().traces.map(fetchTrace));
      // (only the data: a trace's other settings, e.g. its axis, may have changed meanwhile)
      if (gen === docGen) set({ traces: get().traces.map((t) => { const f = fresh.find((x) => x.expr === t.expr); return f ? { ...t, data: f.data, error: f.error } : t; }) });
    }
    // first plottable run of a document: plot the labelled nets so there is something to look at
    // (.sens plots hold no nodes: they do not use up this one-time auto-plot)
    if (sim.run && !op && !sim.run.plot.startsWith("sens") && !autoPlotted.has(plotKind(sim)) && gen === docGen) {
      autoPlotted.add(plotKind(sim));
      const nets = new Set(items.flatMap((it, i) => (it.t === "flag" ? [sim.netlist.nets[i]?.[0]] : [])));
      if (get().traces.length) { /* the user already plotted something */ }
      else if (sim.run.plot.startsWith("noise")) await addTraceNow("onoise", "onoise_spectrum", "V/√Hz"); // .noise v(...): volts
      else {
        // (a .step run: fewer, each is a family of curves)
        for (const n of [...nets].filter((n) => n && n !== "0").slice(0, sim.run.steps?.length ? 3 : 6)) await addTraceNow(`V(${n})`, vexpr(n), "V");
        // DC rails (vcc ...) would squash the signals on a shared axis: drop flat ones if any trace varies
        // (a loop: spreading 70k samples of a stepped trace into Math.min overflows the stack; gaps skipped)
        const flat = (t: Trace) => {
          let lo = Infinity, hi = -Infinity;
          for (const y of t.data?.re ?? []) if (y != null) (lo = Math.min(lo, y)), (hi = Math.max(hi, y));
          return hi - lo <= 1e-6 * Math.max(Math.abs(hi), Math.abs(lo), 1e-9);
        };
        const ts = get().traces;
        if (ts.some((t) => !flat(t))) set({ traces: ts.filter((t) => !flat(t)).slice(0, 4) });
      }
    }
    // (its traces, the auto-plotted too, failed for another window's run since: run once more)
    if (sim.run && !op && gen === docGen) gone = get().traces.some((t) => t.error) && (await invoke<number>("run_id")) !== sim.run.id;
  } catch (e) {
    // stale cleared so live mode does not retry a hard failure every 350 ms
    if (gen === docGen) set({ shown, sim: { netlist: get().nets ?? { text: "", nets: [], insts: [], probes: [], pin_currents: [], sheets: [], warnings: [] }, simulated: "", run: null, log: [String(e)], sweepable: [] }, panel: "log", stale: false });
  } finally {
    set({ running: false });
  }
  return gone && gen === docGen;
}

export const idle = () =>
  new Promise<void>((resolve) => {
    if (!get().running) return resolve();
    const off = useS.subscribe((s) => !s.running && (off(), resolve()));
  });

/** Add a trace once no simulation is in flight (so it is evaluated against the final plot). */
export async function addTrace(label: string, expr: string, unit: string) {
  await idle();
  await addTraceNow(label, expr, unit);
}

async function addTraceNow(label: string, expr: string, unit: string, power = false) {
  const { traces, sim } = get();
  if (!sim?.run || isOp(sim) || traces.some((t) => t.expr === expr)) return;
  const gen = docGen;
  // a result tab brought back may hold a run ngspice no longer has (another tab or a root locus ran
  // since): run it again first (not from inside a run, e.g. simulate's auto-plot)
  if (!get().running && get().sim?.run && get().sim!.run!.id !== (await invoke<number>("run_id"))) await simulate();
  let seq: number, t: Trace;
  do {
    seq = runSeq;
    t = await fetchTrace({ label, expr, unit, slot: 0, power: power || undefined });
    if (seq !== runSeq) await idle(); // a run started meanwhile: evaluate again on its result
  } while (seq !== runSeq);
  if (gen !== docGen || !get().sim?.run || isOp(get().sim)) return;
  const rest = get().traces.filter((x) => x.expr !== expr);
  set({ traces: [...rest, { ...t, slot: freeSlot(rest) }] }); // slot picked at insert time: concurrent adds differ
}

export const removeTrace = (expr: string) => set({ traces: get().traces.filter((t) => t.expr !== expr) });
/** A trace onto its pane's right axis (own scale) or back to the left one. */
export const setTraceRight = (expr: string, right: boolean) => set({ traces: get().traces.map((t) => (t.expr === expr ? { ...t, right: right || undefined } : t)) });
/** Draw the trace in the pane of `unit` (on its right axis, its own unit there); undefined: its own pane. */
export const setTracePane = (expr: string, unit: string | undefined) => set({ traces: get().traces.map((t) => (t.expr === expr ? { ...t, pane: unit } : t)) });

/** Whether clicking `target` while probing can plot something (also picks the probe cursor). */
/** Whether `target` is something a probe can plot (structure only; see `probeMode` for the results). */
export function canProbe(target: Item | undefined, pin?: number) {
  const s = get();
  if (!target) return false;
  const nets = currentNets(s), i = s.items.indexOf(target);
  const net = nets?.nets[i]?.[0];
  const ground = net === "0" || (target.t === "flag" && /^(0|gnd)$/i.test(target.name));
  // parts: the backend says which have a current probe; without a fresh netlist, any known symbol
  // (without one: a subcircuit of more than two pins has currents per pin only)
  const def = target.t === "sym" ? defOf(s.lib, target) : undefined;
  const part = target.t === "sym" && (nets ? !!partCurrent(nets, i, pin) : !!def && (pin != null || !(prefixOf(target, def) === "X" && def.pins.length > 2)));
  return ((target.t === "wire" || target.t === "flag") && !ground) || part;
}

/** A two-pin part's power by the name of its current: I(D1) -> P(D1). */
export const powerName = (current: string) => current.replace(/^I/, "P");

/** The name of part `i`'s current, which is also its trace expression (the backend reads I(R1),
 *  Ib(Q1) …): into `pin` where that has a sensor, else the part's own (pin 1; a source's I(V1)).
 *  "": none (a subcircuit's current is per pin). */
export function partCurrent(nl: Netlist, i: number, pin?: number) {
  const named = nl.pin_currents?.[i] ?? [];
  return (pin != null && named[pin]) || (nl.probes[i] ? named[0] ?? "" : "");
}

/** LTspice probe: wire/label -> V(net), part -> current. Re-runs first if the results are stale,
 *  so net names always come from the netlist the results were computed with. */
export async function probe(target: Item, power = false, pin?: number) {
  if (!canProbe(target, pin)) return; // only things that can be plotted are worth a re-simulation
  await idle();
  if (get().stale) await simulate();
  const { sim, items, stale } = get();
  const i = items.indexOf(target); // edits during the await may have shifted indices
  if (!sim?.run || !probeMode(sim) || stale || i < 0) return; // e.g. the re-run turned out to be .noise
  if (target.t === "sym" && power) {
    if (!powerMode(sim)) return;
    // LTspice's power probe: the voltage across a two-pin part times the current into pin 1 —
    // positive where it takes power (a resistor), negative where it gives it (a source)
    const pins = sim.netlist.nets[i], cur = partCurrent(sim.netlist, i);
    if (cur && pins?.length === 2) await addTraceNow(powerName(cur), `(${vexpr(pins[0])}-${vexpr(pins[1])})*${cur}`, "W", true);
  } else if (target.t === "sym") {
    const cur = partCurrent(sim.netlist, i, pin); // (also the expression: the backend reads the name)
    if (cur) await addTraceNow(cur, cur, "A");
  } else {
    const net = sim.netlist.nets[i]?.[0];
    if (net && net !== "0") await addTraceNow(`V(${net})`, vexpr(net), "V");
  }
}
