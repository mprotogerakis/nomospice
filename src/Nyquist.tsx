// Nyquist and Nichols plots of .ac results: each curve runs through the complex plane (Nyquist:
// Im over Re) or the gain-phase plane (Nichols: dB over degrees) with the frequency as its
// parameter; the critical point -1 (0 dB at -180° + k·360°) marked, which a loop gain must not
// encircle. Nyquist: the negative frequencies too (the mirror image, dashed), so the whole contour
// can be read.
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { si } from "./spice";
import { useS } from "./store";
import { tickStep } from "./PoleZero";
import { useSize } from "./lib/useSize";

/** A curve: its points (x, y) at the frequencies `freqs` (null: a gap). */
export type Curve = { label: string; slot: number; x: (number | null)[]; y: (number | null)[] };
/** A point under the pointer: curve c, frequency index i; m: on the mirror (the negative frequency). */
type Hit = { c: number; i: number; m?: boolean };

/** `others`: traces not drawn here (real ones: db(), a group delay, a level), named in a note. */
export const Polar = memo(function Polar({ curves, freqs, mode, onHover, others }: { curves: Curve[]; freqs: number[]; mode: "nyquist" | "nichols"; onHover: (f: number | null) => void; others: string[] }) {
  const host = useRef<HTMLDivElement>(null);
  const size = useSize(host);
  const nyq = mode === "nyquist";
  // the bounds of every curve (Nyquist: and its mirror, the origin, -1; Nichols: the critical
  // point nearest the curves' middle, -180° + k·360°). Nyquist: only where |H| ≤ 10, around the
  // unit circle (a loop gain with an integrator is 10^5 at low frequencies: -1 would be a dot);
  // zooming out shows the rest
  const fit = useMemo(() => {
    // (one pass: the bounds of all points and of the near ones; the near ones if there are any)
    const all = [Infinity, -Infinity, Infinity, -Infinity], close = [...all];
    const grow = (b: number[], x: number, y: number) => ((b[0] = Math.min(b[0], x)), (b[1] = Math.max(b[1], x)), (b[2] = Math.min(b[2], y, nyq ? -y : y)), (b[3] = Math.max(b[3], y, nyq ? -y : y)));
    for (const c of curves) c.x.forEach((x, i) => {
      const y = c.y[i];
      if (x == null || y == null || !Number.isFinite(x) || !Number.isFinite(y)) return;
      grow(all, x, y);
      if (nyq && x * x + y * y <= 100) grow(close, x, y);
    });
    let [x0, x1, y0, y1] = close[0] <= close[1] ? close : all;
    const crit = nyq ? -1 : x0 > x1 ? -180 : -180 + 360 * Math.round(((x0 + x1) / 2 + 180) / 360);
    [x0, x1, y0, y1] = [Math.min(x0, crit, nyq ? 0 : crit), Math.max(x1, crit, nyq ? 0 : crit), Math.min(y0, 0), Math.max(y1, 0)];
    const [sx, sy] = [x1 - x0 || 1, y1 - y0 || 1];
    return { x0: x0 - 0.08 * sx, x1: x1 + 0.08 * sx, y0: y0 - 0.08 * sy, y1: y1 + 0.08 * sy };
  }, [curves, nyq]);
  // view: Nyquist at one scale on both axes (angles and circles true), Nichols each its own;
  // wheel zooms around the cursor, double-click fits again (and other curves: they may lie elsewhere)
  const [view, setView] = useState<{ cx: number; cy: number; kx: number; ky: number } | null>(null);
  const [hit, setHit] = useState<Hit | null>(null);
  // (another mode or other curves: fitted afresh; a re-run of the same ones keeps the zoom)
  const which = useMemo(() => curves.map((c) => c.label).join("\n"), [curves]); // (another mode: a new Polar, keyed by it)
  useEffect(() => setView(null), [which]);
  useEffect(() => () => onHover(null), [onHover]); // (gone: the table's hover value with it)
  // (a re-run: its points are others)
  useEffect(() => {
    setHit(null);
    onHover(null);
  }, [curves]); // eslint-disable-line react-hooks/exhaustive-deps
  const margin = Math.min(32, size.w / 4, size.h / 4); // (less in a tiny pane: never a negative scale)
  const base = useMemo(() => {
    const kx = Math.max((size.w - 2 * margin) / (fit.x1 - fit.x0), 1e-30), ky = Math.max((size.h - 2 * margin) / (fit.y1 - fit.y0), 1e-30);
    const k = Math.min(kx, ky);
    return { cx: (fit.x0 + fit.x1) / 2, cy: (fit.y0 + fit.y1) / 2, kx: nyq ? k : kx, ky: nyq ? k : ky };
  }, [fit, size.w, size.h, nyq]);
  const v = view ?? base;
  const px = (x: number, y: number) => [size.w / 2 + (x - v.cx) * v.kx, size.h / 2 - (y - v.cy) * v.ky];
  const val = (X: number, Y: number) => ({ x: v.cx + (X - size.w / 2) / v.kx, y: v.cy - (Y - size.h / 2) / v.ky });

  const path = (c: Curve, sign: number) => {
    let d = "", pen = false;
    c.x.forEach((x, i) => {
      const y = c.y[i];
      if (x == null || y == null || !Number.isFinite(x) || !Number.isFinite(y)) return void (pen = false);
      const [X, Y] = px(x, sign * y);
      d += `${pen ? "L" : "M"}${X.toFixed(1)},${Y.toFixed(1)}`;
      pen = true;
    });
    return d;
  };
  const drawn = useMemo(() => curves.map((c) => ({ d: path(c, 1), mirror: nyq ? path(c, -1) : "" })), [curves, nyq, v.cx, v.cy, v.kx, v.ky, size.w, size.h]); // eslint-disable-line react-hooks/exhaustive-deps

  // hover: the nearest point within 14 px, its frequency to the trace table
  // drag: pan (both ways). ponytail: each move rebuilds the paths and the hover scans every point;
  // fine for 10^5 points, a transform and a grid if .step families of 10^6 lag

  const drag = useRef<{ X: number; Y: number; cx: number; cy: number } | null>(null);
  const onMove = (e: React.MouseEvent) => {
    const r = host.current!.getBoundingClientRect(), X = e.clientX - r.left, Y = e.clientY - r.top;
    if (drag.current && e.buttons & 1) {
      const d = drag.current;
      return setView({ ...v, cx: d.cx - (X - d.X) / v.kx, cy: d.cy + (Y - d.Y) / v.ky });
    }
    drag.current = null; // (released elsewhere)
    // (inline, no arrays: a .step family may hold a million points)
    let best: Hit | null = null, bd = 14 * 14;
    const [w2, h2] = [size.w / 2, size.h / 2];
    curves.forEach((c, ci) => {
      for (let i = 0; i < c.x.length; i++) {
        const x = c.x[i], y = c.y[i];
        if (x == null || y == null) continue;
        const dx = w2 + (x - v.cx) * v.kx - X, dy = h2 - (y - v.cy) * v.ky - Y, d = dx * dx + dy * dy;
        if (d < bd) (best = { c: ci, i }), (bd = d);
        if (nyq) { // (the mirror: -f, the conjugate)
          const dm = h2 - (-y - v.cy) * v.ky - Y, e2 = dx * dx + dm * dm;
          if (e2 < bd) (best = { c: ci, i, m: true }), (bd = e2);
        }
      }
    });
    const h = best as Hit | null;
    if (h?.c !== hit?.c || h?.i !== hit?.i || h?.m !== hit?.m) {
      setHit(h);
      onHover(h && !h.m ? freqs[h.i] ?? null : null); // (the mirror's -f: the table has no such column)
    }
  };
  // wheel: zoom around the cursor (a mouse's lines as pixels), a sideways swipe pans; not passive,
  // so the page does not scroll along (registered once: the current view from a ref)
  const now = useRef({ v, base, val });
  now.current = { v, base, val };
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const wheel = (e: WheelEvent) => {
      const { v, base, val } = now.current;
      e.preventDefault();
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1, dx = e.deltaX * unit, dy = e.deltaY * unit; // (lines, pages as pixels)
      if (Math.abs(dx) > Math.abs(dy)) return setView({ ...v, cx: v.cx + dx / v.kx });
      const r = el.getBoundingClientRect(), at = val(e.clientX - r.left, e.clientY - r.top);
      const f = Math.exp(-dy * 0.002);
      if ((f > 1 && v.kx > base.kx * 1e6) || (f < 1 && v.kx < base.kx * 1e-3)) return;
      setView({ kx: v.kx * f, ky: v.ky * f, cx: at.x - (at.x - v.cx) / f, cy: at.y - (at.y - v.cy) / f });
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  }, []);

  // grid
  const lo = val(0, size.h), hi = val(size.w, 0);
  // (Nyquist: one step on both axes, square cells at its one scale)
  const square = tickStep(Math.max(hi.x - lo.x, hi.y - lo.y));
  const ticks = (a: number, b: number, phase = false) => {
    // (degrees: 15/30/45/90 … so -180° + k·360° is a grid line; under 8° the usual steps)
    const deg = b - a >= 8 ? [1, 2, 5, 10, 15, 30, 45, 90, 180, 360].find((d) => (b - a) / d <= 8) ?? 360 : tickStep(b - a);
    const step = nyq ? square : phase ? deg : tickStep(b - a), first = Math.ceil(a / step), n = Math.floor(b / step) - first;
    if (!isFinite(n) || n < 0 || n > 200) return [];
    return Array.from({ length: n + 1 }, (_, i) => { const x = (first + i) * step; return Math.abs(x) < step * 1e-9 ? 0 : x; });
  };
  const fmt = (t: number, unit: string) => (unit ? `${+t.toPrecision(6)}${unit}` : si(t, "", 3));
  const [xu, yu] = nyq ? ["", ""] : ["°", "dB"];
  const [ox, oy] = px(0, 0);
  // the critical points in view: -1; 0 dB at -180° + k·360°
  const k0 = Math.ceil((lo.x + 180) / 360), nk = Math.floor((hi.x + 180) / 360) - k0 + 1;
  const crits = nyq ? [-1] : Number.isFinite(nk) && nk > 0 && nk <= 50 ? Array.from({ length: nk }, (_, k) => -180 + 360 * (k0 + k)) : [];
  // the cursors A and B (set in the Bode plot): where each curve is at their frequencies
  const fa = useS((s) => s.plot.a), fb = useS((s) => s.plot.b);
  // (outside the sweep: none, as the table shows none)
  const marks = useMemo(() => {
    // (nearest on a log scale when the sweep has one: as the Bode plot shows it)
    const gap = freqs[0] > 0 ? (x: number, f: number) => Math.abs(Math.log(x / f)) : (x: number, f: number) => Math.abs(x - f);
    const nearest = (f: number | null) => (f == null || !freqs.length || f < freqs[0] || f > freqs[freqs.length - 1] ? -1 : freqs.reduce((best, x, i) => (gap(x, f) < gap(freqs[best], f) ? i : best), 0));
    return ([["A", nearest(fa)], ["B", nearest(fb)]] as const).filter(([, i]) => i >= 0);
  }, [fa, fb, freqs]);
  const h = hit && curves[hit.c];
  const hx = h ? h.x[hit!.i] : null, hy = h ? ((y) => (y != null && hit!.m ? -y : y))(h.y[hit!.i]) : null;

  return (
    <div ref={host} className="relative h-full min-w-0 flex-1 cursor-grab select-none overflow-hidden active:cursor-grabbing" onMouseMove={onMove} onMouseLeave={() => ((drag.current = null), setHit(null), onHover(null))} onDoubleClick={() => setView(null)}
      onMouseDown={(e) => { if (e.button !== 0) return; const r = host.current!.getBoundingClientRect(); drag.current = { X: e.clientX - r.left, Y: e.clientY - r.top, cx: v.cx, cy: v.cy }; }} onMouseUp={() => (drag.current = null)}>
      {size.w > 0 && (
        <svg width={size.w} height={size.h} className="block text-text-secondary">
          {ticks(lo.x, hi.x, true).map((t) => { const [x] = px(t, 0); return <g key={`x${t}`}><line x1={x} x2={x} y1={0} y2={size.h} stroke="var(--border)" /><text x={x + 3} y={size.h - 6} fontSize={10} fill="currentColor" className="font-mono">{fmt(t, xu)}</text></g>; })}
          {ticks(lo.y, hi.y).map((t) => { const [, y] = px(0, t); return <g key={`y${t}`}><line x1={0} x2={size.w} y1={y} y2={y} stroke="var(--border)" /><text x={4} y={y - 3} fontSize={10} fill="currentColor" className="font-mono">{nyq && t ? `${t < 0 ? "−" : ""}j${si(Math.abs(t), "", 3)}` : fmt(t, yu)}</text></g>; })}
          {nyq && <>
            <line x1={0} x2={size.w} y1={oy} y2={oy} stroke="var(--text-tertiary)" />
            <line x1={ox} x2={ox} y1={0} y2={size.h} stroke="var(--text-tertiary)" />
            {/* (the unit circle: |H| = 1, where the phase margin is read) */}
            <circle cx={ox} cy={oy} r={v.kx} fill="none" stroke="var(--text-tertiary)" strokeDasharray="4 3" />
          </>}
          {!nyq && <line x1={0} x2={size.w} y1={oy} y2={oy} stroke="var(--text-tertiary)" />}
          {crits.map((c) => {
            const [cx, cy] = px(c, 0);
            return (
              <g key={c}>
                <path d={`M${cx - 7},${cy}L${cx + 7},${cy}M${cx},${cy - 7}L${cx},${cy + 7}`} stroke="var(--status-danger)" strokeWidth={2} />
                <text x={cx + 10} y={cy + 16} fontSize={10} fill="var(--status-danger)">{nyq ? "−1" : `${c}°, 0 dB`}</text>
              </g>
            );
          })}
          {/* (at the axes, kept inside when the origin is panned away) */}
          <text x={size.w - 8} y={nyq ? Math.min(Math.max(oy - 6, 14), size.h - 22) : size.h - 22} fontSize={11} textAnchor="end" fill="currentColor">{nyq ? "Re" : "phase"}</text>
          <text x={nyq ? Math.min(Math.max(ox + 6, 40), size.w - 30) : 6} y={14} fontSize={11} fill="currentColor">{nyq ? "Im" : "gain"}</text>
          {drawn.map(({ d, mirror }, i) => (
            <g key={i} fill="none" stroke={`var(--series-${curves[i].slot + 1})`} strokeLinejoin="round">
              {mirror && <path d={mirror} strokeWidth={1} strokeDasharray="4 3" opacity={0.6} />}
              <path d={d} strokeWidth={1.8} />
            </g>
          ))}
          {/* (where each curve starts: its lowest frequency) */}
          {curves.map((c, i) => {
            const k = c.x.findIndex((x, j) => x != null && c.y[j] != null);
            if (k < 0) return null;
            const [X, Y] = px(c.x[k]!, c.y[k]!);
            return <circle key={`s${i}`} cx={X} cy={Y} r={3} fill={`var(--series-${c.slot + 1})`} />;
          })}
          {marks.flatMap(([name, i]) => curves.map((c, j) => {
            const [x, y] = [c.x[i], c.y[i]];
            if (x == null || y == null) return null;
            const [X, Y] = px(x, y);
            return <g key={`${name}${j}`} fill="var(--selection)"><circle cx={X} cy={Y} r={3} /><text x={X + 5} y={Y - 5} fontSize={10}>{name}</text></g>;
          }))}
          {h && hx != null && hy != null && (() => { const [X, Y] = px(hx, hy); return <circle cx={X} cy={Y} r={4} fill="none" stroke="var(--text-strong)" strokeWidth={1.5} />; })()}
        </svg>
      )}
      {h && hx != null && hy != null && (
        <div className="pointer-events-none absolute rounded-control border border-border bg-surface-panel px-2 py-1 font-mono text-2xs text-text shadow-float" style={{ left: Math.max(4, Math.min(px(hx, hy)[0] + 12, size.w - 200)), top: Math.max(px(hx, hy)[1] - 56, 4) }}>
          <div className="text-text-tertiary">{h.label}</div>
          <div>f = {hit!.m ? "−" : ""}{si(freqs[hit!.i], "Hz", 4)}</div>
          <div>{nyq ? `${si(hx, "", 4)} ${hy < 0 ? "−" : "+"} j${si(Math.abs(hy), "", 4)}` : `${hy.toFixed(2)} dB ∠ ${hx.toFixed(1)}°`}</div>
        </div>
      )}
      {!!others.length && <div className="pointer-events-none absolute left-2 top-6 max-w-[60%] truncate text-2xs text-text-tertiary">Only in the Bode plot: {others.join(", ")}</div>}
      <div className="pointer-events-none absolute bottom-6 right-2 text-2xs text-text-tertiary">● lowest frequency{nyq ? " · dashed: negative frequencies" : ""} · drag: pan · wheel: zoom · dbl-click: fit</div>
    </div>
  );
});
