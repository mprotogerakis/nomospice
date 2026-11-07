// SPICE syntax for the form-based editors: numbers, sources, analyses, directives.
// Pure functions (no imports): scripts/spice-check.ts tests them with plain Node.

/** SI formatting for readouts: 1.234m, 4.7k ... */
export function si(v: number, unit = "", digits = 4): string {
  if (!isFinite(v)) return String(v);
  if (v === 0) return `0${unit}`;
  const p = ["f", "p", "n", "µ", "m", "", "k", "M", "G", "T"];
  let e = Math.max(-5, Math.min(4, Math.floor(Math.log10(Math.abs(v)) / 3)));
  let m = +(v / 10 ** (3 * e)).toPrecision(digits);
  if (Math.abs(m) >= 1000 && e < 4) m = +(v / 10 ** (3 * ++e)).toPrecision(digits); // 999.96 → 1k, not 1000
  return `${m}${p[e + 5]}${unit}`;
}

/** Whitespace tokens; `{a * (b + c)}`, `'a * b'`, `NAME(args)` and `C={1/(2*pi)}` stay whole. */
export function tokens(s: string): string[] {
  const out: string[] = [];
  let cur = "", depth = 0, q = "";
  for (const c of s) {
    const quote = !!q;
    if (c === "'" || c === '"') q = !q ? c : q === c ? "" : q; // like spice_tokens in schematic.rs
    else if (!quote && (c === "(" || c === "{")) depth++;
    else if (!quote && (c === ")" || c === "}")) depth = Math.max(0, depth - 1);
    if (/\s/.test(c) && !depth && !q && !quote) (cur && out.push(cur), (cur = ""));
    else cur += c;
  }
  if (cur) out.push(cur);
  return out;
}
/** Numbers and parameter expressions ({T}, 'T*2') are values; `uic` & co. are not. */
export const isNum = (t: string) => spiceNum(t) !== null || /^[{']/.test(t);

/** Suffix -> power of ten (mil: 25.4 µm). Negative powers divide, so 10u is exactly 1e-5. */
const SUFFIX: [RegExp, number][] = [
  [/^meg/, 6], [/^t/, 12], [/^g/, 9], [/^k/, 3], [/^m/, -3], [/^[uµμ]/, -6], [/^n/, -9], [/^p/, -12], [/^f/, -15], [/^a/, -18],
];
/** A SPICE number (`4.7k`, `1Meg`, `10uF`); null for expressions, names and junk. */
export function spiceNum(s: string): number | null {
  const m = s.trim().replace(/Ω$/, "").match(/^([+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?)([a-zµμ]*)$/i);
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
  const tail = v.replace(/Ω$/, "").match(/[a-zµμ]+$/i)?.[0] ?? ""; // 10kΩ: the netlist drops the Ω
  const warn = /^M/.test(tail) && !/^(meg|mil)/i.test(tail) ? "M is milli in SPICE: write Meg for mega"
    : /^F/.test(tail) ? "F is femto in SPICE (write 10u, not 10F)"
    : /^a/i.test(tail) ? "a is atto (1e-18) in SPICE: for amperes write the number alone (1, not 1A)"
    : undefined;
  return { text: `= ${si(n, unit).replace(/(\d)M/, "$1Meg")}`, warn }; // Meg, as SPICE writes mega
}

// ---------------------------------------------------------------- independent sources (V, I)

export type Func = "SINE" | "PULSE" | "EXP" | "SFFM" | "PWL";
/** One function argument; unit "y" is the source's own unit (V or A). `def` fills a gap before a
 *  given argument; `spice` is what SPICE uses when trailing arguments are left out (for the
 *  preview): a number, "inf" (PULSE width/period: no repetition) or "?" (depends on .tran). */
export interface Arg { label: string; unit: string; def: string; spice?: string }
export const FUNCS: Record<Exclude<Func, "PWL">, Arg[]> = {
  SINE: [
    { label: "Offset", unit: "y", def: "0", spice: "0" }, { label: "Amplitude", unit: "y", def: "1", spice: "0" }, { label: "Frequency", unit: "Hz", def: "1k", spice: "?" },
    { label: "Delay", unit: "s", def: "0", spice: "0" }, { label: "Damping", unit: "1/s", def: "0", spice: "0" }, { label: "Phase", unit: "°", def: "0", spice: "0" },
  ],
  PULSE: [
    { label: "Initial value", unit: "y", def: "0", spice: "0" }, { label: "Pulsed value", unit: "y", def: "5", spice: "0" }, { label: "Delay", unit: "s", def: "0", spice: "0" },
    { label: "Rise time", unit: "s", def: "1n", spice: "0" }, { label: "Fall time", unit: "s", def: "1n", spice: "0" }, { label: "Pulse width", unit: "s", def: "0.5m", spice: "inf" },
    { label: "Period", unit: "s", def: "1m", spice: "inf" },
  ],
  EXP: [
    { label: "Initial value", unit: "y", def: "0", spice: "0" }, { label: "Pulsed value", unit: "y", def: "1", spice: "0" }, { label: "Rise delay", unit: "s", def: "0", spice: "0" },
    { label: "Rise time constant", unit: "s", def: "0.1m", spice: "1f" }, { label: "Fall delay", unit: "s", def: "1m", spice: "inf" }, { label: "Fall time constant", unit: "s", def: "0.1m", spice: "1f" },
  ],
  SFFM: [
    { label: "Offset", unit: "y", def: "0", spice: "0" }, { label: "Amplitude", unit: "y", def: "1", spice: "0" }, { label: "Carrier frequency", unit: "Hz", def: "10k", spice: "?" },
    { label: "Modulation index", unit: "", def: "5", spice: "0" }, { label: "Signal frequency", unit: "Hz", def: "1k", spice: "?" },
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
  /** what the form does not know, kept verbatim where it was: in Value (e.g. a PWL's `r=0`, which
   *  must follow the function) and in Value2/SpiceLine (other instance parameters) */
  vrest: string;
  rest: string;
}

/** Index of the last argument that is filled in (-1: none); the ones after it are SPICE's defaults. */
export const lastFilled = (args: string[]) => args.reduce((n, a, i) => (a?.trim() ? i : n), -1);

/** Commas outside `{…}` as spaces: argument separators (`PWL(0,0 1m,1)`), not `{max(a,b)}`. */
const argCommas = (s: string) => {
  let d = 0, q = "", out = "";
  for (const c of s) {
    if (c === "'" || c === '"') q = !q ? c : q === c ? "" : q;
    else if (!q) d += c === "{" ? 1 : c === "}" ? -1 : 0;
    out += c === "," && d <= 0 && !q ? " " : c; // not inside {…} or '…'
  }
  return out;
};

/** Reads a source from its Value, Value2 and SpiceLine attributes (LTspice spreads it over all three). */
export function parseSource(...parts: (string | undefined)[]): Source {
  const s: Source = { dc: "", func: "", args: [], acMag: "", acPhase: "", rser: "", cpar: "", vrest: "", rest: "" };
  // tokens of Value first, then Value2 and SpiceLine; `inValue` tells where an unknown one goes
  const t = parts.flatMap((p, k) => tokens((p ?? "").replace(/\s*=\s*/g, "=")).map((w) => [w, k === 0] as const)); // `Rser = 50` is `Rser=50`
  const vrest: string[] = [], rest: string[] = [];
  for (let i = 0; i < t.length; i++) {
    const [w, inValue] = t[i];
    const fn = w.match(/^(SINE|SIN|PULSE|EXP|SFFM|PWL)\((.*)\)$/is);
    const kv = w.match(/^(rser|cpar)=(.+)$/i);
    if (fn && !s.func) {
      s.func = (fn[1].toUpperCase() === "SIN" ? "SINE" : fn[1].toUpperCase()) as Func;
      s.args = tokens(argCommas(fn[2]));
    } else if (/^dc$/i.test(w) && t[i + 1] && isNum(t[i + 1][0])) s.dc = t[++i][0];
    else if (/^ac$/i.test(w) && t[i + 1] && isNum(t[i + 1][0])) {
      s.acMag = t[++i][0];
      if (t[i + 1] && isNum(t[i + 1][0])) s.acPhase = t[++i][0];
    } else if (kv) s[kv[1].toLowerCase() as "rser" | "cpar"] = kv[2];
    else if (i === 0 && isNum(w)) s.dc = w;
    else (inValue ? vrest : rest).push(w);
  }
  [s.vrest, s.rest] = [vrest.join(" "), rest.join(" ")];
  return s;
}

/** Whether the source form writes these attributes back with the same meaning (token order,
 *  SIN/SINE, a leading DC and commas in argument lists do not matter); otherwise it opens as text. */
export function sourceRoundTrips(...parts: (string | undefined)[]): boolean {
  const norm = (s: string) => tokens(argCommas(s.replace(/\s*=\s*/g, "=")).replace(/\bsin\(/gi, "SINE(").replace(/(^|\s)dc\s+/gi, "$1").toUpperCase()).map((t) => t.replace(/\s+/g, " ")).sort().join(" ");
  const p = parseSource(...parts);
  // a waveform the form did not recognise (`SINE (…)`, `PWL file=…`, `DC PULSE(…)`) would show as DC,
  // and a stray number (`AC 1 2 3`) would become the DC value when written back: text instead
  if (/\(|\b(sine?|pulse|exp|sffm|pwl|am|trnoise|trrandom|dc)\b/i.test(p.vrest) || tokens(p.vrest).some(isNum)) return false;
  const f = formatSource(p);
  return norm(parts.filter(Boolean).join(" ") || "0") === norm(`${f.Value} ${f.Value2} ${f.SpiceLine}`); // nothing: DC 0
}

/** Function arguments for SPICE: gaps filled with defaults, trailing blanks dropped. */
function argList(func: Func, args: string[]) {
  if (func === "PWL") {
    // complete (time, value) rows only: a half-filled row must not shift the pairs after it
    const out: string[] = [];
    for (let i = 0; i < args.length; i += 2) if (args[i]?.trim() && args[i + 1]?.trim()) out.push(args[i].trim(), args[i + 1].trim());
    return out.length ? out : ["0", "0"]; // no complete point: 0 (`PWL()` does not parse)
  }
  const defs = FUNCS[func];
  const last = lastFilled(args);
  // arguments beyond the form's fields (PULSE cycles, LTspice's Ncycles ...) are kept as typed
  return Array.from({ length: Math.max(last + 1, 2) }, (_, i) => args[i]?.trim() || defs[i]?.def || "0");
}

/** The attributes LTspice writes: Value (DC and/or function), Value2 (AC), SpiceLine (parasitics). */
export function formatSource(s: Source): { Value: string; Value2: string; SpiceLine: string } {
  const fn = s.func ? `${s.func}(${argList(s.func, s.args).join(" ")})` : "";
  let value = [s.dc.trim() && (fn ? `DC ${s.dc.trim()}` : s.dc.trim()), fn, s.vrest.trim()].filter(Boolean).join(" ");
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
    // given arguments; a gap gets the form default (as written); trailing ones what SPICE does
    const last = lastFilled(s.args);
    // the first two are required: blank means the form's default (as argList writes it)
    const a = FUNCS[s.func].map((d, i) => (i <= last || i < 2 || d.spice === undefined ? num(s.args[i], d.def) : d.spice === "inf" ? Infinity : d.spice === "?" ? null : spiceNum(d.spice)));
    if (a.some((x) => x === null)) return null;
    const p = a as number[];
    if (s.func === "SINE") {
      const [off, amp, fr, td, th, ph] = p;
      end = td + (fr > 0 ? 3 / fr : 1);
      f = (t) => off + amp * (t < td ? Math.sin((ph * Math.PI) / 180) : Math.exp(-(t - td) * th) * Math.sin(2 * Math.PI * fr * (t - td) + (ph * Math.PI) / 180));
    } else if (s.func === "PULSE") {
      const [v1, v2, td, tr, tf, pw, per] = p;
      const one = tr + pw + tf;
      const periodic = isFinite(per) && per > 0;
      end = td + (periodic ? 2.2 * per : isFinite(one) && one > 0 ? 1.5 * one : td || 1);
      f = (t) => {
        if (t < td) return v1;
        const x = periodic ? (t - td) % per : t - td;
        return x < tr ? v1 + ((v2 - v1) * x) / tr : x < tr + pw ? v2 : x < one ? v2 + ((v1 - v2) * (x - tr - pw)) / tf : v1;
      };
    } else if (s.func === "EXP") {
      const [v1, v2, td1, tau1, td2, tau2] = p;
      end = isFinite(td2) ? Math.max(td2 + 5 * tau2, td1 + 5 * tau1, 1e-12) : Math.max(td1 + 5 * tau1, 2 * td1, 1e-12);
      const rise = (x: number, tau: number) => (x < 0 ? 0 : tau > 0 ? 1 - Math.exp(-x / tau) : 1); // tau 0: a step, not 0/0
      f = (t) => v1 + (v2 - v1) * rise(t - td1, tau1) + (v1 - v2) * rise(t - td2, tau2);
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
const KINDS = "tran|ac|dc|op|noise|tf|pz|sens";
export const ANALYSIS = new RegExp(`^\\.(${KINDS})\\b`, "i");
/** Display names, in the order of the simulation form's tabs and the result tabs. */
export const ANALYSES: [Analysis, string][] = [
  ["tran", "Transient"], ["ac", "AC"], ["dc", "DC sweep"], ["noise", "Noise"], ["op", "DC op pnt"],
  ["tf", "Transfer fn"], ["pz", "Poles/zeros"], ["sens", "Sensitivity"],
];

/** The analyses a schematic offers (one per kind, for the result tabs): its active analysis
 *  directives and the commented-out alternatives (LTspice keeps those as `;.ac …` comments).
 *  Per kind the active one wins, else the first written; `line` includes its `+` lines. */
export function analysisOptions(texts: { text: string; directive?: boolean }[]) {
  const found: { kind: Analysis; line: string; active: boolean }[] = [];
  for (const { text, directive } of texts) {
    const lines = text.split("\n");
    // a comment is an alternative only if it holds nothing but analyses (LTspice's `;.ac …`), not
    // a note that mentions one ("Try this later:\n.tran 5m")
    if (!directive && !lines.every((l) => !l.trim() || l.trim().startsWith("+") || (ANALYSIS.test(l.trim()) && plausible(l.trim())))) continue;
    lines.forEach((l, i) => {
      if (!ANALYSIS.test(l.trim()) || (!directive && !plausible(l.trim()))) return;
      const more = continuation(lines, i).map((j) => lines[j].trim());
      found.push({ kind: parseAnalysis(l.trim()).kind, line: [l.trim(), ...more].join("\n"), active: !!directive });
    });
  }
  return ANALYSES.flatMap(([k]) => found.find((f) => f.kind === k && f.active) ?? found.find((f) => f.kind === k) ?? []);
}

/** A comment that reads as an analysis (an alternative), not a note that starts with `.op` … */
function plausible(line: string) {
  const { kind, f } = parseAnalysis(line);
  const num = (k: string) => !!f[k] && (spiceNum(f[k]) !== null || /^\{.*\}$/.test(f[k]));
  const words = (f.rest ?? "").split(/\s+/).filter(Boolean);
  switch (kind) {
    case "op": return !words.length;
    case "tran": return num("stop") && words.every((w) => /^(startup|steady|nodiscard|step|uic)$/i.test(w));
    case "ac": case "noise": return num("fstart") && num("fstop");
    case "dc": return num("start") && num("stop");
  }
  const t = tokens(line).slice(1); // .tf/.pz/.sens: an output V(…)/I(…), or the four nodes and vol|cur
  return kind === "pz" ? t.length >= 5 && /^(vol|cur)$/i.test(t[4]) && (!t[5] || /^(pol|zer|pz)$/i.test(t[5])) : /^[vi]\(/i.test(t[0] ?? "") && (kind !== "tf" || t.length >= 2);
}

/** The line holding the analysis of `kind` (else the first), as the result tabs read the text:
 *  in a comment only lines that read as an analysis count. -1: none. */
export function analysisLineIndex(lines: string[], kind: Analysis | null | undefined, directive: boolean) {
  const ok = (l: string) => ANALYSIS.test(l.trim()) && (directive || plausible(l.trim()));
  const own = lines.findIndex((l) => ok(l) && parseAnalysis(l.trim()).kind === kind);
  return own >= 0 ? own : lines.findIndex(ok);
}

/** Every ngspice analysis, also those the form does not offer (.sp, .disto, .pss, .hb). */
export const ANY_ANALYSIS = new RegExp(`^\\.(${KINDS}|sp|disto|pss|hb)\\b`, "i");

/** `text` without its analysis statements (and their `+` lines): a result tab runs its own instead. */
export function stripAnalyses(text: string) {
  const lines = text.split("\n"), drop = new Set<number>();
  lines.forEach((l, i) => ANY_ANALYSIS.test(l.trim()) && [i, ...continuation(lines, i)].forEach((j) => drop.add(j)));
  return lines.filter((_, i) => !drop.has(i)).join("\n");
}

/** The lines after `li` that continue its statement: its `+` lines (blank and `*` lines between
 *  them are skipped, as SPICE does). */
export function continuation(lines: string[], li: number) {
  const out: number[] = [];
  for (let j = li + 1; j < lines.length; j++) {
    const t = lines[j].trim();
    if (t.startsWith("+")) out.push(j);
    else if (t && !t.startsWith("*")) break;
  }
  return out;
}

/** The first analysis line of a directive text (the one the simulation dialog edits). */
export const analysisIndex = (lines: string[]) => analysisLineIndex(lines, null, true);
export const analysisLine = (text: string) => { const l = text.split("\n"); return l[analysisIndex(l)] as string | undefined; };
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
      const ac = /^ac$/i.test(t[1] ?? ""), dc = /^dc$/i.test(t[1] ?? ""); // `dc` may be written out
      return { kind, f: { ...parseOut(t[0]), mode: ac ? "ac" : dc ? "dc" : "", sweep: ac ? (t[2] ?? "dec").toLowerCase() : "dec", pts: ac ? t[3] ?? "" : "", fstart: ac ? t[4] ?? "" : "", fstop: ac ? t[5] ?? "" : "", rest: after(ac ? 6 : dc ? 2 : 1) } };
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
    sens: () => `.sens ${formatOut(f)}${v("mode") === "ac" ? ` ac ${sweep("10")}` : v("mode") === "dc" ? " dc" : ""}`,
  }[kind]();
  return [line, v("rest")].filter(Boolean).join(" ");
}

// ---------------------------------------------------------------- other directives

export type DirectiveKind = "param" | "model" | "meas" | "ic" | "temp" | "include" | "text";

/** A line in comparable form: case, quotes, spaces around `=` and runs of spaces do not matter. */
export const sameSpice = (a: string, b: string) => {
  const n = (l: string) => l.toLowerCase().replace(/["']/g, "").replace(/\s*=\s*/g, "=").replace(/^\s*\.inc(lude)?\b/, ".include").replace(/^\s*\.measure\b/, ".meas").replace(/\s+/g, " ").trim();
  return n(a) === n(b);
};

/** The line a form would write for `line` (null: the form cannot read it). */
function viaForm(kind: DirectiveKind, line: string): string | null {
  switch (kind) {
    case "param": return formatParams(parseParams(line));
    case "model": { const m = parseModel(line); return m && formatModel(m.name, m.type, m.params); }
    case "meas": { const m = parseMeas(line); return m && formatMeas(m); }
    case "ic": return formatIc(parseIc(line));
    case "temp": return `.temp ${line.trim().split(/\s+/)[1] ?? ""}`;
    case "include": { const i = parseInclude(line); return formatInclude(i.lib, i.path, i.section); }
    default: return null;
  }
}

/** Which form edits this directive: one line of a known kind that the form writes back unchanged
 *  (an inline comment, `.ic I(L1)=…`, a `.meas … TD=1m` the form does not show: "text"). */
export function directiveKind(text: string): DirectiveKind {
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  if (lines.length !== 1) return "text";
  const l = lines[0], low = l.toLowerCase();
  const kind: DirectiveKind = low.startsWith(".param ") ? "param" : low.startsWith(".model ") ? "model" : /^\.meas(ure)?\s/.test(low) ? "meas"
    : low.startsWith(".ic ") ? "ic" : low.startsWith(".temp ") ? "temp" : /^\.(inc|include|lib)\s/.test(low) ? "include" : "text";
  const back = viaForm(kind, l);
  return back !== null && sameSpice(back, l) ? kind : "text";
}

/** The sweeps the simulation form has (dec, oct, lin); an analysis without one counts as known. */
export const SWEEP_KINDS = ["dec", "oct", "lin"];
export const knownSweep = (f: Fields) => f.sweep === undefined || SWEEP_KINDS.includes(f.sweep);

/** Whether the simulation dialog writes this analysis line back unchanged. */
export const analysisRoundTrips = (line: string) => {
  const { kind, f } = parseAnalysis(line);
  if (!knownSweep(f)) return false; // e.g. LTspice `.ac list …`
  return sameSpice(formatAnalysis(kind, f), line);
};

/** Whether the simulation form can edit `text`'s analysis: it writes the line back unchanged and
 *  no `+` line continues it (the form would not see the rest). */
export const analysisEditable = (text: string) => { const lines = text.split("\n"); return analysisEditableAt(lines, analysisIndex(lines)); };
/** The same for the analysis on line `li` (a text may hold several). */
export const analysisEditableAt = (lines: string[], li: number) => li >= 0 && analysisRoundTrips(lines[li]) && !continuation(lines, li).length;

/** Include paths with forward slashes: portable, and safe from the .asc `\n` newline escape. */
export const portablePaths = (text: string) => text.split("\n").map((l) => (/^\s*\.(inc|lib)/i.test(l) ? l.replaceAll("\\", "/") : l)).join("\n");

/** `name = value` pairs, spaces around `=` allowed. */
const pairs = (s: string) => tokens(s.replace(/\s*=\s*/g, "=")).map((p) => { const k = p.indexOf("="); return k > 0 ? [p.slice(0, k), p.slice(k + 1)] : [p, ""]; });

export const parseParams = (line: string) => pairs(line.replace(/^\s*\.param\s*/i, ""));
export const formatParams = (rows: string[][]) => `.param ${rows.filter(([n]) => n.trim()).map(([n, v]) => `${n.trim()}=${v.trim() || "0"}`).join(" ")}`;

export const parseIc = (line: string) => [...line.matchAll(/v\(\s*([^)\s]+)\s*\)\s*=\s*(\S+)/gi)].map((m) => [m[1], m[2]]);
export const formatIc = (rows: string[][]) => `.ic ${rows.filter(([n]) => n.trim()).map(([n, v]) => `V(${n.trim()})=${v.trim() || "0"}`).join(" ")}`;

export function parseInclude(line: string) {
  const m = line.trim().match(/^\.(inc\w*|lib)\s+("([^"]*)"|'([^']*)'|(\S+))\s*(\S*)/i);
  return { lib: !!m && m[1].toLowerCase() === "lib", path: m ? (m[3] ?? m[4] ?? m[5]) : "", section: m?.[6] ?? "" };
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
  const edge = (from: number, to = t.length) => { const e = t.slice(from, to).find((x) => /^(rise|fall|cross)=/i.test(x)); return e ? { edge: e.split("=")[0].toLowerCase(), n: e.split("=")[1] } : { edge: "", n: "" }; };
  if (!["tran", "ac", "dc"].includes(an)) return null;
  if (["AVG", "RMS", "MIN", "MAX", "PP", "INTEG"].includes(op)) return { an, name, type: "stat", f: { op, expr: t[4] ?? "", from: kv("FROM"), to: kv("TO") } };
  if (op === "FIND") return { an, name, type: "find", f: { expr: t[4] ?? "", at: kv("AT") } };
  if (op === "WHEN") { const [expr, val] = (t[4] ?? "").split("="); return { an, name, type: "when", f: { expr, val: val ?? "", ...edge(5) } }; }
  if (op === "TRIG") {
    const k = t.findIndex((x) => x.toUpperCase() === "TARG");
    if (k < 0) return null;
    const side = (from: number, to: number) => ({ expr: t[from + 1] ?? "", val: t.slice(from, to).find((x) => /^val=/i.test(x))?.slice(4) ?? "", ...edge(from, to) });
    const [a, b] = [side(3, k), side(k, t.length)];
    return { an, name, type: "delay", f: { expr: a.expr, val: a.val, edge: a.edge, n: a.n, expr2: b.expr, val2: b.val, edge2: b.edge, n2: b.n } };
  }
  return null;
}
export function formatMeas(m: Meas): string {
  const v = (k: string, d = "") => m.f[k]?.trim() || d;
  const edge = (e: string, n: string) => (v(e) ? ` ${v(e).toUpperCase()}=${v(n, "1")}` : ""); // none: the first crossing
  const body = {
    stat: () => [v("op", "AVG"), v("expr", "V(out)"), v("from") && `FROM=${v("from")}`, v("to") && `TO=${v("to")}`].filter(Boolean).join(" "),
    find: () => `FIND ${v("expr", "V(out)")} AT=${v("at", "1m")}`,
    when: () => `WHEN ${v("expr", "V(out)")}=${v("val", "0")}${edge("edge", "n")}`,
    delay: () => `TRIG ${v("expr", "V(in)")} VAL=${v("val", "0.5")}${edge("edge", "n")} TARG ${v("expr2", "V(out)")} VAL=${v("val2", "0.5")}${edge("edge2", "n2")}`,
  }[m.type]();
  return `.meas ${m.an} ${m.name.trim() || "result"} ${body}`;
}

// ---------------------------------------------------------------- sampled results

/** Trapezoid integral of y over [lo, hi] (samples inside, ends interpolated), its average and RMS. */
export function integrate(xs: number[], ys: (number | null)[], lo: number, hi: number): { area: number; avg: number; rms: number } {
  const n = xs.length;
  if (n > 1 && xs[n - 1] < xs[0]) return integrate([...xs].reverse(), [...ys].reverse(), lo, hi); // a sweep running downwards
  // the samples strictly inside (lo, hi): found by bisection, not a scan (stats follow every zoom)
  const first = (x: number, strict: boolean) => { let a = 0, b = n; while (a < b) { const m = (a + b) >> 1; if (strict ? xs[m] <= x : xs[m] < x) a = m + 1; else b = m; } return a; };
  const pts: [number, number][] = [[lo, at(xs, ys, lo)]];
  for (let i = first(lo, true), end = first(hi, false); i < end; i++) pts.push([xs[i], ys[i] ?? NaN]); // a gap's segments are left out
  pts.push([hi, at(xs, ys, hi)]);
  // average and RMS over the part with data (a gap, or a range beyond the run, does not count as 0)
  let area = 0, sq = 0, span = 0;
  for (let i = 1; i < pts.length; i++) {
    const [[x0, y0], [x1, y1]] = [pts[i - 1], pts[i]];
    if (Number.isNaN(y0) || Number.isNaN(y1)) continue;
    area += ((y0 + y1) / 2) * (x1 - x0);
    sq += ((y0 * y0 + y0 * y1 + y1 * y1) / 3) * (x1 - x0); // exact for a straight segment
    span += x1 - x0;
  }
  return { area, avg: span ? area / span : NaN, rms: span ? Math.sqrt(sq / span) : NaN };
}

/** y at x on a sampled curve (linear between samples; NaN outside or at a gap). */
export function at(xs: number[], ys: (number | null)[], x: number) {
  if (!xs.length || x < Math.min(xs[0], xs[xs.length - 1]) || x > Math.max(xs[0], xs[xs.length - 1])) return NaN;
  let lo = 0, hi = xs.length - 1;
  const up = xs[hi] >= xs[0];
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (xs[mid] <= x === up) lo = mid; else hi = mid;
  }
  const f = xs[hi] === xs[lo] ? 0 : (x - xs[lo]) / (xs[hi] - xs[lo]);
  const [a, b] = [ys[lo], ys[hi]];
  return a == null || b == null ? NaN : a + Math.max(0, Math.min(1, f)) * (b - a);
}
