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

/** 10^e exactly: `10 ** -5` is 9.999999999999999e-6 in some engines (Node 22). */
const pow10 = (e: number) => Number(`1e${e}`);
/** The place of a tick step's last digit: uPlot steps by 1, 2, 2.5 and 5 × 10^k, so 2.5e-4 → -5. */
const lastPlace = (incr: number) => {
  const e = Math.floor(Math.log10(incr)), m = incr / pow10(e);
  return e - (Math.abs(m - Math.round(m)) > 1e-9 ? 1 : 0);
};
/** A tick label's significant digits: as many as its step needs, 3 at least. Zoomed in on 5 V, its
 *  ticks read 5.00025 V, 5.0005 V, not 5 V each. */
export const tickDigits = (v: number, incr: number) => Math.min(12, Math.max(3, Math.floor(Math.log10(Math.abs(v))) - lastPlace(incr) + 1));
/** The tick step uPlot takes for at least `x`: 1, 2, 2.5 or 5 × 10^k (4.2e-5 → 5e-5). */
export const niceStep = (x: number) => {
  const p = pow10(Math.floor(Math.log10(x)));
  return [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= x * (1 - 1e-12)) ?? x;
};
/** How far apart x ticks go (px): 50, as uPlot's own, while their labels fit there (7 characters);
 *  else as far as they are long, up to 13 characters. Counted, not formatted (a round value's label
 *  drops its zeros): the digits the step at 50 px needs and one more (uPlot may step by 2.5 at the
 *  wider space), point, prefix, unit, a sign below 0, a character between; 6.6 px a character. */
export const xTickSpace = (lo: number, hi: number, dim: number, unit: string) => {
  if (!(hi > lo) || !(dim > 0)) return 50;
  const d = tickDigits(Math.max(Math.abs(lo), Math.abs(hi)), niceStep(((hi - lo) * 50) / dim));
  const chars = d + 2 + unit.length + (lo < 0 ? 1 : 0);
  return chars <= 7 ? 50 : Math.min(chars + 2, 13) * 6.6;
};
/** A tick label's decimals (dB, degrees): 2.5 → 1, 0.25 → 2, 20 → 0. */
export const tickDecimals = (incr: number) => Math.max(0, -lastPlace(incr));
