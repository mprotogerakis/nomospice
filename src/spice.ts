// SPICE syntax for the form-based editors: numbers, sources, analyses, directives.
// Pure functions (no imports): scripts/spice-check.ts tests them with plain Node.

/** SI formatting for readouts: 1.234m, 4.7k ... */
export function si(v: number, unit = "", digits = 4): string {
  if (!isFinite(v)) return String(v);
  if (v === 0) return `0${unit}`;
  const p = ["f", "p", "n", "µ", "m", "", "k", "M", "G", "T"];
  const e = Math.max(-5, Math.min(4, Math.floor(Math.log10(Math.abs(v)) / 3)));
  return `${+(v / 10 ** (3 * e)).toPrecision(digits)}${p[e + 5]}${unit}`;
}

/** Whitespace tokens; `{a * (b + c)}`, `'a * b'`, `NAME(args)` and `C={1/(2*pi)}` stay whole. */
export function tokens(s: string): string[] {
  const out: string[] = [];
  let cur = "", depth = 0, quote = false;
  for (const c of s) {
    if (c === "'") quote = !quote;
    else if (!quote && (c === "(" || c === "{")) depth++;
    else if (!quote && (c === ")" || c === "}")) depth = Math.max(0, depth - 1);
    if (/\s/.test(c) && !depth && !quote) (cur && out.push(cur), (cur = ""));
    else cur += c;
  }
  if (cur) out.push(cur);
  return out;
}
/** Numbers and parameter expressions ({T}, 'T*2') are values; `uic` & co. are not. */
export const isNum = (t: string) => /^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?[a-zµμ]*$/i.test(t) || /^[{']/.test(t);

/** Suffix -> power of ten (mil: 25.4 µm). Negative powers divide, so 10u is exactly 1e-5. */
const SUFFIX: [RegExp, number][] = [
  [/^meg/, 6], [/^t/, 12], [/^g/, 9], [/^k/, 3], [/^m/, -3], [/^[uµμ]/, -6], [/^n/, -9], [/^p/, -12], [/^f/, -15],
];
/** A SPICE number (`4.7k`, `1Meg`, `10uF`); null for expressions, names and junk. */
export function spiceNum(s: string): number | null {
  const m = s.trim().match(/^([+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?)([a-zµμ]*)$/i);
  if (!m) return null;
  const tail = m[2].toLowerCase();
  if (tail.startsWith("mil")) return (+m[1] * 25.4) / 1e6;
  const e = SUFFIX.find(([re]) => re.test(tail))?.[1] ?? 0;
  return e < 0 ? +m[1] / 10 ** -e : +m[1] * 10 ** e;
}

/** What a number field shows under it: the value as SPICE reads it, and the classic pitfalls. */
export function numInfo(s: string, unit = ""): { text: string; warn?: string } | null {
  const v = s.trim();
  if (!v) return null;
  if (/^[{']/.test(v)) return { text: "parameter expression" };
  if (/^[a-z_]\w*$/i.test(v)) return { text: "", warn: `write {${v}} to use a parameter` };
  const n = spiceNum(v);
  if (n === null) return { text: "", warn: "not a number: SPICE writes 4.7k, 10u, 1Meg, 2.5e-3" };
  const tail = v.match(/[a-zµμ]+$/i)?.[0] ?? "";
  const warn = /^M/.test(tail) && !/^meg/i.test(tail) ? "M is milli in SPICE: write Meg for mega"
    : /^F/.test(tail) ? "F is femto in SPICE (write 10u, not 10F)"
    : undefined;
  return { text: `= ${si(n, unit)}`, warn };
}

// ---------------------------------------------------------------- independent sources (V, I)

export type Func = "SINE" | "PULSE" | "EXP" | "SFFM" | "PWL";
/** One function argument; unit "y" is the source's own unit (V or A). */
export interface Arg { label: string; unit: string; def: string }
export const FUNCS: Record<Exclude<Func, "PWL">, Arg[]> = {
  SINE: [
    { label: "Offset", unit: "y", def: "0" }, { label: "Amplitude", unit: "y", def: "1" }, { label: "Frequency", unit: "Hz", def: "1k" },
    { label: "Delay", unit: "s", def: "0" }, { label: "Damping", unit: "1/s", def: "0" }, { label: "Phase", unit: "°", def: "0" },
  ],
  PULSE: [
    { label: "Initial value", unit: "y", def: "0" }, { label: "Pulsed value", unit: "y", def: "5" }, { label: "Delay", unit: "s", def: "0" },
    { label: "Rise time", unit: "s", def: "1n" }, { label: "Fall time", unit: "s", def: "1n" }, { label: "Pulse width", unit: "s", def: "0.5m" },
    { label: "Period", unit: "s", def: "1m" },
  ],
  EXP: [
    { label: "Initial value", unit: "y", def: "0" }, { label: "Pulsed value", unit: "y", def: "1" }, { label: "Rise delay", unit: "s", def: "0" },
    { label: "Rise time constant", unit: "s", def: "0.1m" }, { label: "Fall delay", unit: "s", def: "1m" }, { label: "Fall time constant", unit: "s", def: "0.1m" },
  ],
  SFFM: [
    { label: "Offset", unit: "y", def: "0" }, { label: "Amplitude", unit: "y", def: "1" }, { label: "Carrier frequency", unit: "Hz", def: "10k" },
    { label: "Modulation index", unit: "", def: "5" }, { label: "Signal frequency", unit: "Hz", def: "1k" },
  ],
};

export interface Source {
  dc: string;
  func: Func | "";
  /** function arguments as typed; PWL: t1 v1 t2 v2 ... */
  args: string[];
  acMag: string;
  acPhase: string;
  rser: string;
  cpar: string;
  /** everything else (other instance parameters, LTspice-only options), kept verbatim */
  rest: string;
}

/** Reads a source from its Value, Value2 and SpiceLine attributes (LTspice spreads it over all three). */
export function parseSource(...parts: (string | undefined)[]): Source {
  const s: Source = { dc: "", func: "", args: [], acMag: "", acPhase: "", rser: "", cpar: "", rest: "" };
  const t = tokens(parts.filter(Boolean).join(" "));
  const rest: string[] = [];
  for (let i = 0; i < t.length; i++) {
    const w = t[i];
    const fn = w.match(/^(SINE|SIN|PULSE|EXP|SFFM|PWL)\((.*)\)$/is);
    const kv = w.match(/^(rser|cpar)=(.+)$/i);
    if (fn && !s.func) {
      s.func = (fn[1].toUpperCase() === "SIN" ? "SINE" : fn[1].toUpperCase()) as Func;
      s.args = tokens(fn[2].replace(/,/g, " "));
    } else if (/^dc$/i.test(w) && t[i + 1]) s.dc = t[++i];
    else if (/^ac$/i.test(w) && t[i + 1] && isNum(t[i + 1])) {
      s.acMag = t[++i];
      if (t[i + 1] && isNum(t[i + 1])) s.acPhase = t[++i];
    } else if (kv) s[kv[1].toLowerCase() as "rser" | "cpar"] = kv[2];
    else if (i === 0 && isNum(w)) s.dc = w;
    else rest.push(w);
  }
  s.rest = rest.join(" ");
  return s;
}

/** Function arguments for SPICE: gaps filled with defaults, trailing blanks dropped. */
function argList(func: Func, args: string[]) {
  if (func === "PWL") return args.map((a) => a.trim()).filter(Boolean);
  const defs = FUNCS[func];
  const last = args.reduce((n, a, i) => (a.trim() ? i : n), -1);
  return defs.slice(0, Math.max(last + 1, 2)).map((d, i) => args[i]?.trim() || d.def);
}

/** The attributes LTspice writes: Value (DC and/or function), Value2 (AC), SpiceLine (parasitics). */
export function formatSource(s: Source): { Value: string; Value2: string; SpiceLine: string } {
  const fn = s.func ? `${s.func}(${argList(s.func, s.args).join(" ")})` : "";
  let value = [s.dc.trim() && (fn ? `DC ${s.dc.trim()}` : s.dc.trim()), fn].filter(Boolean).join(" ");
  let value2 = s.acMag.trim() ? `AC ${s.acMag.trim()}${s.acPhase.trim() ? ` ${s.acPhase.trim()}` : ""}` : "";
  if (!value) [value, value2] = [value2 || "0", ""]; // AC only: "AC 1" as the value, like LTspice
  const line = [s.rser.trim() && `Rser=${s.rser.trim()}`, s.cpar.trim() && `Cpar=${s.cpar.trim()}`, s.rest.trim()].filter(Boolean).join(" ");
  return { Value: value, Value2: value2, SpiceLine: line };
}

/** Points of the waveform for a preview, or null when it depends on parameters. */
export function sample(s: Source, n = 240): [number, number][] | null {
  const num = (a: string | undefined, d: string) => spiceNum(a?.trim() || d);
  const dc = s.dc.trim() ? spiceNum(s.dc) : 0;
  if (dc === null) return null;
  if (!s.func) return [[0, dc], [1, dc]];
  let f: (t: number) => number, end: number;
  if (s.func === "PWL") {
    const v = argList("PWL", s.args).map((a) => spiceNum(a));
    if (v.some((x) => x === null) || v.length < 2) return null;
    const pts: [number, number][] = [];
    for (let i = 0; i + 1 < v.length; i += 2) pts.push([v[i]!, v[i + 1]!]);
    end = Math.max(pts[pts.length - 1][0] * 1.1, 1e-12);
    f = (t) => {
      if (t <= pts[0][0]) return pts[0][1];
      const k = pts.findIndex(([x]) => x >= t);
      if (k < 0) return pts[pts.length - 1][1];
      const [[x0, y0], [x1, y1]] = [pts[k - 1], pts[k]];
      return x1 === x0 ? y1 : y0 + ((y1 - y0) * (t - x0)) / (x1 - x0);
    };
  } else {
    const a = FUNCS[s.func].map((d, i) => num(s.args[i], d.def));
    if (a.some((x) => x === null)) return null;
    const p = a as number[];
    if (s.func === "SINE") {
      const [off, amp, fr, td, th, ph] = p;
      end = td + (fr > 0 ? 3 / fr : 1);
      f = (t) => off + amp * (t < td ? Math.sin((ph * Math.PI) / 180) : Math.exp(-(t - td) * th) * Math.sin(2 * Math.PI * fr * (t - td) + (ph * Math.PI) / 180));
    } else if (s.func === "PULSE") {
      const [v1, v2, td, tr, tf, pw, per] = p;
      const one = tr + pw + tf;
      end = td + (per > 0 ? 2.2 * per : 1.5 * one || 1);
      f = (t) => {
        if (t < td) return v1;
        const x = per > 0 ? (t - td) % per : t - td;
        return x < tr ? v1 + ((v2 - v1) * x) / tr : x < tr + pw ? v2 : x < one ? v2 + ((v1 - v2) * (x - tr - pw)) / tf : v1;
      };
    } else if (s.func === "EXP") {
      const [v1, v2, td1, tau1, td2, tau2] = p;
      end = Math.max(td2 + 5 * tau2, td1 + 5 * tau1, 1e-12);
      f = (t) => v1 + (t < td1 ? 0 : (v2 - v1) * (1 - Math.exp(-(t - td1) / tau1))) + (t < td2 ? 0 : (v1 - v2) * (1 - Math.exp(-(t - td2) / tau2)));
    } else {
      const [off, amp, fc, mdi, fs] = p;
      end = fs > 0 ? 2 / fs : 1;
      f = (t) => off + amp * Math.sin(2 * Math.PI * fc * t + mdi * Math.sin(2 * Math.PI * fs * t));
    }
  }
  // transient waveform: the function alone (a DC value only sets the operating point)
  return Array.from({ length: n + 1 }, (_, i) => { const t = (end * i) / n; return [t, f(t)]; });
}

// ---------------------------------------------------------------- analyses

export type Analysis = "tran" | "ac" | "dc" | "noise" | "op" | "tf" | "pz" | "sens";
export const ANALYSIS = /^\.(tran|ac|dc|op|noise|tf|pz|sens)\b/i;
export type Fields = Record<string, string>;

/** `v(a)`, `v(a,b)`, `i(V1)` -> output fields (okind v|i, node, ref, isrc). */
export function parseOut(s = ""): Fields {
  const v = s.match(/^v\(\s*([^,)\s]+)\s*(?:,\s*([^)\s]+))?\s*\)$/i);
  if (v) return { okind: "v", node: v[1], ref: v[2] ?? "" };
  const i = s.match(/^i\(\s*([^)\s]+)\s*\)$/i);
  return i ? { okind: "i", isrc: i[1] } : { okind: "v", node: s };
}
export const formatOut = (f: Fields) =>
  f.okind === "i" ? `I(${f.isrc || "V1"})` : `V(${f.node || "out"}${f.ref && f.ref !== "0" ? `,${f.ref}` : ""})`;

/** An analysis line as form fields; `rest` keeps what the form does not show (e.g. LTspice options). */
export function parseAnalysis(line: string): { kind: Analysis; f: Fields } {
  const tok = tokens(line.trim());
  const kind = (tok[0]?.slice(1).toLowerCase() ?? "tran") as Analysis;
  const t = tok.slice(1);
  const after = (n: number) => t.slice(n).join(" ");
  switch (kind) {
    case "tran": {
      const flags = t.filter((x) => !isNum(x));
      const nums = t.filter(isNum);
      const rest = flags.filter((x) => !/^uic$/i.test(x)).join(" ");
      const uic = flags.some((x) => /^uic$/i.test(x)) ? "1" : "";
      return { kind, f: nums.length === 1 ? { stop: nums[0], uic, rest } : { step: nums[0] ?? "", stop: nums[1] ?? "", start: nums[2] ?? "", max: nums[3] ?? "", uic, rest } };
    }
    case "ac": return { kind, f: { sweep: (t[0] ?? "dec").toLowerCase(), pts: t[1] ?? "", fstart: t[2] ?? "", fstop: t[3] ?? "", rest: after(4) } };
    case "dc": return { kind, f: { src: t[0] ?? "", start: t[1] ?? "", stop: t[2] ?? "", incr: t[3] ?? "", src2: t[4] ?? "", start2: t[5] ?? "", stop2: t[6] ?? "", incr2: t[7] ?? "", rest: after(8) } };
    case "noise": return { kind, f: { ...parseOut(t[0]), src: t[1] ?? "", sweep: (t[2] ?? "dec").toLowerCase(), pts: t[3] ?? "", fstart: t[4] ?? "", fstop: t[5] ?? "", rest: after(6) } };
    case "tf": return { kind, f: { ...parseOut(t[0]), src: t[1] ?? "", rest: after(2) } };
    case "pz": return { kind, f: { inp: t[0] ?? "", inn: t[1] ?? "", outp: t[2] ?? "", outn: t[3] ?? "", tf: (t[4] ?? "vol").toLowerCase(), what: (t[5] ?? "pz").toLowerCase(), rest: after(6) } };
    case "sens": {
      const ac = /^ac$/i.test(t[1] ?? "");
      return { kind, f: { ...parseOut(t[0]), mode: ac ? "ac" : "dc", sweep: ac ? (t[2] ?? "dec").toLowerCase() : "dec", pts: ac ? t[3] ?? "" : "", fstart: ac ? t[4] ?? "" : "", fstop: ac ? t[5] ?? "" : "", rest: after(ac ? 6 : 1) } };
    }
    default: return { kind: "op", f: { rest: after(0) } };
  }
}

/** Form fields -> analysis line (blank fields get the placeholder defaults). */
export function formatAnalysis(kind: Analysis, f: Fields): string {
  const v = (k: string, d = "") => f[k]?.trim() || d;
  const sweep = (pts: string) => `${v("sweep", "dec")} ${v("pts", pts)} ${v("fstart", "10")} ${v("fstop", "1Meg")}`;
  const line = {
    tran: () => {
      const tail = [v("start") || (v("max") ? "0" : ""), v("max")].filter(Boolean);
      return [".tran", v("step") || (tail.length ? "0" : ""), v("stop", "1m"), ...tail, v("uic") ? "uic" : ""].filter(Boolean).join(" ");
    },
    ac: () => `.ac ${sweep("100")}`,
    dc: () => [".dc", v("src", "V1"), v("start", "0"), v("stop", "5"), v("incr", "10m"),
      ...(v("src2") ? [v("src2"), v("start2", "0"), v("stop2", "5"), v("incr2", "1")] : [])].join(" "),
    noise: () => `.noise ${formatOut({ ...f, okind: "v" })} ${v("src", "V1")} ${sweep("10")}`,
    op: () => ".op",
    tf: () => `.tf ${formatOut(f)} ${v("src", "V1")}`,
    pz: () => `.pz ${v("inp", "in")} ${v("inn", "0")} ${v("outp", "out")} ${v("outn", "0")} ${v("tf", "vol")} ${v("what", "pz")}`,
    sens: () => `.sens ${formatOut(f)}${v("mode") === "ac" ? ` ac ${sweep("10")}` : ""}`,
  }[kind]();
  return [line, v("rest")].filter(Boolean).join(" ");
}

// ---------------------------------------------------------------- other directives

export type DirectiveKind = "param" | "model" | "meas" | "ic" | "temp" | "include" | "text";

/** Which form edits this directive text (one line of a known kind), else "text". */
export function directiveKind(text: string): DirectiveKind {
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  if (lines.length !== 1) return "text";
  const l = lines[0].toLowerCase();
  return l.startsWith(".param ") ? "param" : l.startsWith(".model ") && parseModel(lines[0]) ? "model" : l.startsWith(".meas") && parseMeas(lines[0]) ? "meas"
    : l.startsWith(".ic ") ? "ic" : l.startsWith(".temp ") ? "temp" : /^\.(inc|include|lib)\s/.test(l) ? "include" : "text";
}

/** `name = value` pairs, spaces around `=` allowed. */
const pairs = (s: string) => tokens(s.replace(/\s*=\s*/g, "=")).map((p) => { const k = p.indexOf("="); return k > 0 ? [p.slice(0, k), p.slice(k + 1)] : [p, ""]; });

export const parseParams = (line: string) => pairs(line.replace(/^\s*\.param\s*/i, ""));
export const formatParams = (rows: string[][]) => `.param ${rows.filter(([n]) => n.trim()).map(([n, v]) => `${n.trim()}=${v.trim() || "0"}`).join(" ")}`;

export const parseIc = (line: string) => [...line.matchAll(/v\(\s*([^)\s]+)\s*\)\s*=\s*(\S+)/gi)].map((m) => [m[1], m[2]]);
export const formatIc = (rows: string[][]) => `.ic ${rows.filter(([n]) => n.trim()).map(([n, v]) => `V(${n.trim()})=${v.trim() || "0"}`).join(" ")}`;

export function parseInclude(line: string) {
  const m = line.trim().match(/^\.(inc\w*|lib)\s+("([^"]*)"|(\S+))\s*(\S*)/i);
  return { lib: !!m && m[1].toLowerCase() === "lib", path: m ? (m[3] ?? m[4]) : "", section: m?.[5] ?? "" };
}
export const formatInclude = (lib: boolean, path: string, section: string) => `${lib ? ".lib" : ".include"} "${path.trim()}"${lib && section.trim() ? ` ${section.trim()}` : ""}`;

/** Device model parameters the form offers (name, meaning, unit); others can be added by name. */
export const MODEL_PARAMS: Record<string, [string, string, string][]> = {
  D: [["IS", "Saturation current", "A"], ["N", "Emission coefficient", ""], ["RS", "Series resistance", "Ω"], ["CJO", "Zero-bias junction capacitance", "F"],
    ["VJ", "Junction potential", "V"], ["M", "Grading coefficient", ""], ["TT", "Transit time", "s"], ["BV", "Reverse breakdown voltage", "V"], ["IBV", "Current at breakdown", "A"]],
  NPN: [["IS", "Transport saturation current", "A"], ["BF", "Forward current gain", ""], ["BR", "Reverse current gain", ""], ["VAF", "Forward Early voltage", "V"],
    ["IKF", "Forward knee current", "A"], ["RB", "Base resistance", "Ω"], ["RC", "Collector resistance", "Ω"], ["RE", "Emitter resistance", "Ω"],
    ["CJE", "B-E zero-bias capacitance", "F"], ["CJC", "B-C zero-bias capacitance", "F"], ["TF", "Forward transit time", "s"], ["TR", "Reverse transit time", "s"]],
  NMOS: [["LEVEL", "Model level (1 = Shichman-Hodges)", ""], ["VTO", "Threshold voltage", "V"], ["KP", "Transconductance parameter", "A/V²"], ["LAMBDA", "Channel-length modulation", "1/V"],
    ["GAMMA", "Body-effect coefficient", "√V"], ["PHI", "Surface potential", "V"], ["RD", "Drain resistance", "Ω"], ["RS", "Source resistance", "Ω"],
    ["CGSO", "Gate-source overlap capacitance per width", "F/m"], ["CGDO", "Gate-drain overlap capacitance per width", "F/m"], ["CBD", "Bulk-drain capacitance", "F"], ["CBS", "Bulk-source capacitance", "F"]],
  NJF: [["VTO", "Threshold (pinch-off) voltage", "V"], ["BETA", "Transconductance parameter", "A/V²"], ["LAMBDA", "Channel-length modulation", "1/V"],
    ["RD", "Drain resistance", "Ω"], ["RS", "Source resistance", "Ω"], ["CGS", "Gate-source capacitance", "F"], ["CGD", "Gate-drain capacitance", "F"], ["IS", "Gate junction saturation current", "A"]],
  SW: [["VT", "Threshold voltage", "V"], ["VH", "Hysteresis voltage", "V"], ["RON", "On resistance", "Ω"], ["ROFF", "Off resistance", "Ω"]],
  CSW: [["IT", "Threshold current", "A"], ["IH", "Hysteresis current", "A"], ["RON", "On resistance", "Ω"], ["ROFF", "Off resistance", "Ω"]],
};
MODEL_PARAMS.PNP = MODEL_PARAMS.NPN;
MODEL_PARAMS.PMOS = MODEL_PARAMS.NMOS;
MODEL_PARAMS.PJF = MODEL_PARAMS.NJF;
export const MODEL_TYPES = ["D", "NPN", "PNP", "NMOS", "PMOS", "NJF", "PJF", "SW", "CSW"];

export function parseModel(line: string): { name: string; type: string; params: string[][] } | null {
  const m = line.trim().match(/^\.model\s+(\S+)\s+([a-z]+)\s*(?:\((.*)\)|(.*))$/is);
  if (!m || !MODEL_TYPES.includes(m[2].toUpperCase())) return null;
  return { name: m[1], type: m[2].toUpperCase(), params: pairs(m[3] ?? m[4] ?? "").filter(([k]) => k) };
}
export const formatModel = (name: string, type: string, params: string[][]) =>
  `.model ${name.trim() || "MyModel"} ${type}(${params.filter(([k, v]) => k.trim() && v.trim()).map(([k, v]) => `${k.trim().toUpperCase()}=${v.trim()}`).join(" ")})`;

/** `.meas` in the four shapes the form offers; null for anything else (edited as text). */
export interface Meas { an: string; name: string; type: "stat" | "find" | "when" | "delay"; f: Fields }
export function parseMeas(line: string): Meas | null {
  const t = tokens(line.trim().replace(/\s*=\s*/g, "="));
  if (!/^\.meas(ure)?$/i.test(t[0] ?? "") || t.length < 4) return null;
  const [an, name, op] = [t[1].toLowerCase(), t[2], t[3].toUpperCase()];
  const kv = (k: string, from = 4) => t.slice(from).find((x) => x.toUpperCase().startsWith(`${k}=`))?.slice(k.length + 1) ?? "";
  const edge = (from: number) => { const e = t.slice(from).find((x) => /^(rise|fall|cross)=/i.test(x)); return e ? { edge: e.split("=")[0].toLowerCase(), n: e.split("=")[1] } : { edge: "", n: "" }; };
  if (!["tran", "ac", "dc"].includes(an)) return null;
  if (["AVG", "RMS", "MIN", "MAX", "PP", "INTEG"].includes(op)) return { an, name, type: "stat", f: { op, expr: t[4] ?? "", from: kv("FROM"), to: kv("TO") } };
  if (op === "FIND") return { an, name, type: "find", f: { expr: t[4] ?? "", at: kv("AT") } };
  if (op === "WHEN") { const [expr, val] = (t[4] ?? "").split("="); return { an, name, type: "when", f: { expr, val: val ?? "", ...edge(5) } }; }
  if (op === "TRIG") {
    const k = t.findIndex((x) => x.toUpperCase() === "TARG");
    if (k < 0) return null;
    const side = (from: number, to: number) => ({ expr: t[from + 1] ?? "", val: t.slice(from, to).find((x) => /^val=/i.test(x))?.slice(4) ?? "", ...edge(from) });
    const [a, b] = [side(3, k), side(k, t.length)];
    return { an, name, type: "delay", f: { expr: a.expr, val: a.val, edge: a.edge, n: a.n, expr2: b.expr, val2: b.val, edge2: b.edge, n2: b.n } };
  }
  return null;
}
export function formatMeas(m: Meas): string {
  const v = (k: string, d = "") => m.f[k]?.trim() || d;
  const edge = (e: string, n: string) => `${(v(e) || "rise").toUpperCase()}=${v(n, "1")}`;
  const body = {
    stat: () => [v("op", "AVG"), v("expr", "V(out)"), v("from") && `FROM=${v("from")}`, v("to") && `TO=${v("to")}`].filter(Boolean).join(" "),
    find: () => `FIND ${v("expr", "V(out)")} AT=${v("at", "1m")}`,
    when: () => `WHEN ${v("expr", "V(out)")}=${v("val", "0")} ${edge("edge", "n")}`,
    delay: () => `TRIG ${v("expr", "V(in)")} VAL=${v("val", "0.5")} ${edge("edge", "n")} TARG ${v("expr2", "V(out)")} VAL=${v("val2", "0.5")} ${edge("edge2", "n2")}`,
  }[m.type]();
  return `.meas ${m.an} ${m.name.trim() || "result"} ${body}`;
}
