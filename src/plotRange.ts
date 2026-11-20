import uPlot from "uplot";

/** A y axis's range for its data's: uPlot's padded and rounded one (rangeNum), but never two ends
 *  its ticks cannot tell apart. A trace all but constant (70 °C drifting in its 10th digit) is
 *  rounded by rangeNum to [70, 70]; uPlot then steps its ticks by 1e-16, which changes no double
 *  near 70: its loop never ends, the page freezes and is killed for its memory. One within a
 *  billionth of its size is drawn as a constant; dB and phase at least `span` tall. */
export function yRange(lo: number, hi: number, span = 0, pad = 0.1): [number, number] {
  const least = span || Math.max(Math.abs(lo), Math.abs(hi)) * 1e-9;
  // (all but constant, below a billionth: drawn as a constant, whose range rangeNum never rounds
  // together; dB, phase: its span about it)
  if (!span && hi - lo < least) lo = hi = lo / 2 + hi / 2; // (halves: no overflow near 1e308)
  else if (span && hi >= lo && hi - lo < span) [lo, hi] = [(lo + hi - span) / 2, (lo + hi + span) / 2];
  const [a, b] = uPlot.rangeNum(lo, hi, pad, true);
  // (about the data, but for rangeNum's own rounding: -0 for a soft zero, data below 1e-24)
  if (a != null && b != null && Number.isFinite(a) && Number.isFinite(b) && b > a && b - a >= Math.max(hi - lo, least) && a <= lo + (b - a) * 1e-6 && b >= hi - (b - a) * 1e-6) return [a, b];
  // (rounded together, or not about the data: the data's own, a least height about them)
  const d = Math.max(hi - lo, least) || 1;
  return [lo / 2 + hi / 2 - d, lo / 2 + hi / 2 + d];
}

/** Whether an axis's ends are far enough apart for its ticks: a billionth of their size. */
export const apart = (lo: number, hi: number) => hi > lo && hi - lo >= 1e-9 * Math.max(Math.abs(lo), Math.abs(hi));

/** An axis's ends as set or zoomed to (the time axis, a fixed y range), kept apart: zoomed in
 *  past a billionth of where it is, it stays that wide about its middle. */
export function wide(lo: number, hi: number): [number, number] {
  if (apart(lo, hi)) return [lo, hi];
  const c = lo / 2 + hi / 2, w = 1e-9 * Math.max(Math.abs(lo), Math.abs(hi)) || 1e-9;
  return [c - w, c + w];
}

/** A tick label's significant digits: as many as its step needs, 3 at least, 6 at most (what the
 *  axis has room for). Zoomed in on 5 V, its ticks read 5.0004 V, 5.0008 V, not 5 V each. */
export const tickDigits = (v: number, incr: number) =>
  Math.min(6, Math.max(3, Math.floor(Math.log10(Math.abs(v))) - Math.floor(Math.log10(incr)) + 1));
