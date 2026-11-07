// Nyquist and Nichols plots of .ac results: each curve runs through the complex plane (Nyquist:
// Im over Re) or the gain-phase plane (Nichols: dB over degrees) with the frequency as its
// parameter; the critical point -1 (0 dB at -180°) marked, where a loop gain must not encircle.
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { si } from "./spice";
import { tickStep } from "./PoleZero";
import { useSize } from "./lib/useSize";

/** A curve: its points (x, y) at the frequencies `freqs` (null: a gap). */
export type Curve = { label: string; slot: number; x: (number | null)[]; y: (number | null)[] };
type Hit = { c: number; i: number };

export const Polar = memo(function Polar({ curves, freqs, mode, onHover }: { curves: Curve[]; freqs: number[]; mode: "nyquist" | "nichols"; onHover: (f: number | null) => void }) {
  const host = useRef<HTMLDivElement>(null);
  const size = useSize(host);
  const nyq = mode === "nyquist";
  const crit = nyq ? { x: -1, y: 0 } : { x: -180, y: 0 };
  // the bounds of every curve and the critical point (Nyquist: the origin too)
  const fit = useMemo(() => {
    let [x0, x1, y0, y1] = [crit.x, crit.x, crit.y, crit.y];
    if (nyq) [x1, y0, y1] = [Math.max(x1, 0), Math.min(y0, 0), Math.max(y1, 0)];
    for (const c of curves) c.x.forEach((x, i) => {
      const y = c.y[i];
      if (x == null || y == null || !Number.isFinite(x) || !Number.isFinite(y)) return;
      [x0, x1, y0, y1] = [Math.min(x0, x), Math.max(x1, x), Math.min(y0, y), Math.max(y1, y)];
    });
    const [sx, sy] = [x1 - x0 || 1, y1 - y0 || 1];
    return { x0: x0 - 0.08 * sx, x1: x1 + 0.08 * sx, y0: y0 - 0.08 * sy, y1: y1 + 0.08 * sy };
  }, [curves, nyq, crit.x, crit.y]);
  // view: Nyquist at one scale on both axes (angles and circles true), Nichols each its own;
  // wheel zooms around the cursor, double-click fits again
  const [view, setView] = useState<{ cx: number; cy: number; kx: number; ky: number } | null>(null);
  useEffect(() => setView(null), [mode]);
  const margin = 32;
  const base = useMemo(() => {
    const kx = Math.max((size.w - 2 * margin) / (fit.x1 - fit.x0), 1e-30), ky = Math.max((size.h - 2 * margin) / (fit.y1 - fit.y0), 1e-30);
    const k = Math.min(kx, ky);
    return { cx: (fit.x0 + fit.x1) / 2, cy: (fit.y0 + fit.y1) / 2, kx: nyq ? k : kx, ky: nyq ? k : ky };
  }, [fit, size.w, size.h, nyq]);
  const v = view ?? base;
  const px = (x: number, y: number) => [size.w / 2 + (x - v.cx) * v.kx, size.h / 2 - (y - v.cy) * v.ky];
  const val = (X: number, Y: number) => ({ x: v.cx + (X - size.w / 2) / v.kx, y: v.cy - (Y - size.h / 2) / v.ky });

  const drawn = useMemo(() => curves.map((c) => {
    let d = "", pen = false;
    c.x.forEach((x, i) => {
      const y = c.y[i];
      if (x == null || y == null || !Number.isFinite(x) || !Number.isFinite(y)) return void (pen = false);
      const [X, Y] = px(x, y);
      d += `${pen ? "L" : "M"}${X.toFixed(1)},${Y.toFixed(1)}`;
      pen = true;
    });
    return d;
  }), [curves, v.cx, v.cy, v.kx, v.ky, size.w, size.h]); // eslint-disable-line react-hooks/exhaustive-deps

  // hover: the nearest point within 14 px, its frequency to the trace table
  const [hit, setHit] = useState<Hit | null>(null);
  const onMove = (e: React.MouseEvent) => {
    const r = host.current!.getBoundingClientRect(), X = e.clientX - r.left, Y = e.clientY - r.top;
    let best: Hit | null = null, bd = 14;
    curves.forEach((c, ci) => c.x.forEach((x, i) => {
      const y = c.y[i];
      if (x == null || y == null) return;
      const [hx, hy] = px(x, y), d = Math.hypot(hx - X, hy - Y);
      if (d < bd) (best = { c: ci, i }), (bd = d);
    }));
    const h = best as Hit | null;
    if (h?.c !== hit?.c || h?.i !== hit?.i) {
      setHit(h);
      onHover(h ? freqs[h.i] ?? null : null);
    }
  };
  const onWheel = (e: React.WheelEvent) => {
    const r = host.current!.getBoundingClientRect(), at = val(e.clientX - r.left, e.clientY - r.top);
    const f = Math.exp(-e.deltaY * 0.002);
    if ((f > 1 && v.kx > base.kx * 1e6) || (f < 1 && v.kx < base.kx * 1e-3)) return;
    setView({ kx: v.kx * f, ky: v.ky * f, cx: at.x - (at.x - v.cx) / f, cy: at.y - (at.y - v.cy) / f });
  };

  // grid
  const lo = val(0, size.h), hi = val(size.w, 0);
  const ticks = (a: number, b: number) => {
    const step = tickStep(b - a), first = Math.ceil(a / step), n = Math.floor(b / step) - first;
    if (!isFinite(n) || n < 0 || n > 200) return [];
    return Array.from({ length: n + 1 }, (_, i) => { const x = (first + i) * step; return Math.abs(x) < step * 1e-9 ? 0 : x; });
  };
  const fmt = (t: number, unit: string) => (unit ? `${+t.toPrecision(6)}${unit}` : si(t, "", 3));
  const [xu, yu] = nyq ? ["", ""] : ["°", "dB"];
  const [cx, cy] = px(crit.x, crit.y), [ox, oy] = px(0, 0);
  const h = hit && curves[hit.c];
  const hx = h ? h.x[hit!.i] : null, hy = h ? h.y[hit!.i] : null;

  return (
    <div ref={host} className="relative h-full min-w-0 flex-1 overflow-hidden" onMouseMove={onMove} onMouseLeave={() => (setHit(null), onHover(null))} onWheel={onWheel} onDoubleClick={() => setView(null)}>
      {size.w > 0 && (
        <svg width={size.w} height={size.h} className="block text-text-secondary">
          {ticks(lo.x, hi.x).map((t) => { const [x] = px(t, 0); return <g key={`x${t}`}><line x1={x} x2={x} y1={0} y2={size.h} stroke="var(--border)" /><text x={x + 3} y={size.h - 6} fontSize={10} fill="currentColor" className="font-mono">{fmt(t, xu)}</text></g>; })}
          {ticks(lo.y, hi.y).map((t) => { const [, y] = px(0, t); return <g key={`y${t}`}><line x1={0} x2={size.w} y1={y} y2={y} stroke="var(--border)" /><text x={4} y={y - 3} fontSize={10} fill="currentColor" className="font-mono">{nyq && t ? `j${si(t, "", 3)}` : fmt(t, yu)}</text></g>; })}
          {nyq && <>
            <line x1={0} x2={size.w} y1={oy} y2={oy} stroke="var(--text-tertiary)" />
            <line x1={ox} x2={ox} y1={0} y2={size.h} stroke="var(--text-tertiary)" />
            {/* (the unit circle: |H| = 1, where the phase margin is read) */}
            <circle cx={ox} cy={oy} r={v.kx} fill="none" stroke="var(--text-tertiary)" strokeDasharray="4 3" />
          </>}
          {!nyq && <line x1={0} x2={size.w} y1={cy} y2={cy} stroke="var(--text-tertiary)" />}
          <path d={`M${cx - 7},${cy}L${cx + 7},${cy}M${cx},${cy - 7}L${cx},${cy + 7}`} stroke="var(--status-danger)" strokeWidth={2} />
          <text x={cx + 10} y={cy + 16} fontSize={10} fill="var(--status-danger)">{nyq ? "−1" : "−180°, 0 dB"}</text>
          <text x={size.w - 8} y={(nyq ? oy : size.h) - 6} fontSize={11} textAnchor="end" fill="currentColor">{nyq ? "Re" : "phase"}</text>
          <text x={(nyq ? ox : 0) + 6} y={14} fontSize={11} fill="currentColor">{nyq ? "Im" : "gain"}</text>
          {drawn.map((d, i) => <path key={i} d={d} fill="none" stroke={`var(--series-${curves[i].slot + 1})`} strokeWidth={1.8} strokeLinejoin="round" />)}
          {/* (where each curve starts: its lowest frequency) */}
          {curves.map((c, i) => {
            const k = c.x.findIndex((x, j) => x != null && c.y[j] != null);
            if (k < 0) return null;
            const [X, Y] = px(c.x[k]!, c.y[k]!);
            return <circle key={`s${i}`} cx={X} cy={Y} r={3} fill={`var(--series-${c.slot + 1})`} />;
          })}
          {h && hx != null && hy != null && (() => { const [X, Y] = px(hx, hy); return <circle cx={X} cy={Y} r={4} fill="none" stroke="var(--text-strong)" strokeWidth={1.5} />; })()}
        </svg>
      )}
      {h && hx != null && hy != null && (
        <div className="pointer-events-none absolute rounded-control border border-border bg-surface-panel px-2 py-1 font-mono text-2xs text-text shadow-float" style={{ left: Math.min(px(hx, hy)[0] + 12, size.w - 200), top: Math.max(px(hx, hy)[1] - 56, 4) }}>
          <div className="text-text-tertiary">{h.label}</div>
          <div>f = {si(freqs[hit!.i], "Hz", 4)}</div>
          <div>{nyq ? `${si(hx, "", 4)} ${hy < 0 ? "−" : "+"} j${si(Math.abs(hy), "", 4)}` : `${hy.toFixed(2)} dB ∠ ${hx.toFixed(1)}°`}</div>
        </div>
      )}
      <div className="pointer-events-none absolute bottom-6 right-2 text-2xs text-text-tertiary">● lowest frequency · wheel: zoom · dbl-click: fit</div>
    </div>
  );
});
