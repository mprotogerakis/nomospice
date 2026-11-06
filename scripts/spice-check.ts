// Self-check of src/spice.ts (the form editors' SPICE parsing/formatting): node scripts/spice-check.ts
import assert from "node:assert/strict";
import {
  tokens, formatAnalysis, formatIc, formatInclude, formatMeas, formatModel, formatParams, formatSource, numInfo, parseAnalysis,
  parseIc, parseInclude, parseMeas, parseModel, parseParams, parseSource, sample, si, spiceNum, directiveKind, analysisRoundTrips, analysisEditable, analysisOptions, stripAnalyses, portablePaths, sourceRoundTrips, holdsAnalysis,
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
// result tabs: one per kind, the active one wins, alternatives from comments, fixed order
{
  const o = analysisOptions([{ text: ".ac dec 10 1 1k", directive: false }, { text: ".param R=1k\n.tran 0 1m\n+ 0 1u", directive: true }, { text: ".ac dec 50 1 1Meg", directive: true }, { text: "a note" }]);
  assert.deepEqual(o.map((x) => [x.kind, x.line, x.active]), [["tran", ".tran 0 1m\n+ 0 1u", true], ["ac", ".ac dec 50 1 1Meg", true]]);
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
assert.ok(sourceRoundTrips("") && sourceRoundTrips(undefined, "", "")); // an empty source is DC 0, not text
assert.equal(analysisRoundTrips(".ac list 1k 2k 5k"), false); // the form has no `list` sweep
assert.ok(holdsAnalysis({ t: "text", directive: true, text: ".param a=1\n.tran 1m" }) && !holdsAnalysis({ t: "text", directive: false, text: ".tran 1m" }));
for (const v of ["AC 1 2 3", "DC PULSE(0 5 0 1n 1n 1m 2m)"]) assert.equal(sourceRoundTrips(v), false, v); // would change meaning
assert.equal(formatSource(parseSource("SINE(0 'max(a,b)' 1k)")).Value, "SINE(0 'max(a,b)' 1k)"); // quoted commas stay
assert.equal(formatSource({ ...parseSource("SINE(0 1 1k)"), args: [] }).Value, "SINE(0 1)"); // required ones: the shown defaults
assert.equal(sample({ ...parseSource("SINE(0 1 1k)"), args: [] }), null); // frequency left to SPICE (from .tran): no preview
assert.equal(spiceNum("1a"), 1e-18); // ngspice reads a as atto (checked against the shared library)
assert.match(numInfo("1A", "A")!.warn!, /atto/);
assert.deepEqual([parseSource(undefined, undefined, "Rser = 50").rser, parseSource(undefined, undefined, "Rser = 50").rest], ["50", ""]);
assert.deepEqual(tokens('PWL file="my wave.txt" r=0'), ["PWL", 'file="my wave.txt"', "r=0"]); // double quotes like schematic.rs
assert.equal(numInfo("1Meg", "Hz")!.text, "= 1MegHz"); // not "1M", the milli trap's spelling
console.log("spice-check: ok");
