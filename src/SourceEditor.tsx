// Independent source (V, I) as a form: waveform function with named fields, AC, parasitics,
// a waveform preview, and the SPICE attributes it writes (Value, Value2, SpiceLine like LTspice).
import { useMemo, useState } from "react";
import { FUNCS, formatSource, lastFilled, parseSource, sample, si, sourceRoundTrips, tokens, type Func, type Source } from "./spice";
import { Field, NumField, Rows, Section, SpiceLine, Tabs, TextField } from "./ui/dialog";
import { Button } from "./ui/button";

type Attrs = Record<string, string>;
const KINDS: [Func | "", string][] = [["", "DC"], ["SINE", "Sine"], ["PULSE", "Pulse"], ["EXP", "Exponential"], ["SFFM", "FM"], ["PWL", "Piecewise linear"]];

export function SourceEditor({ attrs: given, setAttrs, current, spiceLine }: { attrs: Attrs; setAttrs: (a: Attrs) => void; current: boolean; spiceLine?: string }) {
  // no SpiceLine attribute: the symbol's default applies (as in the netlister)
  const attrs = given.SpiceLine === undefined && spiceLine ? { ...given, SpiceLine: spiceLine } : given;
  const y = current ? "A" : "V";
  const [src, setSrc] = useState<Source>(() => parseSource(attrs.Value, attrs.Value2, attrs.SpiceLine));
  // what the form cannot read (e.g. `PWL file=…`, `SINE (…)`) opens as text instead of being rewritten
  const [text, setText] = useState(() => !sourceRoundTrips(attrs.Value, attrs.Value2, attrs.SpiceLine));
  const [hadRest] = useState(() => !!src.rest); // keep the "Other" fields while they are being retyped
  const [hadVrest] = useState(() => !!src.vrest);
  // arguments per function, so switching away and back keeps what was typed
  const [kept, setKept] = useState<Partial<Record<Func, string[]>>>({});
  const update = (next: Source) => {
    setSrc(next);
    setAttrs({ ...attrs, ...formatSource(next) });
  };
  // no holes: a sparse array would reach formatSource as undefined
  const setArg = (i: number, v: string) => { const args = Array.from({ length: Math.max(src.args.length, i + 1) }, (_, k) => src.args[k] ?? ""); args[i] = v; update({ ...src, args }); };
  const pick = (func: Func | "") => {
    if (func === src.func) return; // the selected one: keep what was typed
    setMore(null);
    // a new function starts with every value written out: the line, the fields and the preview agree
    const args = func ? kept[func] ?? (func === "PWL" ? ["0", "0", "1m", "1"] : FUNCS[func].map((d) => d.def)) : [];
    if (src.func) setKept({ ...kept, [src.func]: src.args });
    update({ ...src, func, args });
  };
  const out = formatSource(src);
  const lastArg = lastFilled(src.args); // empty fields after it: SPICE's defaults
  // "More arguments" keeps its raw text while typed (re-tokenizing each key would eat the spaces)
  const [more, setMore] = useState<string | null>(null);
  const nDefs = src.func && src.func !== "PWL" ? FUNCS[src.func].length : 0;

  if (text) {
    return (
      <>
        <TextField label="Value" value={attrs.Value ?? ""} set={(v) => setAttrs({ ...attrs, Value: v })} autofocus hint="SPICE syntax: 5 · SINE(off amp freq) · PULSE(v1 v2 td tr tf ton per)" />
        <TextField label="Value2" value={attrs.Value2 ?? ""} set={(v) => setAttrs({ ...attrs, Value2: v })} hint="e.g. AC 1" />
        <TextField label="SpiceLine" value={attrs.SpiceLine ?? ""} set={(v) => setAttrs({ ...attrs, SpiceLine: v })} hint="e.g. Rser=50" />
        {sourceRoundTrips(attrs.Value, attrs.Value2, attrs.SpiceLine)
          ? <Button type="button" size="sm" className="self-start" onClick={() => { setSrc(parseSource(attrs.Value, attrs.Value2, attrs.SpiceLine)); setMore(null); setText(false); }}>Back to the form</Button>
          : <p className="text-2xs text-text-tertiary">The form cannot show this source without changing it, so it stays text.</p>}
      </>
    );
  }
  return (
    <>
      <Tabs tabs={KINDS} value={src.func} set={pick} />
      <div className="grid grid-cols-[1fr_300px] gap-4">
        <div className="flex flex-col gap-2">
          {src.func === "PWL" ? <PwlTable args={src.args} set={(args) => update({ ...src, args })} y={y} />
            : src.func ? FUNCS[src.func].map((a, i) => (
              <NumField key={`${src.func}${i}`} label={a.label} value={src.args[i] ?? ""} set={(v) => setArg(i, v)} unit={a.unit === "y" ? y : a.unit}
                placeholder={i < lastArg || i < 2 ? a.def : "SPICE default"} autofocus={i === 0} />
            )) : null}
          {nDefs > 0 && (src.args.length > nDefs || more !== null) && (
            <TextField label="More arguments" value={more ?? src.args.slice(nDefs).join(" ")} hint="kept as written (e.g. a cycle count)"
              set={(v) => { setMore(v); update({ ...src, args: [...src.args.slice(0, nDefs), ...tokens(v)] }); }} />
          )}
          <NumField label={src.func ? "DC value" : "Value"} value={src.dc} set={(dc) => update({ ...src, dc })} unit={y} autofocus={!src.func}
            hint={src.func ? "only for the operating point (.op, .dc)" : undefined} />
          <Section>Small signal (.ac, .noise)</Section>
          <NumField label="AC amplitude" value={src.acMag} set={(acMag) => update({ ...src, acMag })} unit={y} hint={src.acMag ? undefined : "empty: no AC excitation"} />
          {src.acMag && <NumField label="AC phase" value={src.acPhase} set={(acPhase) => update({ ...src, acPhase })} unit="°" placeholder="0" />}
          {!current && (
            <>
              <Section>Parasitics</Section>
              <NumField label="Series resistance" value={src.rser} set={(rser) => update({ ...src, rser })} unit="Ω" />
              <NumField label="Parallel capacitance" value={src.cpar} set={(cpar) => update({ ...src, cpar })} unit="F" />
            </>
          )}
          {(src.vrest || hadVrest) && <TextField label="Value options" value={src.vrest} set={(vrest) => update({ ...src, vrest })} hint="kept after the function, as written" />}
          {(src.rest || hadRest) && <TextField label="Other" value={src.rest} set={(rest) => update({ ...src, rest })} hint="kept as written" />}
        </div>
        <div className="flex flex-col gap-2">
          <Preview src={src} y={y} />
          <SpiceLine>{[out.Value, out.Value2, out.SpiceLine].filter(Boolean).join("\n")}</SpiceLine>
          <Button type="button" size="sm" className="self-start" onClick={() => setText(true)}>Edit as text</Button>
        </div>
      </div>
    </>
  );
}

/** PWL corner points: time, value rows (the args list flattened). */
function PwlTable({ args, set, y }: { args: string[]; set: (a: string[]) => void; y: string }) {
  const rows = Array.from({ length: Math.max(1, Math.ceil(args.length / 2)) }, (_, i) => [args[2 * i] ?? "", args[2 * i + 1] ?? ""]);
  return (
    <Field label="Points" hint="time, value: linear in between, the last value holds">
      <Rows rows={rows} set={(r) => set(r.flat())} a="time" b="value" sep="" units={["s", y]} add="Add point" />
    </Field>
  );
}

/** The transient waveform, scaled into a small box with min/max and end-time labels. */
function Preview({ src, y }: { src: Source; y: string }) {
  const pts = useMemo(() => sample(src), [src]);
  const W = 300, H = 130, P = 6, PY = 18; // PY: room for the value labels above and below
  if (!pts) return <div className="flex h-[130px] items-center justify-center rounded-control border border-border text-center text-2xs text-text-tertiary">Preview needs plain numbers<br />(parameters are evaluated in the simulation)</div>;
  const ys = pts.map(([, v]) => v), lo = Math.min(...ys), hi = Math.max(...ys), span = hi - lo || Math.abs(hi) || 1;
  const t1 = pts[pts.length - 1][0] || 1;
  const sx = (t: number) => P + ((W - 2 * P) * t) / t1, sy = (v: number) => H - PY - ((H - 2 * PY) * (v - lo + (hi === lo ? span / 2 : 0))) / span;
  return (
    <div className="relative rounded-control border border-border bg-surface-schematic-canvas">
      <svg width={W} height={H} className="block">
        <line x1={P} x2={W - P} y1={sy(0)} y2={sy(0)} stroke="var(--border)" strokeDasharray="3 3" />
        <polyline fill="none" stroke="var(--series-1)" strokeWidth={1.6} points={pts.map(([t, v]) => `${sx(t)},${sy(v)}`).join(" ")} />
      </svg>
      <span className="absolute left-1 top-0.5 font-mono text-2xs text-text-tertiary">{si(hi, y, 3)}</span>
      <span className="absolute bottom-0.5 left-1 font-mono text-2xs text-text-tertiary">{si(lo, y, 3)}</span>
      <span className="absolute bottom-0.5 right-1 font-mono text-2xs text-text-tertiary">{src.func ? si(t1, "s", 3) : "DC"}</span>
    </div>
  );
}
