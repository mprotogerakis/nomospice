// SPICE directives as forms: .param, .model, .meas, .ic, .temp, .include; anything else as text.
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { Item } from "./types";
import { relToDoc, useS } from "./store";
import {
  directiveKind, formatIc, formatInclude, formatMeas, formatModel, formatParams, MODEL_PARAMS, MODEL_TYPES, parseIc, parseInclude,
  analysisLine, analysisRoundTrips, parseMeas, parseModel, parseParams, portablePaths, type DirectiveKind, type Meas,
} from "./spice";
import { CircuitLists, Tabs, Field, FIELD, Modal, numHelp, NumField, Section, Select, SpiceLine, TextField } from "./ui/dialog";
import { Button } from "./ui/button";
import { FIELD_BASE, FIELD_FOCUS } from "./ui/focus";
import { cn, keys } from "@/lib/utils";

type Text = Extract<Item, { t: "text" }>;
const TABS: [DirectiveKind, string][] = [
  ["text", "Text"], ["param", "Parameters"], ["model", "Model"], ["meas", "Measurement"], ["ic", "Initial conditions"], ["temp", "Temperature"], ["include", "Include file"],
];
const MEAS_TYPES: [Meas["type"], string][] = [
  ["stat", "Statistic over an interval (average, RMS, min, max …)"], ["find", "Value at a point (FIND … AT)"],
  ["when", "Where a signal reaches a value (WHEN)"], ["delay", "Delay between two crossings (TRIG/TARG)"],
];
const EDGES: [string, string][] = [["", "the first one"], ["rise", "rising"], ["fall", "falling"], ["cross", "either direction"]];

/** One model parameter: name, meaning, value, unit and how SPICE reads it, in one row. */
function ParamRow({ name, desc, unit, value, set }: { name: string; desc: string; unit: string; value: string; set: (v: string) => void }) {
  const help = numHelp(value, unit, set);
  return (
    <label className="grid grid-cols-[64px_1fr_150px_36px_64px] items-center gap-x-2 gap-y-0.5 text-xs">
      <span className="font-mono text-text-strong">{name}</span>
      <span className="truncate text-text-secondary" title={desc}>{desc}</span>
      <input className={FIELD} value={value} spellCheck={false} onChange={(e) => set(e.target.value)} />
      <span className="text-2xs text-text-tertiary">{unit}</span>
      <span className="truncate font-mono text-2xs text-text-tertiary">{help.info}</span>
      {help.warn && <span className="col-span-3 col-start-3 text-2xs text-status-warning">⚠ {help.warn}</span>}
    </label>
  );
}

/** Rows of two text cells with add/remove (parameters, initial conditions). */
function Rows({ rows, set, a, b, list, unit }: { rows: string[][]; set: (r: string[][]) => void; a: string; b: string; list?: string; unit?: string }) {
  const put = (r: number, c: number, v: string) => set(rows.map((row, i) => (i === r ? (c ? [row[0], v] : [v, row[1]]) : row)));
  return (
    <div className="flex flex-col gap-1">
      {rows.map(([x, y], r) => (
        <div key={r} className="flex items-center gap-1">
          <input className={FIELD} value={x} placeholder={a} list={list} data-autofocus={r === 0 || undefined} aria-label={`${a} ${r + 1}`} onChange={(e) => put(r, 0, e.target.value)} />
          <span className="text-text-tertiary">=</span>
          <input className={FIELD} value={y} placeholder={b} aria-label={`${b} ${r + 1}`} onChange={(e) => put(r, 1, e.target.value)} />
          {unit && <span className="w-3 text-2xs text-text-tertiary">{unit}</span>}
          <button type="button" aria-label="Remove row" className="px-1 text-text-tertiary hover:text-status-danger" onClick={() => set(rows.filter((_, i) => i !== r))}>×</button>
        </div>
      ))}
      <Button type="button" size="sm" className="self-start" onClick={() => set([...rows, ["", ""]])}>Add</Button>
    </div>
  );
}

/** What form `k` shows for `txt`: the directive's values if `txt` is exactly one of its kind, else empty. */
function formState(k: DirectiveKind, txt: string) {
  const one = directiveKind(txt) === k ? txt.trim() : "";
  const m = k === "model" && one ? parseModel(one) : null;
  const known = (n: string) => (MODEL_PARAMS[m?.type ?? "D"] ?? []).some(([p]) => p === n.toUpperCase());
  return {
    params: k === "param" && one ? parseParams(one) : [["", ""]],
    // listed parameters by name; others as rows that are never re-sorted while their name is typed
    mName: m?.name ?? "", mType: m?.type ?? "D",
    mKnown: Object.fromEntries((m?.params ?? []).filter(([n]) => known(n)).map(([n, v]) => [n.toUpperCase(), v])) as Record<string, string>,
    mExtra: (m?.params ?? []).filter(([n]) => !known(n)),
    meas: (k === "meas" && one && parseMeas(one)) || ({ an: "tran", name: "", type: "stat", f: { op: "AVG" } } as Meas),
    ic: k === "ic" && one ? parseIc(one) : [["", ""]],
    temp: k === "temp" && one ? one.split(/\s+/)[1] ?? "" : "",
    inc: k === "include" && one ? parseInclude(one) : { lib: false, path: "", section: "" },
  };
}

/** New or edited directive; `onDone` gets the item (commit in place, or start placing it). */
export function DirectiveDialog({ item, title, raw, onDone }: { item: Text; title: string; raw?: boolean; onDone: (it: Item) => void }) {
  // The text is the truth. A form, when entered, reads the text if the text is exactly one
  // directive of its kind (then it edits it); otherwise it starts empty and adds its line.
  // Leaving a form that was used writes its result into the text, so no tab loses edits.
  const [tab, setTab] = useState<DirectiveKind>(raw ? "text" : directiveKind(item.text));
  const [text, setText] = useState(item.text);
  const [directive, setDirective] = useState(item.directive); // the Text tab switches directive / comment
  const [tried, setTried] = useState(false);
  const [st, setSt] = useState(() => formState(tab, item.text)); // the first tab reads the directive
  const put = (patch: Partial<typeof st>) => setSt((s) => ({ ...s, ...patch }));
  const { params, mName, mType, mKnown, mExtra, meas, ic, temp, inc } = st;
  const lineOf = (k: DirectiveKind, s: typeof st) => ({
    text: () => text,
    param: () => formatParams(s.params),
    model: () => formatModel(s.mName, s.mType, [...Object.entries(s.mKnown), ...s.mExtra]),
    meas: () => formatMeas(s.meas),
    ic: () => formatIc(s.ic),
    temp: () => `.temp ${s.temp.trim() || "27"}`,
    include: () => formatInclude(s.inc.lib, s.inc.path, s.inc.section),
  })[k]();
  const line = lineOf(tab, st);
  // "changed": the form writes something else than it read (a click alone, or Browse… cancelled, is no change)
  const [base, setBase] = useState(line);
  const touched = tab !== "text" && line !== base;
  const load = (k: DirectiveKind, txt: string) => { const s = formState(k, txt); setSt(s); setBase(lineOf(k, s)); };

  const known = MODEL_PARAMS[mType] ?? [];
  // another device type: its listed parameters move into the table, the rest become extra rows
  const retype = (t: string) => {
    const all = [...Object.entries(mKnown), ...mExtra].filter(([n, v]) => n.trim() || v.trim());
    const kn = (n: string) => (MODEL_PARAMS[t] ?? []).some(([p]) => p === n.toUpperCase());
    put({ mType: t, mKnown: Object.fromEntries(all.filter(([n]) => kn(n)).map(([n, v]) => [n.toUpperCase(), v])), mExtra: all.filter(([n]) => !kn(n)) });
  };
  const mf = (k: string) => meas.f[k] ?? "";
  const setMf = (k: string) => (v: string) => put({ meas: { ...meas, f: { ...meas.f, [k]: v } } });

  const browse = async () => {
    const path = await open({ multiple: false, filters: [{ name: "SPICE models", extensions: ["lib", "sub", "mod", "inc", "cir", "txt", "sp", "spi", "bjt", "dio", "jft", "mos"] }] });
    if (typeof path !== "string") return;
    put({ inc: { ...inc, path: relToDoc(path) } });
  };
  // a form replaces a directive of its own kind; any other text stays and gets the line added
  const keeps = tab !== "text" && directiveKind(text) !== tab && text.trim() !== "";
  const result = tab === "text" || (!touched && keeps) ? text : keeps ? `${text.trimEnd()}\n${line}` : line;
  const invalid = !touched && keeps ? "" : tab === "param" && !params.some(([n]) => n.trim()) ? "Add a parameter name."
    : tab === "ic" && !ic.some(([n]) => n.trim()) ? "Add a node."
    : tab === "include" && !inc.path.trim() ? "Choose a file." : "";
  const done = () => {
    if (invalid) return setTried(true);
    if (!result.trim()) return useS.setState({ dialog: null });
    // the checkbox decides whenever the result is the text itself (Text tab, or an untouched form)
    const isDirective = result === text ? directive : true;
    onDone({ ...item, text: isDirective ? portablePaths(result) : result, directive: isDirective }); // a comment stays as written
  };
  // leaving a used form writes its result into the text; the next form reads the text
  const toTab = (k: DirectiveKind) => {
    if (k === tab) return;
    if (touched && invalid) return setTried(true); // a half-filled form: say what is missing (or discard it)
    const next = touched ? result : text;
    setText(next);
    if (k !== "text") load(k, next);
    setTab(k);
    setTried(false);
  };
  // an analysis the simulation form cannot read stays here (the button would only come back)
  const analysis = analysisLine(text);
  const simForm = !analysis || analysisRoundTrips(analysis);

  return (
    <Modal title={title} onSubmit={done} wide>
      <CircuitLists />
      <Tabs tabs={TABS} value={tab} set={toTab} />
      {tab === "text" && (
        <>
          <textarea data-autofocus rows={6} className={cn(FIELD_BASE, FIELD_FOCUS, "w-full p-2 font-mono")} value={text} spellCheck={false}
            placeholder={directive ? ".tran 1m\n.param R=1k\n.model D1N4148 D(...)" : "Comment"} onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && (e.preventDefault(), done())} />
          <label className="flex items-center gap-2 text-xs text-text-secondary">
            <input type="checkbox" checked={directive} onChange={(e) => setDirective(e.target.checked)} />
            SPICE directive (otherwise a comment, ignored by the simulation)
          </label>
          <p className="text-2xs text-text-tertiary">Any SPICE directive, one per line · {keys("⌘↵")} to confirm · the other tabs build the common ones for you</p>
          {!simForm && <p className="text-2xs text-text-tertiary">This analysis uses options the simulation form does not know, so it is edited here as text.</p>}
        </>
      )}
      {tab === "param" && (
        <Field label="Parameters" hint="use them in values as {name}, e.g. {R*2}">
          <Rows rows={params} set={(params) => put({ params })} a="name" b="value" />
        </Field>
      )}
      {tab === "model" && (
        <>
          <TextField label="Model name" value={mName} set={(mName) => put({ mName })} placeholder="MyDiode" autofocus hint="the Value of the parts that use it" />
          <Select label="Device type" value={mType} set={retype} options={MODEL_TYPES.map((t) => [t, { D: "D – diode", NPN: "NPN – bipolar", PNP: "PNP – bipolar", NMOS: "NMOS – MOSFET", PMOS: "PMOS – MOSFET", NJF: "NJF – JFET", PJF: "PJF – JFET", SW: "SW – voltage-controlled switch", CSW: "CSW – current-controlled switch" }[t] ?? t])} />
          <Section>Parameters (empty: SPICE default)</Section>
          <div className="flex flex-col gap-1">
            {known.map(([k, desc, unit]) => <ParamRow key={k} name={k} desc={desc} unit={unit} value={mKnown[k] ?? ""} set={(v) => put({ mKnown: { ...mKnown, [k]: v } })} />)}
          </div>
          {mExtra.length > 0 && <Section>Other parameters</Section>}
          {mExtra.length > 0 && <Rows rows={mExtra} set={(mExtra) => put({ mExtra })} a="name" b="value" />}
          {!mExtra.length && <Button type="button" size="sm" className="self-start" onClick={() => put({ mExtra: [["", ""]] })}>Add another parameter</Button>}
        </>
      )}
      {tab === "meas" && (
        <>
          <p className="text-xs text-text-secondary">Measures a number after the run; the result is listed in the Log.</p>
          <Select label="Analysis" value={meas.an} set={(an) => put({ meas: { ...meas, an } })} options={[["tran", "Transient"], ["ac", "AC"], ["dc", "DC sweep"]]} />
          <TextField label="Name" value={meas.name} set={(name) => put({ meas: { ...meas, name: name.replace(/\s/g, "_") } })} placeholder="result" autofocus />
          <Select label="Measure" value={meas.type} set={(type) => put({ meas: { ...meas, type: type as Meas["type"] } })} options={MEAS_TYPES} />
          {meas.type === "stat" && (
            <>
              <Select label="Quantity" value={mf("op") || "AVG"} set={setMf("op")} options={[["AVG", "Average"], ["RMS", "RMS"], ["MIN", "Minimum"], ["MAX", "Maximum"], ["PP", "Peak-to-peak"], ["INTEG", "Integral"]]} />
              <TextField label="Signal" value={mf("expr")} set={setMf("expr")} list="vexprs" placeholder="V(out)" />
              <NumField label="From" value={mf("from")} set={setMf("from")} hint="empty: from the start" />
              <NumField label="To" value={mf("to")} set={setMf("to")} hint="empty: to the end" />
            </>
          )}
          {meas.type === "find" && (
            <>
              <TextField label="Signal" value={mf("expr")} set={setMf("expr")} list="vexprs" placeholder="V(out)" />
              <NumField label="At" value={mf("at")} set={setMf("at")} placeholder="1m" hint="time, frequency or sweep value" />
            </>
          )}
          {meas.type === "when" && (
            <>
              <TextField label="Signal" value={mf("expr")} set={setMf("expr")} list="vexprs" placeholder="V(out)" />
              <NumField label="Reaches" value={mf("val")} set={setMf("val")} placeholder="0" />
              <Select label="Crossing" value={mf("edge")} set={setMf("edge")} options={EDGES} />
              {mf("edge") && <TextField label="Number" value={mf("n")} set={setMf("n")} placeholder="1" hint="1 = the first one, LAST = the last" />}
            </>
          )}
          {meas.type === "delay" && (
            <>
              <Section>From (trigger)</Section>
              <TextField label="Signal" value={mf("expr")} set={setMf("expr")} list="vexprs" placeholder="V(in)" />
              <NumField label="Reaches" value={mf("val")} set={setMf("val")} placeholder="0.5" />
              <Select label="Crossing" value={mf("edge")} set={setMf("edge")} options={EDGES} />
              {mf("edge") && <TextField label="Number" value={mf("n")} set={setMf("n")} placeholder="1" hint="or LAST" />}
              <Section>To (target)</Section>
              <TextField label="Signal" value={mf("expr2")} set={setMf("expr2")} list="vexprs" placeholder="V(out)" />
              <NumField label="Reaches" value={mf("val2")} set={setMf("val2")} placeholder="0.5" />
              <Select label="Crossing" value={mf("edge2")} set={setMf("edge2")} options={EDGES} />
              {mf("edge2") && <TextField label="Number" value={mf("n2")} set={setMf("n2")} placeholder="1" hint="or LAST" />}
            </>
          )}
        </>
      )}
      {tab === "ic" && (
        <Field label="Node voltages" hint="the transient starts from these (with uic: without an operating point)">
          <Rows rows={ic} set={(ic) => put({ ic })} a="node" b="voltage" list="nets" unit="V" />
        </Field>
      )}
      {tab === "temp" && <NumField label="Temperature" value={temp} set={(temp) => put({ temp })} unit="°C" placeholder="27" autofocus hint="circuit temperature for all models" />}
      {tab === "include" && (
        <>
          <Field label="File">
            <span className="flex gap-1">
              <input className={FIELD} value={inc.path} placeholder="models/mymodels.lib" data-autofocus onChange={(e) => put({ inc: { ...inc, path: e.target.value } })} />
              <Button type="button" onClick={browse}>Browse…</Button>
            </span>
          </Field>
          <Select label="Load as" value={inc.lib ? "lib" : "include"} set={(k) => put({ inc: { ...inc, lib: k === "lib" } })} options={[["include", ".include – the whole file"], ["lib", ".lib – a library (optionally one section)"]]} />
          {inc.lib && <TextField label="Section" value={inc.section} set={(section) => put({ inc: { ...inc, section } })} hint="empty: the whole file" />}
        </>
      )}
      {tab !== "text" && <SpiceLine>{line}</SpiceLine>}
      {keeps && <p className="text-2xs text-text-tertiary">Added as a new line; the directive keeps {text.split("\n").filter((l) => l.trim()).length} line(s): {text.trim().split("\n")[0]}{text.trim().includes("\n") ? " …" : ""}</p>}
      {tried && invalid && (
        <p className="text-2xs text-status-danger">
          {invalid}
          <button type="button" className="ml-2 underline hover:text-text-strong" onClick={() => { load(tab, text); setTried(false); }}>Discard this form's changes</button>
        </p>
      )}
    </Modal>
  );
}
