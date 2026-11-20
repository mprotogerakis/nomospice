// Self-check of src/plotRange.ts (a plot's y range): node scripts/plot-check.ts (pnpm check runs it)
// Never two ends uPlot's ticks cannot tell apart: its loop over them would not end.
import assert from "node:assert/strict";
import { apart, niceStep, tickDecimals, tickDigits, wide, yRange } from "../src/plotRange.ts";

for (const v of [-3e5, -1, 1e-6, 0.4345556386526741, 1, 70, 1e5, 1e9]) for (let e = -17; e <= 0; e++) {
  const [lo, hi] = [Math.min(v, v + Math.abs(v) * 10 ** e), Math.max(v, v + Math.abs(v) * 10 ** e)], [a, b] = yRange(lo, hi);
  assert.ok(a <= lo + (b - a) * 1e-6 && b >= hi - (b - a) * 1e-6 && b - a >= 0.99e-9 * Math.abs(v), `${lo} to ${hi}: ${a} to ${b}`);
}
assert.ok(((r) => r[1] - r[0] >= 7e-8)(yRange(69.9999999999751, 70)), "all but constant: not rounded to one value");
assert.deepEqual(yRange(2.4999999999999996, 2.5000000000000004), yRange(2.5, 2.5), "rounding noise: as a constant");
assert.ok(((r) => r[1] - r[0] > 0)(yRange(0, 0)) && ((r) => r[1] - r[0] > 0)(yRange(5, 5)), "constant: a range about it");
assert.ok(((r) => r[1] - r[0] >= 20)(yRange(-3, -2.9, 20, 0.05)), "dB: at least its span");
assert.deepEqual(yRange(-3, -3, 20, 0.05), yRange(-3 - 1e-12, -3 + 1e-12, 20, 0.05).map(Math.round), "dB, constant: its span about it, as one all but constant");
assert.deepEqual(yRange(-1e-30, 0), yRange(0, 1e-30), "a soft zero, below or above 0: flat about 0 alike");
assert.ok(yRange(1e308, 1e308).every(Number.isFinite) && wide(1e308, 1e308 + 1e298).every(Number.isFinite), "near the largest doubles: finite");
assert.deepEqual(wide(0, 10), [0, 10]);
assert.ok(apart(...wide(250, 250 + 1e-13)) && apart(...wide(70, 70.00000000000001)) && apart(...wide(0, 0)), "zoomed in past its doubles: still apart");
assert.ok(!apart(70, 70.00000000000001) && !apart(1, 1) && apart(-1, 1));
// (uPlot's steps: 1, 2, 2.5, 5 × 10^k)
assert.deepEqual([tickDigits(5.00025, 2.5e-4), tickDigits(5.0005, 5e-4), tickDigits(10.25, 0.25), tickDigits(2.5e-4, 2.5e-4), tickDigits(0, 1), tickDigits(1.2e-3, 2e-4), tickDigits(2.00001e-3, 1e-8), tickDigits(5.000001e-3, 1e-9)], [6, 5, 4, 3, 3, 3, 6, 7], "tick digits: as the step needs, 3 at least");
assert.deepEqual([2.5, 0.25, 5, 20, 0.5].map(tickDecimals), [1, 2, 0, 0, 1], "dB and degrees: decimals as the step needs");
assert.deepEqual([4.17e-5, 2.5, 0.3, 1, 7e3].map(niceStep), [5e-5, 2.5, 0.5, 1, 1e4], "uPlot's steps: 1, 2, 2.5, 5 × 10^k");
assert.equal(tickDigits(1e-3, niceStep(1e-3 * 50 / 1200)), 3, ".tran 1m on 1200 px: 3 digits, ticks 50 px apart");
console.log("plot-check: ok");
