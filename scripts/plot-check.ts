// Self-check of src/plotRange.ts (a plot's y range): node scripts/plot-check.ts (pnpm check runs it)
// Never two ends uPlot's ticks cannot tell apart: its loop over them would not end.
import assert from "node:assert/strict";
import { apart, wide, yRange } from "../src/plotRange.ts";

for (const v of [-3e5, -1, 1e-6, 0.4345556386526741, 1, 70, 1e5, 1e9]) for (let e = -17; e <= 0; e++) {
  const [lo, hi] = [Math.min(v, v + Math.abs(v) * 10 ** e), Math.max(v, v + Math.abs(v) * 10 ** e)], [a, b] = yRange(lo, hi);
  assert.ok(a <= lo && b >= hi && b - a >= 0.99e-9 * Math.abs(v), `${lo} to ${hi}: ${a} to ${b}`);
}
assert.ok(((r) => r[1] - r[0] >= 7e-8)(yRange(69.9999999999751, 70)), "all but constant: not rounded to one value");
assert.deepEqual(yRange(2.4999999999999996, 2.5000000000000004), yRange(2.5, 2.5), "rounding noise: as a constant");
assert.ok(((r) => r[1] - r[0] > 0)(yRange(0, 0)) && ((r) => r[1] - r[0] > 0)(yRange(5, 5)), "constant: a range about it");
assert.ok(((r) => r[1] - r[0] >= 20)(yRange(-3, -2.9, 20, 0.05)), "dB: at least its span");
assert.deepEqual(yRange(-3, -3, 20, 0.05), yRange(-3 - 1e-12, -3 + 1e-12, 20, 0.05).map(Math.round), "dB, constant: its span about it, as one all but constant");
assert.deepEqual(wide(0, 10), [0, 10]);
assert.ok(apart(...wide(250, 250 + 1e-13)) && apart(...wide(70, 70.00000000000001)) && apart(...wide(0, 0)), "zoomed in past its doubles: still apart");
assert.ok(!apart(70, 70.00000000000001) && !apart(1, 1) && apart(-1, 1));
console.log("plot-check: ok");
