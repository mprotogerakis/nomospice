// Mirrors src-tauri/src/{schematic,ngspice}.rs (serde JSON).

export type Item =
  | { t: "wire"; x1: number; y1: number; x2: number; y2: number }
  | { t: "flag"; x: number; y: number; name: string; extra?: string[] }
  | { t: "sym"; sym: string; x: number; y: number; rot: string; attrs: Record<string, string>; extra?: string[] }
  | { t: "text"; x: number; y: number; text: string; directive: boolean; align?: string }
  /** .asc lines nomospice does not interpret; kept so saving never loses them. Not drawn. */
  | { t: "raw"; line: string };

/** Items that live on the canvas (everything but raw lines). */
export type Shape = Exclude<Item, { t: "raw" }>;

export interface SymbolDef {
  name: string;
  prefix: string;
  value: string;
  spice_line: string;
  desc: string;
  pins: { x: number; y: number; name: string }[];
  shapes: [string, number[]][];
  windows: Record<string, [number, number]>;
  /** draw pin names (generated boxes for library subcircuits) */
  pin_labels: boolean;
}

/** A model or subcircuit in the indexed libraries (library.rs). */
export interface LibEntry {
  name: string;
  /** model type (D, NPN, NMOS ...) or SUBCKT */
  kind: string;
  pins: string[];
  source: string;
  file: string;
  /** symbol to place it with */
  symbol: string;
}

export interface GitStatus {
  repo: boolean;
  branch: string;
  /** clean | modified | untracked | added | new */
  file: string;
}

export interface GitCommit {
  hash: string;
  date: string;
  author: string;
  subject: string;
}

export interface LibSource {
  name: string;
  path: string;
  enabled: boolean;
}

export interface Netlist {
  text: string;
  nets: string[][];
  insts: string[];
  /** current into pin 1 as an ngspice expression ("" if none), aligned with items */
  probes: string[];
  warnings: string[];
}

export interface Run {
  /** which ngspice run: only the latest one's vectors can be traced */
  id: number;
  plot: string;
  scale: string;
  vectors: string[];
  /** name, re, im, unit ("V"/"A"/"") — for plots without a scale (.op, .tf, .pz, .noise totals) */
  op: [string, number, number, string][];
  /** ngspice gave up part-way: the data is partial */
  aborted: boolean;
  /** `.meas` results: name, value (null: failed), the rest of ngspice's line (trig/targ, from/to),
   *  whether the value is on the x axis (a TRIG/TARG span, a WHEN point) */
  meas: [string, number | null, string, boolean][];
}

export interface Sim {
  netlist: Netlist;
  /** the deck ngspice actually ran (current sensors, inlined includes): log line numbers refer to it */
  simulated: string;
  run: Run | null;
  log: string[];
  /** what a root locus can vary: .param names and R/C/L instances (by the sweep's own rules) */
  sweepable: string[];
  /** the schematic folder's symbols changed (edited there): the drawing reloads them */
  symbols?: boolean;
}

export interface TraceData {
  x: number[];
  re: number[];
  im: number[] | null;
  /** one value over the whole x range (0.5, vdd/2): a reference line on the top pane */
  constant?: boolean;
}

export interface Trace {
  label: string;
  expr: string;
  /** "V" | "A" | "" — picks the pane */
  unit: string;
  slot: number;
  data?: TraceData;
  error?: string;
  /** a power probe's V·I: real power only in .tran/.dc, not fetched in other analyses */
  power?: boolean;
}
