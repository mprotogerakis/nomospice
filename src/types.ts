// Mirrors src-tauri/src/{schematic,ngspice}.rs (serde JSON).

export type Item =
  | { t: "wire"; x1: number; y1: number; x2: number; y2: number }
  | { t: "flag"; x: number; y: number; name: string; extra?: string[] }
  | { t: "sym"; sym: string; x: number; y: number; rot: string; attrs: Record<string, string>; extra?: string[] }
  | { t: "text"; x: number; y: number; text: string; directive: boolean; align?: string }
  /** The schematic's own drawing: LINE, RECTANGLE, CIRCLE or ARC as written, its pen (Normal,
   *  Wide), its points (x y pairs: two; an ARC four: box, start, end) and line style (1 dash … 4). */
  | { t: "draw"; kind: string; width: string; p: number[]; style?: number }
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
  /** `just`: where its name is drawn (LEFT, RIGHT, TOP, BOTTOM, V…), `off` units from it; none: not */
  pins: { x: number; y: number; name: string; just?: string; off?: number }[];
  shapes: [string, number[]][];
  windows: Record<string, [number, number]>;
  /** draw pin names (generated boxes for library subcircuits) */
  pin_labels: boolean;
  /** a hierarchical block: the schematic it stands for (its subcircuit) */
  sheet?: string;
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
  /** per item, per pin: the name of the current into it where it has a sensor (I(R1), Ib(Q1),
   *  Ix(U1:OUT)), else ""; trace expressions take these names */
  pin_currents: string[][];
  /** per item: a block's schematic (the sheet it runs as), else "" */
  sheets: string[];
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
  /** a .step run: each step's label (R=1k), in order; a trace has one curve per step */
  steps?: string[];
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
  /** a level (0.5, 3.3/2: numbers only) over the whole x range: a reference line on the top pane */
  constant?: boolean;
  /** a .step run: which steps (indices into Run.steps) its curves are, end to end */
  steps?: number[] | null;
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
  /** drawn against its pane's right axis (own scale: shapes of different size compared) */
  right?: boolean;
  /** drawn in the pane of this unit instead of its own (on the right axis there) */
  pane?: string;
}
