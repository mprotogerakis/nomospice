// Pole-zero map of a .pz analysis in the s-plane, and the root locus: the poles' paths while one
// parameter (.param or an R/C/L value) is swept, one .pz run per step (root_locus in main.rs).
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Run } from "./types";
import { currentNets, dirOf, docId, useS } from "./store";
import { parseParams, si, spiceNum } from "./spice";
import { defOf } from "./geom";
import { NumField, Select, TextField } from "./ui/dialog";
import { Button } from "./ui/button";
import { cn } from "@/lib/utils";

type P = { re: number; im: number };
type Hit = P & { kind: "pole" | "zero" | "locus"; param?: number };
const dist = (a: P, b: P) => Math.hypot(a.re - b.re, a.im - b.im);
const lastOf = <T,>(a: (T | null)[]) => { for (let i = a.length - 1; i >= 0; i--) if (a[i]) return a[i]; return null; };
/** ω₀ = |s|, f₀ = ω₀/2π, ζ = −σ/|s| (for the readout and the table) */
const describe = (p: P) => {
  const w = Math.hypot(p.re, p.im);
  return { s: `${si(p.re, "", 4)} ${p.im < 0 ? "−" : "+"} j${si(Math.abs(p.im), "", 4)}`, f: si(w / (2 * Math.PI), "Hz", 3), zeta: w ? (-p.re / w).toFixed(3) : "–" };
};

/** Poles per step -> continuous branches (greedy nearest neighbour), null where a branch is missing. */
function branches(steps: P[][]): (P | null)[][] {
  const out: (P | null)[][] = [];
  steps.forEach((cur, k) => {
    const last = out.map(lastOf);
    const pairs: [number, number, number][] = [];
    last.forEach((p, i) => p && cur.forEach((c, j) => pairs.push([dist(p, c), i, j])));
    pairs.sort((a, b) => a[0] - b[0]);
    const bi = new Set<number>(), cj = new Set<number>();
    for (const [, i, j] of pairs) if (!bi.has(i) && !cj.has(j)) (out[i].push(cur[j]), bi.add(i), cj.add(j));
    out.forEach((b, i) => !bi.has(i) && b.push(null));
    cur.forEach((c, j) => !cj.has(j) && out.push([...Array<null>(k).fill(null), c]));
  });
  return out;
}

/** Tick step for a span: 1, 2 or 5 × 10^n, about six ticks. */
const tickStep = (span: number) => { const e = 10 ** Math.floor(Math.log10(span / 6)); return [1, 2, 5, 10].map((m) => m * e).find((s) => span / s <= 8) ?? 10 * e; };

export const PoleZero = memo(function PoleZero({ run }: { run: Run }) {
  const poles = useMemo(() => run.op.filter((o) => o[0].startsWith("pole")).map(([, re, im]) => ({ re, im })), [run]);
  const zeros = useMemo(() => run.op.filter((o) => o[0].startsWith("zero")).map(([, re, im]) => ({ re, im })), [run]);
  const [locus, setLocus] = useState<{ values: number[]; steps: P[][]; target: string } | null>(null);
  const paths = useMemo(() => (locus ? branches(locus.steps) : []), [locus]);
  // a new run (edited circuit, live mode): an old locus and zoom would describe another circuit
  useEffect(() => (setLocus(null), setView(null)), [run]);

  // view: equal scale on both axes (angles = damping), wheel zoom around the cursor
  const host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const el = host.current!;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const fit = useMemo(() => {
    const all = [...poles, ...zeros, ...paths.flat().filter((p): p is P => !!p), { re: 0, im: 0 }];
    let [x0, x1] = [Math.min(...all.map((p) => p.re)), Math.max(...all.map((p) => p.re))];
    let [y0, y1] = [Math.min(...all.map((p) => p.im)), Math.max(...all.map((p) => p.im))];
    const span = Math.max(x1 - x0, y1 - y0) || 1;
    [x0, x1, y0, y1] = [x0 - 0.15 * span, x1 + 0.15 * span, y0 - 0.15 * span, y1 + 0.15 * span];
    return { cx: (x0 + x1) / 2, cy: (y0 + y1) / 2, sx: x1 - x0, sy: y1 - y0 };
  }, [poles, zeros, paths]);
  const [view, setView] = useState<{ cx: number; cy: number; k: number } | null>(null);
  const P = Math.min(28, size.w / 4, size.h / 4); // margin for the tick labels (less in a tiny pane)
  const base = Math.max(Math.min((size.w - 2 * P) / fit.sx, (size.h - 2 * P) / fit.sy), 1e-30); // never negative: that mirrors the plot
  const v = view ?? { cx: fit.cx, cy: fit.cy, k: base };
  const px = (p: P) => [size.w / 2 + (p.re - v.cx) * v.k, size.h / 2 - (p.im - v.cy) * v.k];
  const val = (x: number, y: number): P => ({ re: v.cx + (x - size.w / 2) / v.k, im: v.cy - (y - size.h / 2) / v.k });

  // hover: the nearest pole, zero or locus point within 14 px
  const [hit, setHit] = useState<(Hit & { x: number; y: number }) | null>(null);
  const hits = useMemo<Hit[]>(() => [
    ...poles.map((p) => ({ ...p, kind: "pole" as const })), ...zeros.map((p) => ({ ...p, kind: "zero" as const })),
    ...paths.flatMap((b) => b.flatMap((p, k) => (p ? [{ ...p, kind: "locus" as const, param: locus!.values[k] }] : []))),
  ], [poles, zeros, paths, locus]);
  const onMove = (e: React.MouseEvent) => {
    const r = host.current!.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top;
    let best: Hit | null = null, bd = 14;
    for (const h of hits) { const [hx, hy] = px(h); const d = Math.hypot(hx - x, hy - y); if (d < bd) (best = h, bd = d); }
    setHit(best && { ...best, x, y });
  };
  const onWheel = (e: React.WheelEvent) => {
    const r = host.current!.getBoundingClientRect(), at = val(e.clientX - r.left, e.clientY - r.top);
    const f = Math.exp(-e.deltaY * 0.002), k = v.k * f;
    if (f > 1 && size.w / k < Math.max(Math.abs(v.cx), Math.abs(v.cy)) * 1e-6) return; // no deeper than a millionth of the values
    setView({ k, cx: at.re - (at.re - v.cx) / f, cy: at.im - (at.im - v.cy) / f });
  };

  // grid and axes
  const lo = val(0, size.h), hi = val(size.w, 0);
  const step = tickStep(Math.max(hi.re - lo.re, hi.im - lo.im));
  // from an index, not by adding up steps (which stalls once step < the float spacing of x)
  const ticks = (a: number, b: number) => {
    const first = Math.ceil(a / step), n = Math.floor(b / step) - first;
    if (!isFinite(n) || n < 0 || n > 200) return [];
    return Array.from({ length: n + 1 }, (_, i) => { const x = (first + i) * step; return Math.abs(x) < step * 1e-9 ? 0 : x; });
  };
  const [ox, oy] = px({ re: 0, im: 0 });
  const mark = 6;

  return (
    <div className="flex h-full min-h-0 bg-surface-schematic-canvas">
      <div ref={host} className="relative min-w-0 flex-1 overflow-hidden" onMouseMove={onMove} onMouseLeave={() => setHit(null)} onWheel={onWheel} onDoubleClick={() => setView(null)}>
        {size.w > 0 && (
          <svg width={size.w} height={size.h} className="block text-text-secondary">
            <rect x={Math.max(ox, 0)} y={0} width={Math.max(size.w - Math.max(ox, 0), 0)} height={size.h} fill="var(--status-danger-soft)" />
            {ticks(lo.re, hi.re).map((t) => { const [x] = px({ re: t, im: 0 }); return <g key={`x${t}`}><line x1={x} x2={x} y1={0} y2={size.h} stroke="var(--border)" /><text x={x + 3} y={size.h - 6} fontSize={10} fill="currentColor" className="font-mono">{si(t, "", 3)}</text></g>; })}
            {ticks(lo.im, hi.im).map((t) => { const [, y] = px({ re: 0, im: t }); return <g key={`y${t}`}><line x1={0} x2={size.w} y1={y} y2={y} stroke="var(--border)" /><text x={4} y={y - 3} fontSize={10} fill="currentColor" className="font-mono">{t ? `j${si(t, "", 3)}` : "0"}</text></g>; })}
            <line x1={0} x2={size.w} y1={oy} y2={oy} stroke="var(--text-tertiary)" />
            <line x1={ox} x2={ox} y1={0} y2={size.h} stroke="var(--text-tertiary)" strokeWidth={1.5} />
            <text x={size.w - 8} y={oy - 6} fontSize={11} textAnchor="end" fill="currentColor">σ (1/s)</text>
            <text x={ox + 6} y={14} fontSize={11} fill="currentColor">jω (rad/s)</text>
            {ox < size.w - 70 && <text x={size.w - 8} y={14} fontSize={10} textAnchor="end" fill="var(--status-danger)">unstable</text>}
            {paths.map((b, i) => {
              const segs: string[] = [];
              let cur = "";
              b.forEach((p) => { if (p) { const [x, y] = px(p); cur += `${cur ? "L" : "M"}${x},${y}`; } else if (cur) (segs.push(cur), (cur = "")); });
              if (cur) segs.push(cur);
              const first = b.find((p) => p), last = lastOf(b);
              return (
                <g key={i} stroke="var(--series-2)" fill="none" strokeWidth={1.6}>
                  {segs.map((d, j) => <path key={j} d={d} />)}
                  {first && (() => { const [x, y] = px(first); return <circle cx={x} cy={y} r={3} fill="var(--series-2)" />; })()}
                  {last && (() => { const [x, y] = px(last); return <rect x={x - 3} y={y - 3} width={6} height={6} fill="var(--surface-schematic-canvas)" />; })()}
                </g>
              );
            })}
            {zeros.map((p, i) => { const [x, y] = px(p); return <circle key={`z${i}`} cx={x} cy={y} r={mark} fill="none" stroke="var(--series-1)" strokeWidth={2} />; })}
            {poles.map((p, i) => { const [x, y] = px(p); return <path key={`p${i}`} d={`M${x - mark},${y - mark}L${x + mark},${y + mark}M${x - mark},${y + mark}L${x + mark},${y - mark}`} stroke="var(--series-1)" strokeWidth={2} />; })}
          </svg>
        )}
        {hit && (
          <div className="pointer-events-none absolute rounded-control border border-border bg-surface-panel px-2 py-1 font-mono text-2xs text-text shadow-float" style={{ left: Math.min(hit.x + 12, size.w - 220), top: Math.max(hit.y - 64, 4) }}>
            <div className="text-text-tertiary">{hit.kind === "locus" ? `${locus?.target} = ${si(hit.param!, "", 4)}` : hit.kind}</div>
            <div>s = {describe(hit).s} rad/s</div>
            <div>f₀ = {describe(hit).f} · ζ = {describe(hit).zeta}</div>
          </div>
        )}
        {!poles.length && !zeros.length && <div className="absolute inset-0 flex items-center justify-center text-xs text-text-tertiary">No poles or zeros found: check the .pz input and output nodes (see Log).</div>}
      </div>
      <Sidebar poles={poles} zeros={zeros} locus={locus} setLocus={setLocus} />
    </div>
  );
});

function Sidebar({ poles, zeros, locus, setLocus }: { poles: P[]; zeros: P[]; locus: unknown; setLocus: (l: { values: number[]; steps: P[][]; target: string } | null) => void }) {
  const items = useS((s) => s.items);
  const lib = useS((s) => s.lib);
  const nets = useS(currentNets); // null while the netlist lags the items: no index-aligned names then
  const running = useS((s) => s.running);
  // sweepable: .param names and R/C/L instances
  const targets = useMemo(() => [...new Set([
    ...items.flatMap((it) => (it.t === "text" && it.directive ? it.text.split("\n") : [])).filter((l) => /^\s*\.param\b/i.test(l))
      .flatMap((l) => parseParams(l).map(([n]) => n).filter((n) => /^[a-z_]\w*$/i.test(n))),
    ...items.flatMap((it, i) => (it.t === "sym" && "RCL".includes(defOf(lib, it)?.prefix?.[0]?.toUpperCase() ?? "-") ? [nets?.insts[i] || it.attrs.InstName || ""] : [])).filter(Boolean),
  ])], [items, lib, nets]);
  const [f, setF] = useState({ target: targets[0] ?? "", from: "", to: "", steps: "40", scale: "log" });
  const [error, setError] = useState("");

  const compute = async () => {
    const [a, b, n] = [spiceNum(f.from), spiceNum(f.to), Math.round(spiceNum(f.steps) ?? 0)];
    if (!f.target.trim() || a === null || b === null || n < 2) return setError("Enter a parameter, a range and at least 2 steps.");
    if (f.scale === "log" && (a <= 0 || b <= 0)) return setError("A log sweep needs positive values.");
    if (n > 500) return setError("At most 500 steps (each is one .pz run).");
    // the locus is drawn over this run's poles: it must be the same circuit
    if (useS.getState().stale) return setError("The schematic changed since this run: run it again first.");
    const gen = docId();
    const values = Array.from({ length: n }, (_, i) => (f.scale === "log" ? a * (b / a) ** (i / (n - 1)) : a + ((b - a) * i) / (n - 1)));
    setError("");
    useS.setState({ running: true });
    try {
      const { items, path } = useS.getState();
      // a step is null where .pz failed (drawn as a gap); fewer steps than values: halted
      const steps = await invoke<([string, number, number][] | null)[]>("root_locus", { items, dir: dirOf(path), target: f.target.trim(), values });
      if (docId() !== gen) return; // another document was opened meanwhile
      const failed = steps.filter((x) => !x).length;
      setError([failed && `${failed} of ${steps.length} steps found no poles (gaps in the paths).`, steps.length < values.length && `Halted after ${steps.length} of ${values.length} steps.`].filter(Boolean).join(" "));
      setLocus({ values: values.slice(0, steps.length), target: f.target.trim(), steps: steps.map((s) => (s ?? []).filter(([n]) => n.startsWith("pole")).map(([, re, im]) => ({ re, im }))) });
    } catch (e) {
      setError(String(e));
    } finally {
      useS.setState({ running: false });
    }
  };
  // conjugate pairs once: σ ± jω
  // real if the imaginary part is numerical noise (ngspice returns -1e4 - j1e-12 for a real pole)
  const real = (p: P) => Math.abs(p.im) <= 1e-9 * Math.max(Math.abs(p.re), Math.abs(p.im), 1e-300);
  const rows = (list: P[], kind: string) => list.filter((p) => real(p) || p.im > 0).map((p, i) => ({ p: real(p) ? { ...p, im: 0 } : p, kind, pair: !real(p), key: `${kind}${i}` }));
  return (
    <div className="flex w-[340px] shrink-0 flex-col gap-2 overflow-y-auto border-l border-border bg-surface-panel p-2 text-xs">
      <div className="font-medium text-text-strong">Poles and zeros</div>
      <table className="w-full table-fixed font-mono text-2xs">
        <thead className="text-left text-text-tertiary"><tr><th className="w-10 font-normal" /><th className="font-normal">s (rad/s)</th><th className="w-16 font-normal">f₀</th><th className="w-12 font-normal">ζ</th></tr></thead>
        <tbody>
          {[...rows(poles, "×"), ...rows(zeros, "○")].map(({ p, kind, pair, key }) => {
            const d = describe(p);
            return (
              <tr key={key} className="border-t border-border-subtle">
                <td className="py-0.5" style={{ color: "var(--series-1)" }}>{kind}</td>
                <td className="truncate">{si(p.re, "", 4)}{pair ? ` ± j${si(p.im, "", 4)}` : ""}</td>
                <td>{d.f}</td>
                <td className={cn(p.re > 0 && "text-status-danger")}>{d.zeta}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className="mt-2 font-medium text-text-strong">Root locus</div>
      <p className="text-2xs leading-4 text-text-tertiary">Sweeps a .param or an R/C/L value and draws the poles' paths (● start, ■ end). Each step is one .pz run.</p>
      <datalist id="locus-targets">{targets.map((t) => <option key={t} value={t} />)}</datalist>
      <TextField compact label="Parameter" value={f.target} set={(target) => setF({ ...f, target })} list="locus-targets" placeholder="R1 or a .param" hint="a .param wins over a part of the same name" />
      <NumField compact label="From" value={f.from} set={(from) => setF({ ...f, from })} placeholder="100" />
      <NumField compact label="To" value={f.to} set={(to) => setF({ ...f, to })} placeholder="100k" />
      <NumField compact label="Steps" value={f.steps} set={(steps) => setF({ ...f, steps })} />
      <Select compact label="Spacing" value={f.scale} set={(scale) => setF({ ...f, scale })} options={[["log", "logarithmic"], ["lin", "linear"]]} />
      <div className="flex gap-2">
        <Button type="button" variant="primary" disabled={running} onClick={compute}>{running ? "Computing…" : "Draw root locus"}</Button>
        {!!locus && <Button type="button" onClick={() => setLocus(null)}>Clear</Button>}
      </div>
      {error && <p className="whitespace-pre-wrap text-2xs text-status-danger">{error}</p>}
      <p className="mt-auto text-2xs leading-4 text-text-tertiary">Wheel: zoom · double-click: fit · hover: s, f₀ = |s|/2π, ζ = −σ/|s| · shaded: right half-plane (unstable)</p>
    </div>
  );
}
