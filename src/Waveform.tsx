import { memo, useEffect, useMemo, useRef } from "react";
import { useSize } from "./lib/useSize";
import uPlot from "uplot";
import { create } from "zustand";
import { Plus, X } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { removeTrace, setPlot, setTracePane, setTraceRight, setYAxis, showFft, useS } from "./store";
import { openMenu } from "./Menu";
import { at, fftSize, integrate, si, spectrum, unitTimes, xUnitOf } from "./spice";
import { keys } from "./lib/utils";
import type { Trace } from "./types";
import { saveSvg, withLightTheme, xmlEscape } from "./svgExport";

/** A plotted curve: of trace `t`, curve `k` of its `n` in the pane (a .step or nested .dc family);
 *  its values in `u`, `r`: wanted on the right axis (moved there, or moved into a pane of
 *  another unit). */
type Series = { t: Trace; y: (number | null)[]; k: number; n: number; u: string; r: boolean };
/** A pane's scale for a series: "y2" (right axis) for one wanted there, while the pane also has
 *  left ones (all on the right: the left axis serves). */
const scaleOf = (pane: { series: Series[] }, s: Series) => (s.r && pane.series.some((o) => !o.r) ? "y2" : "y");
/** `unit2`: its right axis's unit (one for all its series there, else none). */
type Pane = { unit: string; unit2: string; series: Series[] };
/** Settings of a pane's y axis, by its unit (the spectrum's apart). */
const axisKey = (unit: string, fft: boolean) => `${fft ? "fft:" : ""}${unit}`;

/** px a press may move and still count as a click (shared by uPlot's drag and our click rule). */
const CLICK_DIST = 3;

/** x under the plot crosshair (separate store: only the readout table re-renders on mouse moves). */
const useHover = create(() => ({ x: null as number | null }));

const css = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const color = (slot: number) => css(`--series-${slot + 1}`);
/** dB; an exactly-zero magnitude (AC-quiet node) is a gap, not -6000 dB squashing the pane. */
const mag = (t: Trace) => t.data!.re.map((re, i) => { const m = Math.hypot(re, t.data!.im?.[i] ?? 0); return m ? 20 * Math.log10(m) : null; });
function phase(t: Trace, seg = Infinity) {
  let off = 0, prev = 0, fresh = true;
  return t.data!.re.map((re, i) => {
    if (i % seg === 0) (off = 0), (fresh = true); // (a family's next curve: unwrapped on its own)
    if (re == null) return null; // (a gap: a step halted before this x)
    const p = (Math.atan2(t.data!.im?.[i] ?? 0, re) * 180) / Math.PI;
    if (!fresh) off += p + off - prev > 180 ? -360 : p + off - prev < -180 ? 360 : 0; // unwrap
    fresh = false;
    prev = p + off;
    return prev;
  });
}
/** Nested .dc sweeps repeat the inner sweep: length of one segment (xs.length if not nested).
 *  An aborted/halted run's incomplete last family is left out (split() takes whole segments). */
function segment(xs: number[]) {
  const up = xs[1] > xs[0];
  const n = xs.findIndex((x, i) => i > 0 && (up ? x < xs[i - 1] : x > xs[i - 1]));
  return n > 1 ? n : xs.length;
}

/** Curve `k` of `n` of trace `t`: its .step (R=1k, as the trace says which steps it holds), else its
 *  number (a nested .dc), nothing for a single curve. */
const curveName = (t: Trace, k: number, n: number, steps: string[]) =>
  n < 2 ? "" : t.data?.steps?.length === n ? steps[t.data.steps[k]] ?? `#${k + 1}` : `#${k + 1}`;
const member = (s: Series, steps: string[]) => curveName(s.t, s.k, s.n, steps);

/** The plotted data as CSV: x, then one column per series (family curves named by step or numbered). */
function csv(xs: number[], xUnit: string, panes: Pane[], steps: string[]) {
  const cols = panes.flatMap((p) => p.series.map((s) => ({ name: `${s.t.label}${((m) => (m ? ` ${m}` : ""))(member(s, steps))} [${s.u}]`, y: s.y })));
  const head = [`x [${xUnit}]`, ...cols.map((c) => `"${c.name.replace(/"/g, '""')}"`)].join(",");
  return [head, ...xs.map((x, i) => [x, ...cols.map((c) => c.y[i] ?? "")].join(","))].join("\n") + "\n";
}

/** The panes as one SVG in the light theme: same axes, ticks, zoom and cursors as on screen, plus a
 *  legend (on screen the trace table does that job). uPlot draws on a canvas, so this redraws from
 *  its scales and tick lists. ponytail: every sample becomes a path point (fine up to ~10^5 points). */
function plotSvg(us: uPlot[], panes: Pane[], cur: { a: number | null; b: number | null }): string {
  return withLightTheme(() => {
    const dpr = devicePixelRatio, ink = css("--text-secondary"), grid = css("--border"), sel = css("--selection");
    const out: string[] = [];
    const txt = (x: number, y: number, s: string, anchor = "middle", base = "auto", fill = ink) =>
      out.push(`<text x="${x.toFixed(1)}" y="${y.toFixed(1)}" text-anchor="${anchor}" dominant-baseline="${base}" fill="${fill}">${xmlEscape(s)}</text>`);
    const line = (x1: number, y1: number, x2: number, y2: number, stroke: string, extra = "") =>
      out.push(`<line x1="${x1.toFixed(1)}" y1="${y1.toFixed(1)}" x2="${x2.toFixed(1)}" y2="${y2.toFixed(1)}" stroke="${stroke}" ${extra}/>`);
    // legend: every trace once (a complex trace has a dB and a phase series)
    let lx = us[0].bbox.left / dpr;
    for (const t of new Map(panes.flatMap((p) => p.series.map((s) => [s.t.expr, s.t] as const))).values()) {
      line(lx, 12, lx + 18, 12, color(t.slot), `stroke-width="2"`);
      txt(lx + 24, 12, t.label, "start", "middle");
      lx += 48 + 7 * t.label.length;
    }
    let y = 28;
    us.forEach((u, k) => {
      const L = u.bbox.left / dpr, T = y + u.bbox.top / dpr, PW = u.bbox.width / dpr, PH = u.bbox.height / dpr;
      const ax = u.axes as unknown as { _splits?: (number | null)[]; _values?: (string | number)[] }[];
      const px = (v: number) => L + u.valToPos(v, "x"), py = (v: number, sc = "y") => T + u.valToPos(v, sc);
      out.push(`<clipPath id="pane${k}"><rect x="${L}" y="${T}" width="${PW}" height="${PH}"/></clipPath>`);
      (ax[1]._splits ?? []).forEach((v, i) => {
        if (v == null) return;
        line(L, py(v), L + PW, py(v), grid);
        txt(L - 6, py(v), String(ax[1]._values?.[i] ?? ""), "end", "middle");
      });
      // the right axis (a pane without one may have an empty spacer there)
      (ax[2]?._splits ?? []).forEach((v, i) => v != null && ax[2]._values?.[i] != null && txt(L + PW + 6, py(v, "y2"), String(ax[2]._values[i]), "start", "middle"));
      (ax[0]._splits ?? []).forEach((v, i) => {
        if (v == null) return;
        line(px(v), T, px(v), T + PH, grid);
        const label = ax[0]._values?.[i];
        if (k === us.length - 1 && label) txt(px(v), T + PH + 16, String(label));
      });
      out.push(`<rect x="${L}" y="${T}" width="${PW}" height="${PH}" fill="none" stroke="${grid}"/>`);
      const xs = u.data[0], { min = -Infinity, max = Infinity } = u.scales.x;
      panes[k].series.forEach((s, j) => {
        const ys = u.data[j + 1], sc = scaleOf(panes[k], s);
        let d = "", pen = false;
        for (let i = 0; i < xs.length; i++) {
          const v = ys[i];
          // outside the zoom: skip, but keep one point beyond each edge so the clipped line reaches it
          if ((xs[i + 1] ?? Infinity) < min || (xs[i - 1] ?? -Infinity) > max) continue;
          if (v == null || !Number.isFinite(v)) { pen = false; continue; }
          d += `${pen ? "L" : "M"}${px(xs[i]).toFixed(1)} ${py(v, sc).toFixed(1)}`;
          pen = true;
        }
        out.push(`<path d="${d}" fill="none" stroke="${color(s.t.slot)}" stroke-width="1.5" stroke-linejoin="round" clip-path="url(#pane${k})"/>`);
      });
      for (const [name, xv] of [["A", cur.a], ["B", cur.b]] as const) {
        if (xv == null || xv < min || xv > max) continue;
        line(px(xv), T, px(xv), T + PH, sel, `stroke-dasharray="4 3"`);
        if (k === 0) txt(px(xv) + 4, T + 11, name, "start", "auto", sel);
      }
      y += u.height;
    });
    const W = us[0].width, H = Math.ceil(y + 4);
    return `<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" font-family="IBM Plex Mono, Menlo, Consolas, monospace" font-size="11">${out.join("")}</svg>\n`;
  });
}

/** Linear interpolation of ys at x; NaN outside the data (a cursor from a longer previous run). */
export const Waveform = memo(function Waveform() {
  const run = useS((s) => s.sim?.run);
  const traces = useS((s) => s.traces);
  const ac = !!run?.plot.startsWith("ac"); // complex: magnitude + phase panes
  const fft = useS((s) => s.plot.fft);
  const yAxes = useS((s) => s.plot.y);
  const tran = run?.scale === "time";
  const spec = !!fft && tran; // the spectrum view
  const freq = run?.scale === "frequency" || spec;
  const xUnit = spec ? "Hz" : xUnitOf(run?.scale ?? "");
  const ok = useMemo(() => traces.filter((t) => t.data?.x.length), [traces]);
  // the spectrum view: each trace (each .step curve) as its spectrum over fft.lo..hi, in dB
  const points = useMemo(() => (spec && ok[0] ? fftSize(ok[0].data!.x.slice(0, segment(ok[0].data!.x)), fft!.lo, fft!.hi) : 0), [spec, ok, fft]);
  const shown = useMemo(() => {
    if (!spec || !points) return ok;
    return ok.filter((t) => !t.data!.constant && !t.data!.im).map((t): Trace => {
      const { x, re } = t.data!, sg = segment(x);
      const parts = Array.from({ length: Math.max(1, Math.floor(x.length / sg)) }, (_, k) => spectrum(x.slice(k * sg, (k + 1) * sg), re.slice(k * sg, (k + 1) * sg), fft!.lo, fft!.hi, points));
      return { ...t, unit: "dB", right: undefined, pane: undefined, data: { ...t.data!, x: parts.flatMap((p) => p.f), re: parts.flatMap((p) => p.db) as number[], constant: undefined } };
    });
  }, [ok, spec, fft, points]);
  // constants (reference lines: 0.5, 500m) are drawn on the top pane's axis, whatever it is now:
  // volts first, the magnitude in a Bode plot
  const [lines, signals] = useMemo(() => [shown.filter((t) => t.data!.constant), shown.filter((t) => !t.data!.constant)], [shown]);
  // .ac without an AC source: every value is exactly 0 (no dB at all, phase flat) - say so instead
  const zero = useMemo(() => ac && signals.length > 0 && signals.every((t) => t.data!.re.every((v, i) => !v && !t.data!.im?.[i])), [ac, signals]);
  const all = shown[0]?.data!.x ?? [];
  // nested .dc (family curves): one series per outer step over the first segment's x
  const seg = useMemo(() => segment(all), [all]);
  const xs = useMemo(() => all.slice(0, seg), [all, seg]);
  // frequency sweeps on a log axis, unless it is a linear sweep (evenly spaced, may start at 0 Hz)
  const linear = xs.length > 2 && Math.abs(xs[2] - 2 * xs[1] + xs[0]) <= 1e-9 * Math.abs(xs[1] - xs[0]);
  const logX = freq && xs[0] > 0 && (!linear || spec);
  const panes = useMemo<Pane[]>(() => {
    if (!shown.length || zero) return [];
    const split = (t: Trace, y: (number | null)[], u: string, r: boolean): Series[] => {
      const n = Math.max(1, Math.floor(y.length / seg));
      return Array.from({ length: n }, (_, k) => ({ t, y: y.slice(k * seg, (k + 1) * seg), k, n, u, r }));
    };
    // complex vectors (.ac, AC .sens) -> magnitude (dB) + phase panes; real ones (db(), ph() ...) as they are
    const cplx = signals.filter((t) => t.data!.im);
    const real = signals.filter((t) => !t.data!.im);
    const panes: Pane[] = [];
    if (cplx.length) panes.push({ unit: "dB", unit2: "", series: cplx.flatMap((t) => split(t, mag(t), "dB", !!t.right)) }, { unit: "°", unit2: "", series: cplx.flatMap((t) => split(t, phase(t, seg), "°", !!t.right)) });
    // a trace moved to another unit's pane goes there while that pane has traces of its own
    const own = new Set(real.filter((t) => !t.pane).map((t) => t.unit));
    const home = (t: Trace) => (t.pane != null && own.has(t.pane) ? t.pane : t.unit);
    const units = [...new Set(real.map(home))].sort((a, b) => (a === "V" ? -1 : b === "V" ? 1 : a.localeCompare(b)));
    for (const unit of units) {
      const series = real.filter((t) => home(t) === unit).flatMap((t) => split(t, t.data!.re, t.unit, !!t.right || t.unit !== unit));
      const same = panes.find((p) => p.unit === unit); // db(...) joins the dB pane
      if (same) same.series.push(...series);
      else panes.push({ unit, unit2: "", series });
    }
    if (lines.length && !panes.length) panes.push({ unit: "", unit2: "", series: [] });
    // one series per level (not one per outer .dc step); in a Bode plot a magnitude, as in LTspice:
    // 1 at 0 dB, 0.707 at -3 dB
    panes[0]?.series.push(...lines.map((t) => ({ t, y: (panes[0].unit === "dB" ? mag(t) : t.data!.re).slice(0, seg), k: 0, n: 1, u: panes[0].unit, r: false })));
    for (const p of panes) {
      const right = new Set(p.series.filter((s) => scaleOf(p, s) === "y2").map((s) => s.u));
      p.unit2 = right.size === 1 ? [...right][0] : "";
    }
    return panes;
  }, [shown, lines, signals, zero, seg]);

  const host = useRef<HTMLDivElement>(null);
  const paneEls = useRef<(HTMLDivElement | null)[]>([]);
  // zoom + cursors live in the store (kept across re-runs, reset per document / analysis kind)
  const plots = useRef<uPlot[]>([]);

  const size = useSize(host);

  const sizeRef = useRef(size);
  sizeRef.current = size;
  useEffect(() => {
    const size = sizeRef.current;
    if (!size.w || !panes.length) return;
    const text = css("--text-secondary"), grid = css("--border"), sel = css("--selection");
    let syncing = false;
    const all: uPlot[] = [];
    // cursor state before the first click of a (possible) double-click; shared by all panes
    let before = { a: useS.getState().plot.a, b: useS.getState().plot.b };
    const h = Math.floor(size.h / panes.length);
    // one pane with a right axis: the others keep the same width free, so their x axes line up
    const anyTwin = panes.some((p) => p.series.some((s) => scaleOf(p, s) === "y2"));
    let dragFrom: uPlot | null = null; // the pane a drag started in (the others follow it, synced)
    panes.forEach((pane, pi) => {
      const el = paneEls.current[pi];
      if (!el) return;
      const last = pi === panes.length - 1;
      const span = pane.unit === "dB" ? 20 : pane.unit === "°" ? 90 : 0;
      const twin = pane.series.some((s) => scaleOf(pane, s) === "y2");
      // dB/phase: at least `span` tall, so 0.1 dB of ripple does not fill the pane like a resonance;
      // a spectrum: at most 140 dB below its peak (rounding's floor at -300 dB is no signal)
      const yRange = span ? { range: (_u: uPlot, lo: number | null, hi: number | null): uPlot.Range.MinMax => {
        if (lo == null || hi == null) return [lo, hi];
        if (spec && hi - lo > 140) lo = hi - 140;
        if (hi - lo < span) [lo, hi] = [(lo + hi - span) / 2, (lo + hi + span) / 2];
        return uPlot.rangeNum(lo, hi, 0.05, true);
      } } : {};
      // the left axis as set: log, a fixed range
      const ax = yAxes[axisKey(pane.unit, spec)] ?? {};
      const logY = !!ax.log && !span;
      const fixed = ax.lo != null && ax.hi != null ? { range: (): uPlot.Range.MinMax => [ax.lo!, ax.hi!] } : {};
      const values = (unit: string) => (_u: uPlot, v: number[], _i: number, _s: number, incr: number) =>
        v.map((y) => (unit === "dB" || unit === "°" ? `${+y.toFixed(Math.max(0, Math.ceil(-Math.log10(incr))))}${unit}` : si(y, unit, 3)));
      const opts: uPlot.Options = {
        width: size.w, height: h,
        legend: { show: false },
        // dist: a click with 1-2 px of jitter stays a click (places a cursor) instead of a micro-zoom
        // click: no-op so uPlot does not swallow slightly-moved clicks; our handler decides (3 px rule)
        cursor: { sync: { key: "nomospice" }, drag: { x: true, y: false, setScale: true, dist: CLICK_DIST, click: () => {} }, points: { size: 6 } },
        scales: {
          // exactly the data: a log axis would otherwise round up to whole decades (1 MHz data -> 10 MHz)
          x: { time: false, distr: logX ? 3 : 1, ...(logX ? { range: (_u, lo, hi): uPlot.Range.MinMax => (lo < hi ? [lo, hi] : [lo / 2, hi * 2]) } : {}) },
          y: { ...yRange, ...(logY ? { distr: 3, log: 10 } : {}), ...fixed },
          ...(twin ? { y2: yRange } : {}),
        },
        axes: [
          { stroke: text, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid, width: 1 }, size: last ? 36 : 8, values: last ? (_u, v) => v.map((x) => (x == null ? "" : si(x, xUnit, 3))) : () => [], font: "11px IBM Plex Mono" },
          { stroke: text, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid, width: 1 }, size: 72, values: values(pane.unit), font: "11px IBM Plex Mono", label: pane.unit === "" ? "expr" : undefined },
          // the right axis: its traces' own scale, in the color of its trace when it has one
          ...(!twin && anyTwin ? [{ side: 1, size: 72, values: () => [], grid: { show: false }, ticks: { show: false }, stroke: "transparent" }] : []),
          ...(twin ? [{ scale: "y2", side: 1, stroke: (() => { const r = pane.series.filter((s) => scaleOf(pane, s) === "y2"); return new Set(r.map((s) => s.t)).size === 1 ? color(r[0].t.slot) : text; })() /* (a .dc family: one trace, several series) */, grid: { show: false }, ticks: { stroke: grid, width: 1 }, size: 72, values: values(pane.unit2), font: "11px IBM Plex Mono" }] : []),
        ],
        series: [{}, ...pane.series.map((s) => ({ label: s.t.label, scale: scaleOf(pane, s), stroke: color(s.t.slot), width: 2, points: { show: false } }))],
        hooks: {
          ready: [(u) => {
            // click = cursor; a drag (zoom) also ends in a click, so compare with the press position
            let down = 0;
            u.over.addEventListener("mousedown", (e) => {
              down = e.clientX;
              // shift-drag marks the range A–B (its statistics) instead of zooming; in every pane (synced)
              all.forEach((o) => (o.cursor.drag!.setScale = !e.shiftKey));
              dragFrom = u;
              if (e.detail === 1) before = { a: useS.getState().plot.a, b: useS.getState().plot.b };
            });
            u.over.addEventListener("click", (e) => {
              if (Math.abs(e.clientX - down) >= CLICK_DIST || e.detail > 1 || u.cursor.left == null) return;
              const xv = u.posToVal(u.cursor.left, "x");
              setPlot(e.shiftKey ? { b: xv } : { a: xv });
            });
            // double-click resets the zoom (uPlot) and undoes what its first click did to the cursors
            u.over.addEventListener("dblclick", () => setPlot({ zoom: null, ...before }));
            u.over.addEventListener("wheel", (e) => {
              e.preventDefault();
              const { min, max } = u.scales.x;
              if (min == null || max == null || u.cursor.left == null) return;
              const lg = logX ? Math.log10 : (v: number) => v, ex = logX ? (v: number) => 10 ** v : lg;
              const dx = e.deltaX, dy = e.deltaY * (e.deltaMode === 1 ? 16 : 1);
              let lo = lg(min), hi = lg(max);
              if (Math.abs(dx) > Math.abs(dy)) {
                const shift = (dx / (u.bbox.width / devicePixelRatio)) * (hi - lo); // horizontal swipe pans
                (lo += shift), (hi += shift);
              } else {
                const c = lg(u.posToVal(u.cursor.left, "x")), f = Math.exp(dy * 0.002); // proportional: trackpads send many small deltas
                (lo = c - (c - lo) * f), (hi = c + (hi - c) * f);
              }
              setPlot({ zoom: [ex(lo), ex(hi)] });
              u.setScale("x", { min: ex(lo), max: ex(hi) });
            }, { passive: false });
          }],
          setScale: [(u, key) => {
            if (key !== "x" || syncing) return;
            syncing = true;
            all.forEach((o) => o !== u && o.setScale("x", { min: u.scales.x.min!, max: u.scales.x.max! }));
            syncing = false;
          }],
          setSelect: [(u) => {
            if (u.select.width <= 0) return;
            const range: [number, number] = [u.posToVal(u.select.left, "x"), u.posToVal(u.select.left + u.select.width, "x")];
            // drag-zoom (uPlot applies it after this hook): remember it as the user's zoom
            if (u.cursor.drag!.setScale) return setPlot({ zoom: range });
            // shift-drag: the cursors at its ends, the box gone (the shading between A and B stays);
            // once, from the pane it was made in
            if (u !== dragFrom) return;
            setPlot({ a: range[0], b: range[1] });
            all.forEach((o) => o.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false));
          }],
          setCursor: [(u) => useHover.setState({ x: u.cursor.idx == null ? null : (u.data[0][u.cursor.idx] ?? null) })],
          draw: [(u) => {
            const ctx = u.ctx;
            const { a, b } = useS.getState().plot;
            if (a != null && b != null) {
              // the range A–B, lightly shaded (what the statistics cover)
              const [x0, x1] = [a, b].map((v) => Math.min(Math.max(u.valToPos(v, "x", true), u.bbox.left), u.bbox.left + u.bbox.width)).sort((p, q) => p - q);
              ctx.save();
              ctx.globalAlpha = 0.08;
              ctx.fillStyle = sel;
              ctx.fillRect(x0, u.bbox.top, x1 - x0, u.bbox.height);
              ctx.restore();
            }
            for (const [name, xv] of [["A", a], ["B", b]] as const) {
              if (xv == null || xv < u.scales.x.min! || xv > u.scales.x.max!) continue; // off-screen: not over the axes
              const px = u.valToPos(xv, "x", true);
              ctx.save();
              ctx.strokeStyle = sel;
              ctx.fillStyle = sel;
              ctx.setLineDash([4, 3]);
              ctx.beginPath();
              ctx.moveTo(px, u.bbox.top);
              ctx.lineTo(px, u.bbox.top + u.bbox.height);
              ctx.stroke();
              ctx.font = `${11 * devicePixelRatio}px IBM Plex Mono`;
              ctx.fillText(name, px + 4 * devicePixelRatio, u.bbox.top + 12 * devicePixelRatio);
              ctx.restore();
            }
          }],
        },
      };
      // uPlot needs ascending x (descending .dc sweeps): reverse a copy; at() handles both orders
      const desc = xs.length > 1 && xs[0] > xs[xs.length - 1];
      const ord = <T,>(a: T[]) => (desc ? [...a].reverse() : a);
      // (a log axis: what is not above 0 is a gap)
      const ys = (s: Series) => (logY && scaleOf(pane, s) === "y" ? s.y.map((v) => (v != null && v > 0 ? v : null)) : s.y);
      all.push(new uPlot(opts, [ord(xs), ...pane.series.map((s) => ord(ys(s)))] as uPlot.AlignedData, el));
    });
    plots.current = all;
    const zoom = useS.getState().plot.zoom;
    // a zoom from a linear sweep may reach 0 Hz: unusable on a log axis
    if (zoom && !(logX && zoom[0] <= 0)) all[0]?.setScale("x", { min: zoom[0], max: zoom[1] });
    else if (zoom) setPlot({ zoom: null });
    return () => {
      all.forEach((u) => u.destroy());
      plots.current = [];
    };
    // size is read via sizeRef: resizing must not rebuild the plots
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [panes, ac, logX, xUnit, xs, size.w > 0, yAxes, spec]);

  useEffect(() => {
    const h = Math.floor(size.h / Math.max(panes.length, 1));
    plots.current.forEach((u) => u.setSize({ width: size.w, height: h }));
  }, [size, panes.length]);

  // cursor lines are drawn by the draw hook from the store: redraw when they move
  useEffect(() => useS.subscribe((s, p) => (s.plot.a !== p.plot.a || s.plot.b !== p.plot.b) && plots.current.forEach((u) => u.redraw(false))), []);

  return (
    <div className="flex h-full min-h-0 bg-surface-schematic-canvas">
      <div ref={host} className="relative min-w-0 flex-1 overflow-hidden" onContextMenu={(e) => {
        e.preventDefault();
        const { a, b, zoom } = useS.getState().plot;
        // the spectrum of A–B, else of what is shown, else of the whole run
        const range: [number, number] = a != null && b != null && a !== b ? [Math.min(a, b), Math.max(a, b)] : zoom ?? [Math.min(xs[0], xs[xs.length - 1]), Math.max(xs[0], xs[xs.length - 1])];
        const pane = panes[paneEls.current.findIndex((el) => el?.contains(e.target as Node))];
        const key = pane ? axisKey(pane.unit, spec) : "", ax = yAxes[key] ?? {};
        const db = pane?.unit === "dB" || pane?.unit === "°";
        openMenu(e.clientX, e.clientY, [
          { label: "Reset zoom", hint: "dbl-click", disabled: !panes.length, run: () => { setPlot({ zoom: null }); plots.current[0]?.setScale("x", { min: xs[0], max: xs[xs.length - 1] }); } },
          { label: "Clear cursors", run: () => setPlot({ a: null, b: null }) },
          { label: "Add trace…", disabled: !run, run: () => useS.setState({ dialog: { k: "trace" } }) },
          ...(tran ? [spec ? { label: "Back to the transient", run: () => showFft(null) }
            : { label: "FFT (spectrum)", hint: a != null && b != null && a !== b ? "of A–B" : zoom ? "of the shown range" : "of the whole run", disabled: !panes.length || !(range[1] > range[0]), run: () => showFft(range) }] : []),
          ...(pane ? ["-" as const,
            { label: ax.log ? "Linear y axis" : "Log y axis", hint: db ? `${pane.unit}: no` : undefined, disabled: db, run: () => setYAxis(key, { ...ax, log: !ax.log || undefined, ...(!ax.log && ax.lo != null && ax.lo <= 0 ? { lo: undefined, hi: undefined } : {}) }) },
            { label: "Y range…", hint: ax.lo != null && ax.hi != null ? `${si(ax.lo, pane.unit, 3)} … ${si(ax.hi, pane.unit, 3)}` : "automatic", run: () => useS.setState({ dialog: { k: "yrange", pane: key, unit: pane.unit, log: !!ax.log && !db } }) },
            ...(ax.lo != null ? [{ label: "Automatic y range", run: () => setYAxis(key, { ...ax, lo: undefined, hi: undefined }) }] : []),
          ] : []),
          "-",
          { label: "Copy data as CSV", disabled: !panes.length, run: () => navigator.clipboard.writeText(csv(xs, xUnit, panes, run?.steps ?? [])) },
          { label: "Export plot as SVG…", disabled: !panes.length, run: () => void saveSvg(plotSvg(plots.current, panes, useS.getState().plot), "-plot") },
          { label: "Remove all traces", danger: true, disabled: !traces.length, run: () => useS.setState({ traces: [] }) },
        ]);
      }}>
        {panes.map((p, i) => <div key={`${p.unit}-${i}`} ref={(el) => { paneEls.current[i] = el; }} />)}
        {!panes.length && (
          <div className="absolute inset-0 flex items-center justify-center text-xs text-text-tertiary">
            {!run ? `Run a simulation (${keys("⌘↵")}).` : zero ? "All AC results are 0: no source has an AC amplitude. Give the input source one, e.g. AC 1." : run.plot.startsWith("noise") ? "Add a trace: onoise_spectrum or inoise_spectrum." : run.plot.startsWith("sens") ? "Add a trace: the sensitivity of a part, e.g. r1." : "Click a wire to plot its voltage, a component to plot its current."}
          </div>
        )}
      </div>
      <div className="flex w-[340px] shrink-0 flex-col border-l border-border bg-surface-panel text-xs">
        {spec && (
          <div className="border-b border-border px-2 py-1 font-mono text-2xs text-text-secondary">
            Spectrum of {si(fft!.lo, "s")} … {si(fft!.hi, "s")}: {points} points, Hann window, dB of the amplitude (a sine of 1 V: 0 dB) ·{" "}
            <button className="underline hover:text-text-strong" onClick={() => showFft(null)}>back to the transient</button>
          </div>
        )}
        <Readout traces={spec ? shown : traces} panes={panes} xs={xs} xUnit={xUnit} plot={spec ? "FFT" : run?.plot ?? ""} steps={run?.steps ?? NO_STEPS} />
        <Measurements xUnit={xUnit} />
        <button
          className="m-2 inline-flex h-[22px] items-center justify-center gap-1 rounded-control border border-border-control text-text-secondary hover:bg-surface-hover hover:text-text-strong disabled:opacity-40"
          disabled={!run}
          onClick={() => useS.setState({ dialog: { k: "trace" } })}
        >
          <Plus className="h-3 w-3" /> Add trace
        </button>
        <p className="mt-auto px-2 pb-2 text-2xs leading-4 text-text-tertiary">
          Drag: zoom · shift-drag: range A–B · click: A · shift-click: B · dbl-click: reset · right-click a trace: right axis, another pane · right-click the plot: FFT, y axis
        </p>
      </div>
    </div>
  );
});

/** `.meas` results of the run (ngspice reports them in the log; LTspice in its error log). */
export function Measurements({ xUnit }: { xUnit: string }) {
  const meas = useS((s) => s.sim?.run?.meas);
  if (!meas?.length) return null;
  return (
    <div className="border-b border-border px-2 py-1 font-mono text-2xs">
      <div className="text-text-tertiary">Measurements (.meas)</div>
      {meas.map(([name, v, info, onX], k) => (
        <div key={`${name}${k}`} className="truncate" title={v == null ? `ngspice could not evaluate it: see the log (${keys("⌘L")})` : info}>
          <span className="text-text">{name}</span> = {v == null ? <span className="text-status-danger">failed</span> : si(v, onX ? xUnit : "", 5)}
        </div>
      ))}
    </div>
  );
}

/** Trace table: values under the hover crosshair and at cursors A/B. Own component + hover store,
 *  so mouse moves over the plot re-render only this table. */
const NO_STEPS: string[] = [];
const Readout = memo(function Readout({ traces, panes, xs, xUnit, plot, steps }: { traces: Trace[]; panes: Pane[]; xs: number[]; xUnit: string; plot: string; steps: string[] }) {
  const hover = useHover((h) => h.x);
  const { a, b } = useS(useShallow((s) => ({ a: s.plot.a, b: s.plot.b })));
  // a trace alone in its pane(s): a right axis would change nothing there
  const alone = (t: Trace) => !panes.some((p) => p.series.some((s) => s.t === t) && p.series.some((s) => s.t !== t));
  // in another unit's pane (there on the right axis anyway)
  const moved = (t: Trace) => panes.some((p) => p.series.some((s) => s.t === t && s.u !== p.unit));
  const rows = traces.map((t) => {
    const ps = panes.flatMap((p) => p.series.filter((s) => s.t === t).map((s) => ({ s, unit: s.u })));
    const fmt = (unit: string, v: number) => (Number.isNaN(v) ? "–" : unit === "dB" ? `${v.toFixed(2)} dB` : unit === "°" ? `∠${v.toFixed(1)}°` : si(v, unit));
    const val = (x: number | null) => (x == null || !ps.length ? "–" : ps.map(({ s, unit }) => fmt(unit, at(xs, s.y, x))).join(" "));
    // (a .step family: each value with its step, in a tooltip; each pane's series of this trace once)
    // (not for hundreds of steps: a tooltip that long helps no one, and it is built per hover move)
    const curves = ps[0]?.s.n ?? 0;
    const named = (x: number | null) => {
      if (x == null || !steps.length || curves < 2 || curves > 50) return undefined;
      return ps.map(({ s, unit }) => `${member(s, steps)}: ${fmt(unit, at(xs, s.y, x))}`).join("\n");
    };
    const delta = a != null && b != null && ps.length ? ps.map(({ s, unit }) => fmt(unit, at(xs, s.y, b) - at(xs, s.y, a))).join(" ") : "–";
    return { t, val, delta, named };
  });
  const dx = a != null && b != null ? b - a : null;
  // average, RMS, integral (LTspice: ctrl-click a trace label) between the cursors, else over what is shown
  const zoom = useS((s) => s.plot.zoom);
  const [lo, hi] = dx != null ? [Math.min(a!, b!), Math.max(a!, b!)] : zoom ?? [Math.min(xs[0], xs[xs.length - 1]), Math.max(xs[0], xs[xs.length - 1])];
  // (time and DC sweeps: over a frequency axis an average or integral of a magnitude means little.
  // xs is one sweep segment, rising or falling: nested .dc families are separate series of it)
  const num = (v: number, u: string) => (Number.isNaN(v) ? "–" : si(v, u)); // (no data in the range)
  const stats = useMemo(() => xUnit === "Hz" ? [] : traces.filter((t) => !t.data?.constant).flatMap((t) => panes.flatMap((p) => p.series.filter((s) => s.t === t && p.unit !== "dB" && p.unit !== "°").map((s) => ({ t, unit: s.u, of: member(s, steps), ...integrate(xs, s.y, lo, hi) })))),
    [traces, panes, xs, lo, hi, xUnit, steps]); // not per hover move
  return (
    <>
      <table className="w-full table-fixed font-mono text-2xs">
        <thead className="text-text-tertiary">
          <tr className="h-[22px] border-b border-border">
            <th className="w-[38%] pl-2 text-left font-normal">{plot}{steps.length ? ` · ${steps.length} step${steps.length > 1 ? "s" : ""}` : ""} · x</th>
            <th className="text-right font-normal">{hover == null ? "cursor" : si(hover, xUnit)}</th>
            <th className="text-right font-normal text-selection">A {a == null ? "" : si(a, xUnit)}</th>
            <th className="pr-2 text-right font-normal text-selection">B {b == null ? "" : si(b, xUnit)}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ t, val, delta, named }) => (
            <tr key={t.expr} className="group h-[22px] border-b border-border-subtle align-middle" title={t.error ?? t.expr}
              onContextMenu={(e) => (e.preventDefault(), openMenu(e.clientX, e.clientY, [
                ...(moved(t) ? [] : [{ label: t.right ? "Back to the left axis" : "Right axis (own scale)", hint: !t.right && alone(t) ? "alone in its pane" : undefined, disabled: !t.right && alone(t), run: () => setTraceRight(t.expr, !t.right) }]),
                // (into another unit's pane: on its right axis there; not the Bode panes, nor in a spectrum)
                ...(t.data?.im || plot === "FFT" ? [] : panes.filter((p) => p.unit !== "dB" && p.unit !== "°" && !p.series.some((s) => s.t === t)).map((p) => ({ label: `Into the ${p.unit || "unitless"} pane`, hint: "its right axis", run: () => setTracePane(t.expr, p.unit) }))),
                ...(t.pane != null && plot !== "FFT" ? [{ label: `Back to its own pane (${t.unit || "unitless"})`, run: () => setTracePane(t.expr, undefined) }] : []),
                { label: "Copy expression", hint: t.expr, run: () => navigator.clipboard.writeText(t.expr) },
                { label: `Remove ${t.label}`, danger: true, run: () => removeTrace(t.expr) },
              ]))}>
              <td className="truncate pl-2 text-text">
                <span className="mr-1.5 inline-block h-2 w-2 rounded-full align-middle" style={{ background: `var(--series-${t.slot + 1})` }} />
                <span className={t.error ? "text-status-danger" : ""}>{t.label}</span>
                {panes.some((p) => p.series.some((s) => s.t === t && scaleOf(p, s) === "y2")) && <span className="ml-1 text-text-tertiary" title="on the right axis">⇥</span>}
                <button className="row-actions ml-1 align-middle text-text-tertiary hover:text-text-strong" aria-label={`Remove ${t.label}`} onClick={() => removeTrace(t.expr)}>
                  <X className="inline h-3 w-3" />
                </button>
              </td>
              <td className="truncate text-right text-text-secondary" title={named(hover)}>{val(hover)}</td>
              <td className="truncate text-right" title={named(a)}>{val(a)}</td>
              <td className="truncate pr-2 text-right" title={((n) => `${n ? `${n}\n` : ""}B−A: ${delta}`)(named(b))}>{val(b)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {!!steps.length && (
        <div className="border-b border-border px-2 py-1 font-mono text-2xs text-text-secondary" title="Each trace has a curve per step; its values above are in this order">
          <span className="text-text-tertiary">.step: </span>{steps.length > 12 ? `${steps.slice(0, 10).join(" · ")} … ${steps[steps.length - 1]}` : steps.join(" · ")}
        </div>
      )}
      {dx != null && (
        <div className="border-b border-border px-2 py-1 font-mono text-2xs text-text-secondary">
          Δx = {si(dx, xUnit)}{xUnit === "s" && dx ? ` · 1/Δx = ${si(1 / Math.abs(dx), "Hz")}` : ""}
          {rows.map(({ t, delta }) => <div key={t.expr} className="truncate">Δ {t.label} = {delta}</div>)}
        </div>
      )}
      {!!stats.length && hi > lo && (
        <div className="border-b border-border px-2 py-1 font-mono text-2xs text-text-secondary">
          <div className="text-text-tertiary">{dx != null ? "A–B" : zoom ? "shown range" : "whole run"}: average · RMS · integral</div>
          {stats.map(({ t, unit, of, avg, rms, area }, k) => (
            <div key={`${t.expr}${k}`} className="truncate" title={`${t.label}${of ? ` ${of}` : ""} from ${si(lo, xUnit)} to ${si(hi, xUnit)}`}>
              {t.label}{of ? ` ${of}` : ""}: {num(avg, unit)} · {num(rms, unit)} · {num(area, unitTimes(unit, xUnit))}
            </div>
          ))}
        </div>
      )}
    </>
  );
});
