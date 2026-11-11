// Self-check of src/spice.ts (the form editors' SPICE parsing/formatting): node scripts/spice-check.ts
import assert from "node:assert/strict";
import { box, connectivity, dragPlan, expandPowers, hit, outline, POWER_CALL, powersIn, shifted } from "../src/geom.ts";
import {
  tokens, formatAnalysis, formatIc, formatInclude, formatMeas, formatModel, formatParams, formatSource, numInfo, parseAnalysis,
  parseIc, parseInclude, parseMeas, parseModel, parseParams, parseSource, sample, si, spiceNum, directiveKind, analysisRoundTrips, analysisEditable, analysisOptions, analysisLineIndex, at, integrate, stripAnalyses, portablePaths, sourceRoundTrips,
  exprUnit, unitTimes, fftSize, fftFreqs, spectrum, getParam, setParam, balanced, paramValue,
} from "../src/spice.ts";

// numbers and the classic pitfalls
assert.equal(spiceNum("4.7k"), 4700);
assert.equal(spiceNum("1Meg"), 1e6);
assert.equal(spiceNum("10uF"), 10e-6);
assert.equal(si(999.6, "Hz", 3), "1kHz"); // rounding up crosses into the next prefix
assert.equal(si(-999.99e3), "-1M");
assert.equal(spiceNum("10kΩ"), 1e4); // the netlist drops the Ω
assert.match(numInfo("1MΩ")?.warn ?? "", /milli/);
assert.equal(spiceNum("1MHz"), 1e-3); // SPICE reads M as milli
assert.equal(spiceNum("{R*2}"), null);
assert.match(numInfo("1MHz", "Hz")!.warn!, /milli/);
assert.equal(numInfo("1Meg", "Hz")!.warn, undefined);
assert.match(numInfo("10F", "F")!.warn!, /femto/);
assert.match(numInfo("abc!")!.warn!, /not a number/);

// sources: LTspice's three attributes in, the same out
const s = parseSource("SINE(0 10m 1k)", "AC 1", "Rser=50");
assert.deepEqual([s.func, s.args, s.acMag, s.rser], ["SINE", ["0", "10m", "1k"], "1", "50"]);
assert.deepEqual(formatSource(s), { Value: "SINE(0 10m 1k)", Value2: "AC 1", SpiceLine: "Rser=50" });
assert.deepEqual(formatSource(parseSource("SINE(0 10m 1k) AC 1")), { Value: "SINE(0 10m 1k)", Value2: "AC 1", SpiceLine: "" });
assert.deepEqual(formatSource(parseSource("AC 1")), { Value: "AC 1", Value2: "", SpiceLine: "" });
assert.deepEqual(formatSource(parseSource("5")), { Value: "5", Value2: "", SpiceLine: "" });
// a gap is filled with the default, trailing blanks are dropped; unknown parts survive
const p = parseSource("PULSE(0 5) foo=1");
p.args[5] = "2m";
assert.equal(formatSource(p).Value, "PULSE(0 5 0 1n 1n 2m) foo=1"); // unknown parts stay where they were
// preview: a 1 kHz sine of amplitude 1 peaks near 1
const w = sample(parseSource("SINE(0 1 1k)"))!;
assert.ok(Math.abs(Math.max(...w.map(([, y]) => y)) - 1) < 0.01);
assert.equal(sample(parseSource("SINE(0 {A} 1k)")), null);

// analyses round-trip
for (const l of [".tran 1m", ".tran 0 5m 0 1u uic", ".ac dec 100 10 1Meg", ".dc V1 0 5 10m V2 0 3 1", ".noise V(out) V1 dec 10 10 1Meg",
  ".tf V(out,in) V1", ".tf I(V2) V1", ".pz in 0 out 0 vol pz", ".sens V(out)", ".sens V(out) dc", ".sens I(V1) ac dec 10 10 1Meg", ".op"]) {
  const { kind, f } = parseAnalysis(l);
  assert.equal(formatAnalysis(kind, f), l, l);
}
assert.equal(formatAnalysis("tran", { ...parseAnalysis(".tran 1m startup").f }), ".tran 1m startup"); // LTspice options kept

// directives
assert.deepEqual(parseParams(".param R = 1k C={1/(2*pi*R*f)}"), [["R", "1k"], ["C", "{1/(2*pi*R*f)}"]]);
assert.equal(formatParams([["R", "1k"], ["", ""]]), ".param R=1k");
assert.deepEqual(parseIc(".ic V(out)=1 v(n2) = 0"), [["out", "1"], ["n2", "0"]]);
assert.equal(formatIc([["out", "1"]]), ".ic V(out)=1");
assert.deepEqual(parseInclude('.lib "my lib.lib" fast'), { lib: true, path: "my lib.lib", section: "fast" });
assert.equal(formatInclude(false, "models/x.mod", ""), '.include "models/x.mod"');
const m = parseModel(".model D1 D(Is=1n N = 1.8)")!;
assert.deepEqual(m, { name: "D1", type: "D", params: [["Is", "1n"], ["N", "1.8"]] });
assert.equal(formatModel(m.name, m.type, m.params), ".model D1 D(IS=1n N=1.8)");
assert.equal(parseModel(".model X VDMOS(Rd=1)"), null); // not in the form: edited as text
for (const l of [".meas tran vavg AVG V(out) FROM=1m TO=2m", ".meas ac f3db WHEN V(out)=0.707 FALL=1", ".meas tran v1m FIND V(out) AT=1m",
  ".meas tran tpd TRIG V(in) VAL=0.5 RISE=1 TARG V(out) VAL=0.5 FALL=1"]) {
  assert.equal(formatMeas(parseMeas(l)!), l, l);
}
assert.equal(directiveKind(".param a=1"), "param");
assert.equal(directiveKind(".param a=1\n.param b=2"), "text");
assert.equal(directiveKind(".meas tran x DERIV V(out) AT=1m"), "text");
// a form never rewrites what it cannot show: such lines open as text
for (const l of [".meas tran x FIND V(out) WHEN V(in)=0.5", ".meas tran a AVG V(out) TD=1m", ".param R=1k ; load", ".model D1 D(Is=1n) ; fast",
  ".ic I(L1)=1m", ".ic V(out)=1 I(L1)=0", ".temp 25 50 75", ".model D1 D Is=1n"]) assert.equal(directiveKind(l), "text", l);
for (const [l, k] of [[".ic V(out)=1", "ic"], [".temp 25", "temp"], [".meas ac f3db WHEN V(out)=0.707", "meas"], [".inc models/x.lib", "include"], [".model D1 D(Is=1n N = 1.8)", "model"]]) assert.equal(directiveKind(l), k, l);
assert.equal(formatMeas(parseMeas(".meas ac f3db WHEN V(out)=0.707")!), ".meas ac f3db WHEN V(out)=0.707"); // no edge added
assert.ok(analysisRoundTrips(".tran 1m startup") && analysisRoundTrips(".noise v(out) V1 dec 10 10 1Meg 1"));
assert.equal(analysisRoundTrips(".dc lin V1 0 5 0.1"), false);
// average/RMS/integral between cursors: a ramp 0..2 over 0..2 (also sampled downwards, as a sweep may be)
{
  const r = integrate([0, 1, 2], [0, 1, 2], 0, 2);
  assert.ok(Math.abs(r.area - 2) < 1e-12 && Math.abs(r.avg - 1) < 1e-12 && Math.abs(r.rms - Math.sqrt(4 / 3)) < 1e-12, JSON.stringify(r));
  assert.ok(Math.abs(integrate([2, 1, 0], [2, 1, 0], 0.5, 1.5).avg - 1) < 1e-12);
  assert.equal(at([0, 1], [0, 10], 0.25), 2.5);
  assert.ok(Math.abs(integrate([0, 1, 2], [1, 1, 1], -5, 5).avg - 1) < 1e-12, "a range beyond the data: averaged over the data");
  const g = integrate([0, 1, 2, 3, 4], [1, 1, null, 3, 3], 0, 4); // a gap between 1 and 3
  assert.ok(Math.abs(g.area - 4) < 1e-12 && Math.abs(g.avg - 2) < 1e-12, JSON.stringify(g));
}
// result tabs: one per kind, the active one wins, alternatives from comments, fixed order
{
  const o = analysisOptions([{ text: ".ac dec 10 1 1k", directive: false }, { text: ".param R=1k\n.tran 0 1m\n+ 0 1u", directive: true }, { text: ".ac dec 50 1 1Meg", directive: true }, { text: "a note" }]);
  assert.deepEqual(o.map((x) => [x.kind, x.line, x.active]), [["tran", ".tran 0 1m\n+ 0 1u", true], ["ac", ".ac dec 50 1 1Meg", true]]);
  assert.equal(analysisOptions([{ text: "Try this later:\n.tran 5m" }]).length, 0, "a note mentioning an analysis is no alternative");
  assert.equal(analysisOptions([{ text: ".ac dec 10 1 1k\n+ " }]).length, 1);
  const note = ".tran is great\n.tran 0 1m startup".split("\n");
  assert.equal(analysisLineIndex(note, "tran", false), 1, "a comment's note line is not the analysis");
  assert.equal(analysisLineIndex(note, "tran", true), 0, "a directive's line is");
  assert.deepEqual(analysisOptions([{ text: ".op point is checked first" }, { text: ".tran is great" }, { text: ".tran 0 1m startup" }, { text: ".pz shows poles" }, { text: ".tf is a transfer function" }, { text: ".pz in 0 out 0 vol pz" }]).map((o) => o.kind), ["tran", "pz"], "notes are not analyses");
  assert.equal(stripAnalyses(".param R=1k\n.tran 0 1m\n* c\n+ 0 1u\n.meas tran x AVG V(a)"), ".param R=1k\n* c\n.meas tran x AVG V(a)");
}
{ const { f } = parseAnalysis(".sens V(out) dc"); assert.equal(formatAnalysis("sens", { ...f, mode: "ac" }), ".sens V(out) ac dec 10 10 1Meg"); } // `dc` is the mode, not an option
assert.ok(analysisEditable(".param a=1\n.tran 0 1m") && !analysisEditable(".tran 0 1m\n* c\n+ 0 1u") && !analysisEditable(".param a=1"));
// extra function arguments (PULSE cycles) survive an edit elsewhere
const cyc = parseSource("PULSE(0 5 0 1n 1n 0.5m 1m 10)");
cyc.acMag = "1";
assert.equal(formatSource(cyc).Value, "PULSE(0 5 0 1n 1n 0.5m 1m 10)");
assert.equal(portablePaths(".include C:\\models\\new.lib\nR1 a b 1k"), ".include C:/models/new.lib\nR1 a b 1k");
// PWL: a half-filled row is left out, not shifted into the next pair
const pwl = parseSource("PWL(0 0 2m 1)");
pwl.args = ["0", "0", "1m", "", "2m", "1"];
assert.equal(formatSource(pwl).Value, "PWL(0 0 2m 1)");
// preview of a PULSE without width/period: one step, not a 1 ms square wave
const step = sample(parseSource("PULSE(0 5 1m)"))!;
assert.ok(step.every(([t, v]) => (t < 1e-3 ? v === 0 : v === 5)));
assert.equal(sample(parseSource("SINE(0 1)")), null); // frequency from .tran: no preview
// TRIG without an edge does not borrow TARG's
assert.equal(directiveKind(".meas tran tpd TRIG V(in) VAL=0.5 TARG V(out) VAL=0.5 FALL=1"), "meas");
// sources the form cannot read open as text; harmless spelling differences do not count
for (const v of [["SINE(0 10m 1k)", "AC 1", "Rser=50"], ["SIN(0 1 1k) AC 1"], ["DC 5"], ["PWL(0,0 1m,1)"], ["AC 1 SINE(0 1 1k)"], ["5 Rpar=1Meg"]]) assert.ok(sourceRoundTrips(...v), v.join(" "));
for (const v of [["SINE (0 1 1k)"], ["PWL file=wave.txt"]]) assert.equal(sourceRoundTrips(...v), false, v.join(" "));
assert.equal(numInfo("1Mil")!.warn, undefined); // mil is 25.4 um, not the milli trap
// pass 4: holes in the argument list, commas in expressions, quotes, option position
const holes = parseSource("SINE(0 1 1k)");
holes.args[5] = "30";
holes.args = [...holes.args];
assert.equal(formatSource({ ...holes, args: Array.from(holes.args) }).Value, "SINE(0 1 1k 0 0 30)");
assert.equal(formatSource(parseSource("SINE(0 {max(a,b)} 1k)")).Value, "SINE(0 {max(a,b)} 1k)");
assert.equal(formatInclude(false, parseInclude(".inc 'x.lib'").path, ""), '.include "x.lib"');
assert.deepEqual(formatSource(parseSource("PWL(0 0 1m 1) r=0", "AC 1")), { Value: "PWL(0 0 1m 1) r=0", Value2: "AC 1", SpiceLine: "" });
assert.ok(sample(parseSource("EXP(0 1 0 0)"))!.every(([, v]) => Number.isFinite(v))); // tau 0: no NaN
// TRNOISE: read and written back; its preview a noise of about the rms asked for
const nz = parseSource("TRNOISE(0.1 1u 0 0)");
assert.equal(nz.func, "TRNOISE");
assert.equal(formatSource(nz).Value, "TRNOISE(0.1 1u 0 0)");
const nzs = sample(nz)!;
const nzr = Math.sqrt(nzs.reduce((a, [, v]) => a + v * v, 0) / nzs.length);
assert.ok(nzr > 0.05 && nzr < 0.15, `rms ${nzr}`);
assert.ok(sourceRoundTrips("") && sourceRoundTrips(undefined, "", "")); // an empty source is DC 0, not text
assert.equal(analysisRoundTrips(".ac list 1k 2k 5k"), false); // the form has no `list` sweep
for (const v of ["AC 1 2 3", "DC PULSE(0 5 0 1n 1n 1m 2m)"]) assert.equal(sourceRoundTrips(v), false, v); // would change meaning
assert.equal(formatSource(parseSource("SINE(0 'max(a,b)' 1k)")).Value, "SINE(0 'max(a,b)' 1k)"); // quoted commas stay
assert.equal(formatSource({ ...parseSource("SINE(0 1 1k)"), args: [] }).Value, "SINE(0 1)"); // required ones: the shown defaults
assert.equal(sample({ ...parseSource("SINE(0 1 1k)"), args: [] }), null); // frequency left to SPICE (from .tran): no preview
assert.equal(spiceNum("1a"), 1e-18); // ngspice reads a as atto (checked against the shared library)
assert.match(numInfo("1A", "A")!.warn!, /atto/);
assert.deepEqual([parseSource(undefined, undefined, "Rser = 50").rser, parseSource(undefined, undefined, "Rser = 50").rest], ["50", ""]);
assert.deepEqual(tokens('PWL file="my wave.txt" r=0'), ["PWL", 'file="my wave.txt"', "r=0"]); // double quotes like schematic.rs
assert.equal(numInfo("1Meg", "Hz")!.text, "= 1MegHz"); // not "1M", the milli trap's spelling
// dragging keeps every connection: a T-junction on a stretched wire, a T-leg on a moving wire
{
  const W = (x1: number, y1: number, x2: number, y2: number) => ({ t: "wire" as const, x1, y1, x2, y2 });
  const items = [{ t: "flag" as const, x: 0, y: 0, name: "a" }, W(0, 0, 100, 0), W(50, 0, 50, 50)];
  const dot = (its: typeof items) => connectivity(its as never, {}).dots.some(([x, y]) => x === 50 && y === 0);
  assert.ok(dot(items));
  const moved = dragPlan(items as never, {}, new Set([0]))(0, 20);
  assert.ok(dot(moved as never), `the junction at 50,0 stays: ${JSON.stringify(moved)}`);
  const leg = dragPlan(items as never, {}, new Set([1]))(0, 20).find((w) => w.t === "wire" && w.x1 === 50 && w.x2 === 50) as { y1: number; y2: number };
  assert.ok(leg.y1 === 20 && leg.y2 === 50, `the T-leg follows the moved wire: ${JSON.stringify(leg)}`);
  assert.equal(dragPlan(items as never, {}, new Set([0]))(0, 20).length, 4, "the stretched wire is cut at the junction");
  assert.deepEqual(dragPlan(items as never, {}, new Set([0]))(0, 0), items, "dragged back: the wire is whole again");
  // a bus with three junctions, dragged from one end: cut at the nearest junction only
  const bus = [{ t: "flag" as const, x: 0, y: 0, name: "a" }, W(0, 0, 100, 0), W(25, 0, 25, 50), W(50, 0, 50, 50), W(75, 0, 75, 50)];
  const busOut = dragPlan(bus as never, {}, new Set([0]))(0, 20);
  assert.ok(busOut.length === 6 && [25, 50, 75].every((x) => connectivity(busOut as never, {}).dots.some((p) => p[0] === x && p[1] === 0)), JSON.stringify(busOut));
  // a wire between two moving parts moves whole (no V), its T-leg stretches
  const two = [{ t: "flag" as const, x: 0, y: 0, name: "a" }, { t: "flag" as const, x: 100, y: 0, name: "b" }, W(0, 0, 100, 0), W(50, 0, 50, 50)];
  const out = dragPlan(two as never, {}, new Set([0, 1]))(0, 20) as ReturnType<typeof W>[];
  assert.ok(out.length === 4 && out[2].y1 === 20 && out[2].y2 === 20 && out[3].y1 === 20 && out[3].y2 === 50, JSON.stringify(out));
  // a resting part's pin on that wire: it stretches around the pin instead (the pin keeps its wires)
  const lib = { r: { name: "r", pins: [{ name: "a", x: 0, y: 0 }], shapes: [], prefix: "R", value: "", spice_line: "", desc: "" } };
  const withPin = [...two, { t: "sym" as const, sym: "r", x: 50, y: 0, rot: "R0", attrs: {} }];
  const bent = dragPlan(withPin as never, lib as never, new Set([0, 1]))(0, 20);
  assert.ok(connectivity(bent as never, lib as never).dots.some(([x, y]) => x === 50 && y === 0), JSON.stringify(bent));
  // a selected wire ending on a resting pin: that end stays (the other wire there too)
  const atPin = [{ t: "sym" as const, sym: "r", x: 0, y: 0, rot: "R0", attrs: {} }, W(0, 0, 50, 0), W(0, 0, 0, 50)];
  const pulled = dragPlan(atPin as never, lib as never, new Set([1]))(0, 20);
  assert.ok(connectivity(pulled as never, lib as never).dots.some(([x, y]) => x === 0 && y === 0), JSON.stringify(pulled));
  // a selected wire between two resting pins moves, bridged back to both pins
  const twoPins = [{ t: "sym" as const, sym: "r", x: 0, y: 0, rot: "R0", attrs: {} }, { t: "sym" as const, sym: "r", x: 50, y: 0, rot: "R0", attrs: {} }, W(0, 0, 50, 0)];
  const bridgedOut = dragPlan(twoPins as never, lib as never, new Set([2]))(0, 20) as ReturnType<typeof W>[];
  assert.ok(bridgedOut[2].y1 === 20 && bridgedOut.length === 5 && [0, 50].every((x) => bridgedOut.some((w) => w.t === "wire" && w.x1 === x && w.y1 === 0 && w.y2 === 20)), JSON.stringify(bridgedOut));
  const legOut = dragPlan([...twoPins, W(25, 0, 25, 50)] as never, lib as never, new Set([2]))(0, 20) as ReturnType<typeof W>[];
  assert.ok(legOut[3].y1 === 20 && legOut[3].y2 === 50, `a T-leg on a bridged wire follows it: ${JSON.stringify(legOut)}`);
  // with a resting pin inside it as well: bridged there too (it moved not at all)
  const threePins = [...twoPins.slice(0, 2), { t: "sym" as const, sym: "r", x: 25, y: 0, rot: "R0", attrs: {} }, W(0, 0, 50, 0)];
  const threeOut = dragPlan(threePins as never, lib as never, new Set([3]))(0, 20) as ReturnType<typeof W>[];
  assert.ok(threeOut[3].y1 === 20 && threeOut.filter((w) => w.t === "wire" && w.y1 === 0 && w.y2 === 20).length === 3, JSON.stringify(threeOut));
  // a selected label on a resting pin still takes its wires along (as before)
  const flagOnPin = [{ t: "sym" as const, sym: "r", x: 0, y: 0, rot: "R0", attrs: {} }, { t: "flag" as const, x: 0, y: 0, name: "0" }, W(0, 0, 0, 50)];
  const flagOut = dragPlan(flagOnPin as never, lib as never, new Set([1]))(20, 0) as ReturnType<typeof W>[];
  assert.ok(flagOut[2].x1 === 20 && flagOut[2].y1 === 0, JSON.stringify(flagOut));
  // the same wire selected alone: it stretches around the resting pin too
  const selWire = dragPlan(withPin as never, lib as never, new Set([2]))(0, 20);
  assert.ok(connectivity(selWire as never, lib as never).dots.some(([x, y]) => x === 50 && y === 0), JSON.stringify(selWire));
}

// units of trace expressions: dimensional analysis
{
  const vec = (n: string) => (n.endsWith("#branch") ? "A" : n === "time" ? "s" : "V");
  assert.equal(exprUnit("V(L1)*I(R1)"), "W");
  assert.equal(exprUnit("v(\"n001\")-v(\"n002\")"), "V");
  assert.equal(exprUnit("(v(a)-v(b))*i(vsense_r1)"), "W");
  assert.equal(exprUnit("V(out)/I(R1)"), "Ω");
  assert.equal(exprUnit("P(R1)+P(R2)+p(r3)"), "W");
  assert.equal(expandPowers("P(R1)+ p( r2 )-VP(out)+P(U1)", (n) => (/^r\d$/i.test(n) ? `v(${n})*i(${n})` : "")), "(v(R1)*i(R1))+ (v(r2)*i(r2))-VP(out)+P(U1)");
  assert.equal(expandPowers("P (R1)", (n) => `x${n}`), "(xR1)"); // (a space before the parenthesis: as exprUnit reads it)
  assert.equal(POWER_CALL.exec("2*P (U1)")?.[1], "U1");
  assert.equal(POWER_CALL.test("vp(out)+exp(1)"), false);
  // P() by netlist: R1 by its name, a diode D1 netlisted XD1 by its current I(D1), a two-pin
  // subcircuit U1 (XU1) by its netlist name, a three-pin one not at all
  const nl = { text: "", nets: [["a", "b"], ["c", "0"], ["x", "y", "z"], ["p", "q"]], insts: ["R1", "XD1", "XU2", "XU1"],
    probes: ["I(R1)", "I(D1)", "", "Ix(U1:A)"], pin_currents: [["I(R1)", ""], ["I(D1)", ""], ["Ix(U2:1)", "", ""], ["Ix(U1:A)", ""]], sheets: [], warnings: [] };
  assert.equal(powersIn("P(R1)+p(d1)", nl), '((v("a")-v("b"))*I(R1))+((v("c")-0)*I(D1))');
  assert.equal(powersIn("P(U1)", nl), '((v("p")-v("q"))*Ix(U1:A))');
  assert.equal(powersIn("P(U2)+P(R9)", nl), "P(U2)+P(R9)");
  assert.equal(exprUnit("I(R1)-I(R2)"), "A");
  assert.equal(exprUnit("Ib(Q1)+Ic(Q1)"), "A");
  assert.equal(exprUnit("ig(v1)"), "", "group delay or a gate current: not guessed");
  assert.equal(exprUnit("Ig(M1)*2", (n) => (n === "Ig(M1)" ? "A" : "")), "A", "listed by the run as a current");
  assert.equal(exprUnit("V(a)*V(a)/100"), "V²");
  assert.equal(exprUnit("v(out)+1"), "V", "a number takes the other side's unit");
  assert.equal(exprUnit("v(out)+i(v1)"), "", "volts plus amperes: no unit");
  assert.equal(exprUnit("integ(V(a)*I(R1))"), "J");
  assert.equal(exprUnit("deriv(v(c))"), "V/s");
  assert.equal(exprUnit("sqrt(v(a)*v(a))"), "V");
  assert.equal(exprUnit("abs(i(v1))"), "A");
  assert.equal(exprUnit("2*pi"), "");
  assert.equal(exprUnit("i(v1)/echarge"), "", "a constant with a unit of its own");
  assert.equal(exprUnit("0.5"), "");
  assert.equal(exprUnit("db(v(out))"), "");
  assert.equal(exprUnit("p_ges", vec), "V", "a node by its name");
  assert.equal(exprUnit("v1#branch*out", vec), "W");
  assert.equal(exprUnit("v(out)*@m1[gm]", (n) => (n.startsWith("@") ? "" : "V")), "", "a factor of unknown unit");
  assert.equal(exprUnit("v(a)^2"), "V²");
  assert.equal(exprUnit("v(a"), "V", "an open call: what was read");
  assert.equal(exprUnit("v(a) v(b)"), "", "not one expression");
  assert.equal(unitTimes("W", "s"), "J");
  assert.equal(unitTimes("A", "s"), "C");
  assert.equal(unitTimes("V", "s"), "V·s");
  assert.equal(unitTimes("", "s"), "s");
  assert.equal(unitTimes("V²", "s"), "V²·s");
  assert.equal(unitTimes("V/s", "s"), "V", "a composed unit is read back");
  assert.equal(unitTimes("V", "Hz"), "V·Hz");
  assert.equal(unitTimes("A", "V"), "A·V", "an integral over a .dc sweep is no power");
  assert.equal(unitTimes("Ω", "s"), "V·s/A", "no henry");
  assert.equal(exprUnit("deriv(v(out))", undefined, "Hz"), "");
  assert.equal(exprUnit("deriv(i(v1))", undefined, "V"), "", "over a .dc sweep: not named (no S, no W)");
  assert.equal(exprUnit("integ(i(v1))", undefined, "V"), "");
  assert.equal(exprUnit("integ(v(a))", undefined, "°C"), "", "an x unit it does not know");
  assert.equal(exprUnit("group_delay(v(out)/v(in))", undefined, "Hz"), "s");
  assert.equal(exprUnit("integ(v(a))", undefined, ""), "", "a temperature or resistance sweep");
}

// the FFT view: a 1 kHz sine of amplitude 2 on uneven time steps reads 6 dB at 1 kHz, little elsewhere
{
  const xs: number[] = [];
  for (let t = 0; t <= 10e-3; t += 1e-6 * (1 + 0.5 * Math.sin(t * 1e4))) xs.push(t);
  const ys = xs.map((t) => 5 + 2 * Math.sin(2 * Math.PI * 1e3 * t) + 0.02 * Math.sin(2 * Math.PI * 3e3 * t));
  const n = fftSize(xs, 0, 10e-3);
  assert.equal(n, 32768); // (as fine as the finest step, 0.5 µs)
  const f = fftFreqs(0, 10e-3, n), db = spectrum(xs, ys, 0, 10e-3, n);
  const bin = (hz: number) => db[f.findIndex((x) => Math.abs(x - hz) < 1)]!;
  assert.ok(Math.abs(bin(1e3) - 6.02) < 0.05, `${bin(1e3)}`);
  assert.ok(Math.abs(bin(3e3) + 33.98) < 0.5, `a harmonic at -34 dB: ${bin(3e3)}`);
  assert.ok(bin(5e3) < -80, `${bin(5e3)}`);
  assert.ok(db[0]! < -60, `the 5 V offset leaks into no bin (+14 dB with it): ${db[0]}`);
  assert.equal(f[0], 100);
  assert.ok(spectrum(xs, ys.map((y, k) => (k > xs.length / 2 ? NaN : y)), 0, 10e-3, 4096).every((v) => v == null), "a halted step: none");
  // a breakpoint's femtosecond step among 15: not 2^20 points
  assert.equal(fftSize([0, 1e-3, 1e-3 + 1e-15, ...Array.from({ length: 13 }, (_, k) => 2e-3 + k * 1e-3)], 0, 14e-3), 4096);
}

// a MOSFET's L and W in Value2, as LTspice keeps them
assert.equal(getParam("l=0.18u w=1u", "W"), "1u");
assert.equal(getParam("L = 2u off", "l"), "2u");
assert.equal(getParam("nl=3", "l"), "", "a word of its own");
assert.equal(setParam("l=0.18u w=1u", "w", "2u"), "l=0.18u w=2u");
assert.equal(setParam("off", "m", "4"), "off m=4");
assert.equal(setParam("l=1u m=2 off", "m", ""), "l=1u off");
assert.equal(setParam("", "area", " 2 "), "area=2");
assert.equal(getParam("l={2 * Lmin} w=1u", "l"), "{2 * Lmin}", "an expression whole");
assert.equal(setParam("l={2 * Lmin} w=1u", "l", "3u"), "l=3u w=1u");
assert.equal(getParam("m=2 off m=4", "m"), "4", "the last, as SPICE takes it");
assert.equal(setParam("m=2 off m=4", "m", "8"), "m=8 off");
assert.equal(getParam("l= w=1u", "w"), "1u", "an empty l is no w");
assert.equal(setParam("l= w=1u", "l", "2u"), "l=2u w=1u");
assert.equal(getParam("m = 4 temp =27", "temp"), "27");
assert.equal(setParam("l= {a==1?1u:2u} w=1u", "l", "3u"), "l=3u w=1u", "an expression after l= is its value");
assert.equal(setParam("w=1u", "l", "{2 * Lmin}"), "w=1u l={2 * Lmin}", "blanks inside braces stay");
assert.ok(!balanced("{2*") && !balanced("'a") && balanced("{2*(a+b)}") && balanced("1u"));
assert.equal(setParam("l= w =1u", "l", "2u"), "l=2u w=1u", "an empty l before a spaced w");
assert.equal(setParam("l= off", "l", "1u"), "l=1u off", "a flag is no value");
assert.ok(paramValue("1u w=3u") && paramValue("{2*") && paramValue("w=3u") && !paramValue("{a * b}") && !paramValue("{a==b?1:2}") && !paramValue(""));
assert.equal(getParam("l= Lmin w=1u", "l"), "Lmin", "a name after l=");
assert.equal(setParam("l= Lmin w=1u", "l", "2u"), "l=2u w=1u");

// a schematic's own drawings: drawn as shapes, moved by all their points, picked by their outline
{
  const rect = { t: "draw" as const, kind: "RECTANGLE", width: "Normal", p: [64, 80, -16, 0], style: 2 };
  assert.equal(outline(rect).length, 5, "a rectangle: its four sides");
  assert.deepEqual(box(rect, {}), [-16, 0, 64, 80]);
  assert.deepEqual((shifted(rect, 16, -16) as typeof rect).p, [80, 64, 0, -16]);
  assert.equal(hit([rect], {}, [24, 1], 4), 0, "its edge");
  assert.equal(hit([rect], {}, [24, 40], 4), -1, "not its inside: the circuit it frames");
  const arc = { t: "draw" as const, kind: "ARC", width: "Normal", p: [0, 0, 32, 32, 0, 16, 32, 16] };
  assert.equal(hit([arc], {}, [16, 32], 3), 0, "its drawn half, below (counter-clockwise from the left)");
  assert.equal(hit([arc], {}, [16, 0], 3), -1, "not the half it leaves out");
  const flat = { t: "draw" as const, kind: "CIRCLE", width: "Normal", p: [-100, -10, 100, 10] };
  assert.equal(hit([flat], {}, [150, 0], 6), -1, "a long ellipse: not from 50 past its end");
}

console.log("spice-check: ok");
