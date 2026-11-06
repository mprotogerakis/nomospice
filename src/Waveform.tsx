import { memo, useEffect, useMemo, useRef } from "react";
import { useSize } from "./lib/useSize";
import uPlot from "uplot";
import { create } from "zustand";
import { Plus, X } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { removeTrace, setPlot, useS } from "./store";
import { openMenu } from "./Menu";
import { at, integrate, si } from "./spice";
import { keys } from "./lib/utils";
import type { Trace } from "./types";
import { saveSvg, withLightTheme, xmlEscape } from "./svgExport";

type Series = { t: Trace; y: (number | null)[] };
type Pane = { unit: string; series: Series[] };

/** px a press may move and still count as a click (shared by uPlot's drag and our click rule). */
const CLICK_DIST = 3;

/** x under the plot crosshair (separate store: only the readout table re-renders on mouse moves). */
const useHover = create(() => ({ x: null as number | null }));

const css = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const color = (slot: number) => css(`--series-${slot + 1}`);
/** dB; an exactly-zero magnitude (AC-quiet node) is a gap, not -6000 dB squashing the pane. */
const mag = (t: Trace) => t.data!.re.map((re, i) => { const m = Math.hypot(re, t.data!.im?.[i] ?? 0); return m ? 20 * Math.log10(m) : null; });
function phase(t: Trace) {
  let off = 0, prev = 0;
  return t.data!.re.map((re, i) => {
    const p = (Math.atan2(t.data!.im?.[i] ?? 0, re) * 180) / Math.PI;
    if (i) off += p + off - prev > 180 ? -360 : p + off - prev < -180 ? 360 : 0; // unwrap
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

/** The plotted data as CSV: x, then one column per series (family curves numbered). */
function csv(xs: number[], xUnit: string, panes: Pane[]) {
  const cols = panes.flatMap((p) => p.series.map((s, k, all) => ({ name: `${s.t.label}${all.filter((o) => o.t === s.t).length > 1 ? ` #${all.filter((o, j) => o.t === s.t && j <= k).length}` : ""} [${p.unit}]`, y: s.y })));
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
      const px = (v: number) => L + u.valToPos(v, "x"), py = (v: number) => T + u.valToPos(v, "y");
      out.push(`<clipPath id="pane${k}"><rect x="${L}" y="${T}" width="${PW}" height="${PH}"/></clipPath>`);
      (ax[1]._splits ?? []).forEach((v, i) => {
        if (v == null) return;
        line(L, py(v), L + PW, py(v), grid);
        txt(L - 6, py(v), String(ax[1]._values?.[i] ?? ""), "end", "middle");
      });
      (ax[0]._splits ?? []).forEach((v, i) => {
        if (v == null) return;
        line(px(v), T, px(v), T + PH, grid);
        const label = ax[0]._values?.[i];
        if (k === us.length - 1 && label) txt(px(v), T + PH + 16, String(label));
      });
      out.push(`<rect x="${L}" y="${T}" width="${PW}" height="${PH}" fill="none" stroke="${grid}"/>`);
      const xs = u.data[0], { min = -Infinity, max = Infinity } = u.scales.x;
      panes[k].series.forEach((s, j) => {
        const ys = u.data[j + 1];
        let d = "", pen = false;
        for (let i = 0; i < xs.length; i++) {
          const v = ys[i];
          // outside the zoom: skip, but keep one point beyond each edge so the clipped line reaches it
          if ((xs[i + 1] ?? Infinity) < min || (xs[i - 1] ?? -Infinity) > max) continue;
          if (v == null || !Number.isFinite(v)) { pen = false; continue; }
          d += `${pen ? "L" : "M"}${px(xs[i]).toFixed(1)} ${py(v).toFixed(1)}`;
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
  const freq = run?.scale === "frequency";
  const xUnit = freq ? "Hz" : run?.scale === "time" ? "s" : run?.scale.startsWith("i") ? "A" : run?.scale.endsWith("sweep") && run.scale.startsWith("v") ? "V" : "";
  const ok = useMemo(() => traces.filter((t) => t.data?.x.length), [traces]);
  // .ac without an AC source: every value is exactly 0 (no dB at all, phase flat) - say so instead
  const zero = useMemo(() => ac && ok.length > 0 && ok.every((t) => t.data!.re.every((v, i) => !v && !t.data!.im?.[i])), [ac, ok]);
  const all = ok[0]?.data!.x ?? [];
  // nested .dc (family curves): one series per outer step over the first segment's x
  const seg = useMemo(() => segment(all), [all]);
  const xs = useMemo(() => all.slice(0, seg), [all, seg]);
  // frequency sweeps on a log axis, unless it is a linear sweep (evenly spaced, may start at 0 Hz)
  const linear = xs.length > 2 && Math.abs(xs[2] - 2 * xs[1] + xs[0]) <= 1e-9 * Math.abs(xs[1] - xs[0]);
  const logX = freq && xs[0] > 0 && !linear;
  const panes = useMemo<Pane[]>(() => {
    if (!ok.length || zero) return [];
    const split = (t: Trace, y: (number | null)[]): Series[] =>
      Array.from({ length: Math.max(1, Math.floor(y.length / seg)) }, (_, k) => ({ t, y: y.slice(k * seg, (k + 1) * seg) }));
    // complex vectors (.ac, AC .sens) -> magnitude (dB) + phase panes; real ones (db(), ph() ...) as they are
    const cplx = ok.filter((t) => t.data!.im);
    const real = ok.filter((t) => !cplx.includes(t));
    const panes: Pane[] = [];
    if (cplx.length) panes.push({ unit: "dB", series: cplx.flatMap((t) => split(t, mag(t))) }, { unit: "°", series: cplx.flatMap((t) => split(t, phase(t))) });
    const units = [...new Set(real.map((t) => t.unit))].sort((a, b) => (a === "V" ? -1 : b === "V" ? 1 : a.localeCompare(b)));
    for (const unit of units) {
      const series = real.filter((t) => t.unit === unit).flatMap((t) => split(t, t.data!.re));
      const same = panes.find((p) => p.unit === unit); // db(...) joins the dB pane
      if (same) same.series.push(...series);
      else panes.push({ unit, series });
    }
    return panes;
  }, [ok, zero, seg]);

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
    panes.forEach((pane, pi) => {
      const el = paneEls.current[pi];
      if (!el) return;
      const last = pi === panes.length - 1;
      const span = pane.unit === "dB" ? 20 : pane.unit === "°" ? 90 : 0;
      const opts: uPlot.Options = {
        width: size.w, height: h,
        legend: { show: false },
        // dist: a click with 1-2 px of jitter stays a click (places a cursor) instead of a micro-zoom
        // click: no-op so uPlot does not swallow slightly-moved clicks; our handler decides (3 px rule)
        cursor: { sync: { key: "nomospice" }, drag: { x: true, y: false, setScale: true, dist: CLICK_DIST, click: () => {} }, points: { size: 6 } },
        scales: {
          // exactly the data: a log axis would otherwise round up to whole decades (1 MHz data -> 10 MHz)
          x: { time: false, distr: logX ? 3 : 1, ...(logX ? { range: (_u, lo, hi): uPlot.Range.MinMax => (lo < hi ? [lo, hi] : [lo / 2, hi * 2]) } : {}) },
          // dB/phase: at least `span` tall, so 0.1 dB of ripple does not fill the pane like a resonance
          ...(span ? { y: { range: (_u, lo, hi): uPlot.Range.MinMax => {
            if (lo == null || hi == null) return [lo, hi];
            if (hi - lo < span) [lo, hi] = [(lo + hi - span) / 2, (lo + hi + span) / 2];
            return uPlot.rangeNum(lo, hi, 0.05, true);
          } } } : {}),
        },
        axes: [
          { stroke: text, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid, width: 1 }, size: last ? 36 : 8, values: last ? (_u, v) => v.map((x) => (x == null ? "" : si(x, xUnit, 3))) : () => [], font: "11px IBM Plex Mono" },
          { stroke: text, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid, width: 1 }, size: 72, values: (_u, v, _i, _s, incr) => v.map((y) => (span ? `${+y.toFixed(Math.max(0, Math.ceil(-Math.log10(incr))))}${pane.unit}` : si(y, pane.unit, 3))), font: "11px IBM Plex Mono", label: pane.unit === "" ? "expr" : undefined },
        ],
        series: [{}, ...pane.series.map((s) => ({ label: s.t.label, stroke: color(s.t.slot), width: 2, points: { show: false } }))],
        hooks: {
          ready: [(u) => {
            // click = cursor; a drag (zoom) also ends in a click, so compare with the press position
            let down = 0;
            u.over.addEventListener("mousedown", (e) => {
              down = e.clientX;
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
            // drag-zoom (uPlot applies it after this hook): remember it as the user's zoom
            if (u.select.width > 0) setPlot({ zoom: [u.posToVal(u.select.left, "x"), u.posToVal(u.select.left + u.select.width, "x")] });
          }],
          setCursor: [(u) => useHover.setState({ x: u.cursor.idx == null ? null : (u.data[0][u.cursor.idx] ?? null) })],
          draw: [(u) => {
            const ctx = u.ctx;
            const { a, b } = useS.getState().plot;
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
      all.push(new uPlot(opts, [ord(xs), ...pane.series.map((s) => ord(s.y))] as uPlot.AlignedData, el));
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
  }, [panes, ac, logX, xUnit, xs, size.w > 0]);

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
        openMenu(e.clientX, e.clientY, [
          { label: "Reset zoom", hint: "dbl-click", disabled: !panes.length, run: () => { setPlot({ zoom: null }); plots.current[0]?.setScale("x", { min: xs[0], max: xs[xs.length - 1] }); } },
          { label: "Clear cursors", run: () => setPlot({ a: null, b: null }) },
          { label: "Add trace…", disabled: !run, run: () => useS.setState({ dialog: { k: "trace" } }) },
          "-",
          { label: "Copy data as CSV", disabled: !panes.length, run: () => navigator.clipboard.writeText(csv(xs, xUnit, panes)) },
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
        <Readout traces={traces} panes={panes} xs={xs} xUnit={xUnit} plot={run?.plot ?? ""} />
        <Measurements xUnit={xUnit} />
        <button
          className="m-2 inline-flex h-[22px] items-center justify-center gap-1 rounded-control border border-border-control text-text-secondary hover:bg-surface-hover hover:text-text-strong disabled:opacity-40"
          disabled={!run}
          onClick={() => useS.setState({ dialog: { k: "trace" } })}
        >
          <Plus className="h-3 w-3" /> Add trace
        </button>
        <p className="mt-auto px-2 pb-2 text-2xs leading-4 text-text-tertiary">
          Drag: zoom · wheel: zoom · double-click: reset · click: cursor A · shift-click: cursor B
        </p>
      </div>
    </div>
  );
});

/** `.meas` results of the run (ngspice reports them in the log; LTspice in its error log). */
export function Measurements({ xUnit }: { xUnit: string }) {
  const meas = useS((s) => s.sim?.run?.meas);
  const deck = useS((s) => s.sim?.simulated ?? "");
  // results on the x axis: TRIG/TARG (a span) and WHEN (a point) — not FIND/AVG … (a y value)
  const atX = useMemo(() => new Set(deck.split("\n").map((l) => l.trim().split(/\s+/)).filter((t) => /^\.meas/i.test(t[0] ?? "") && /^when$/i.test(t[3] ?? "")).map((t) => t[2].toLowerCase())), [deck]);
  if (!meas?.length) return null;
  return (
    <div className="border-b border-border px-2 py-1 font-mono text-2xs">
      <div className="text-text-tertiary">Measurements (.meas)</div>
      {meas.map(([name, v, info], k) => (
        <div key={`${name}${k}`} className="truncate" title={v == null ? "ngspice could not evaluate it: see the log (⌘L)" : info}>
          <span className="text-text">{name}</span> = {v == null ? <span className="text-status-danger">failed</span> : si(v, /targ=|trig=/.test(info) || atX.has(name.toLowerCase()) ? xUnit : "", 5)}
        </div>
      ))}
    </div>
  );
}

/** Trace table: values under the hover crosshair and at cursors A/B. Own component + hover store,
 *  so mouse moves over the plot re-render only this table. */
const Readout = memo(function Readout({ traces, panes, xs, xUnit, plot }: { traces: Trace[]; panes: Pane[]; xs: number[]; xUnit: string; plot: string }) {
  const hover = useHover((h) => h.x);
  const { a, b } = useS(useShallow((s) => ({ a: s.plot.a, b: s.plot.b })));
  const rows = traces.map((t) => {
    const ps = panes.flatMap((p) => p.series.filter((s) => s.t === t).map((s) => ({ s, unit: p.unit })));
    const fmt = (unit: string, v: number) => (Number.isNaN(v) ? "–" : unit === "dB" ? `${v.toFixed(2)} dB` : unit === "°" ? `∠${v.toFixed(1)}°` : si(v, unit));
    const val = (x: number | null) => (x == null || !ps.length ? "–" : ps.map(({ s, unit }) => fmt(unit, at(xs, s.y, x))).join(" "));
    const delta = a != null && b != null && ps.length ? ps.map(({ s, unit }) => fmt(unit, at(xs, s.y, b) - at(xs, s.y, a))).join(" ") : "–";
    return { t, val, delta };
  });
  const dx = a != null && b != null ? b - a : null;
  // average, RMS, integral (LTspice: ctrl-click a trace label) between the cursors, else over what is shown
  const zoom = useS((s) => s.plot.zoom);
  const [lo, hi] = dx ? [Math.min(a!, b!), Math.max(a!, b!)] : zoom ?? [Math.min(xs[0], xs[xs.length - 1]), Math.max(xs[0], xs[xs.length - 1])];
  // (time and DC sweeps: over a frequency axis an average or integral of a magnitude means little)
  const stats = useMemo(() => xUnit === "Hz" ? [] : traces.flatMap((t) => panes.flatMap((p) => p.series.filter((s) => s.t === t && p.unit !== "dB" && p.unit !== "°").map((s) => ({ t, unit: p.unit, ...integrate(xs, s.y, lo, hi) })))),
    [traces, panes, xs, lo, hi, xUnit]); // not per hover move
  return (
    <>
      <table className="w-full table-fixed font-mono text-2xs">
        <thead className="text-text-tertiary">
          <tr className="h-[22px] border-b border-border">
            <th className="w-[38%] pl-2 text-left font-normal">{plot} · x</th>
            <th className="text-right font-normal">{hover == null ? "cursor" : si(hover, xUnit)}</th>
            <th className="text-right font-normal text-selection">A {a == null ? "" : si(a, xUnit)}</th>
            <th className="pr-2 text-right font-normal text-selection">B {b == null ? "" : si(b, xUnit)}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ t, val, delta }) => (
            <tr key={t.expr} className="group h-[22px] border-b border-border-subtle align-middle" title={t.error ?? t.expr}
              onContextMenu={(e) => (e.preventDefault(), openMenu(e.clientX, e.clientY, [
                { label: "Copy expression", hint: t.expr, run: () => navigator.clipboard.writeText(t.expr) },
                { label: `Remove ${t.label}`, danger: true, run: () => removeTrace(t.expr) },
              ]))}>
              <td className="truncate pl-2 text-text">
                <span className="mr-1.5 inline-block h-2 w-2 rounded-full align-middle" style={{ background: `var(--series-${t.slot + 1})` }} />
                <span className={t.error ? "text-status-danger" : ""}>{t.label}</span>
                <button className="row-actions ml-1 align-middle text-text-tertiary hover:text-text-strong" aria-label={`Remove ${t.label}`} onClick={() => removeTrace(t.expr)}>
                  <X className="inline h-3 w-3" />
                </button>
              </td>
              <td className="truncate text-right text-text-secondary">{val(hover)}</td>
              <td className="truncate text-right">{val(a)}</td>
              <td className="truncate pr-2 text-right" title={`B−A: ${delta}`}>{val(b)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {dx != null && (
        <div className="border-b border-border px-2 py-1 font-mono text-2xs text-text-secondary">
          Δx = {si(dx, xUnit)}{xUnit === "s" && dx ? ` · 1/Δx = ${si(1 / Math.abs(dx), "Hz")}` : ""}
          {rows.map(({ t, delta }) => <div key={t.expr} className="truncate">Δ {t.label} = {delta}</div>)}
        </div>
      )}
      {!!stats.length && hi > lo && (
        <div className="border-b border-border px-2 py-1 font-mono text-2xs text-text-secondary">
          <div className="text-text-tertiary">{dx ? "A–B" : zoom ? "shown range" : "whole run"}: average · RMS · integral</div>
          {stats.map(({ t, unit, avg, rms, area }, k) => (
            <div key={`${t.expr}${k}`} className="truncate" title={`${t.label} from ${si(lo, xUnit)} to ${si(hi, xUnit)}`}>
              {t.label}: {si(avg, unit)} · {si(rms, unit)} · {si(area, unit && xUnit ? `${unit}·${xUnit}` : unit || xUnit)}
            </div>
          ))}
        </div>
      )}
    </>
  );
});
