// SPICE directives as forms: .param, .model, .meas, .ic, .temp, .include; anything else as text.
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { Item } from "./types";
import { dirOf, useS } from "./store";
import {
  directiveKind, formatIc, formatInclude, formatMeas, formatModel, formatParams, MODEL_PARAMS, MODEL_TYPES, parseIc, parseInclude,
  numInfo, parseMeas, parseModel, parseParams, portablePaths, type DirectiveKind, type Meas,
} from "./spice";
import { CircuitLists, Field, FIELD, Modal, NumField, Section, Select, SpiceLine, TextField } from "./ui/dialog";
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
  const info = numInfo(value, unit);
  return (
    <label className="grid grid-cols-[64px_1fr_150px_36px_64px] items-center gap-x-2 gap-y-0.5 text-xs">
      <span className="font-mono text-text-strong">{name}</span>
      <span className="truncate text-text-secondary" title={desc}>{desc}</span>
      <input className={FIELD} value={value} spellCheck={false} onChange={(e) => set(e.target.value)} />
      <span className="text-2xs text-text-tertiary">{unit}</span>
      <span className="truncate font-mono text-2xs text-text-tertiary">{info && /[a-zµμ{']/i.test(value) ? info.text : ""}</span>
      {info?.warn && <span className="col-span-2 col-start-3 text-2xs text-status-warning">⚠ {info.warn}</span>}
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

/** New or edited directive; `onDone` gets the item (commit in place, or start placing it). */
export function DirectiveDialog({ item, title, raw, onDone }: { item: Text; title: string; raw?: boolean; onDone: (it: Item) => void }) {
  const one = item.text.trim();
  const [tab, setTab] = useState<DirectiveKind>(raw ? "text" : directiveKind(item.text));
  const [text, setText] = useState(item.text);
  const [directive, setDirective] = useState(true); // the Text tab can turn it into a comment
  const [initial] = useState(tab);
  const [touched, setTouched] = useState<Partial<Record<DirectiveKind, boolean>>>({});
  const [tried, setTried] = useState(false);
  const [params, setParams] = useState(() => (tab === "param" ? parseParams(one) : [["", ""]]));
  const [model] = useState(() => (tab === "model" ? parseModel(one) : null));
  const [mName, setMName] = useState(model?.name ?? "");
  const [mType, setMType] = useState(model?.type ?? "D");
  const [mParams, setMParams] = useState<string[][]>(model?.params ?? []);
  const [meas, setMeas] = useState<Meas>(() => (tab === "meas" && parseMeas(one)) || { an: "tran", name: "", type: "stat", f: { op: "AVG" } });
  const [ic, setIc] = useState(() => (tab === "ic" ? parseIc(one) : [["", ""]]));
  const [temp, setTemp] = useState(() => (tab === "temp" ? one.split(/\s+/)[1] ?? "" : ""));
  const [inc, setInc] = useState(() => (tab === "include" ? parseInclude(one) : { lib: false, path: "", section: "" }));

  const mp = (k: string) => mParams.find(([n]) => n.toUpperCase() === k)?.[1] ?? "";
  const setMp = (k: string, v: string) => setMParams(mParams.some(([n]) => n.toUpperCase() === k) ? mParams.map((p) => (p[0].toUpperCase() === k ? [p[0], v] : p)) : [...mParams, [k, v]]);
  const known = MODEL_PARAMS[mType] ?? [];
  const extra = mParams.filter(([n]) => !known.some(([k]) => k === n.toUpperCase()));
  const mf = (k: string) => meas.f[k] ?? "";
  const setMf = (k: string) => (v: string) => setMeas({ ...meas, f: { ...meas.f, [k]: v } });

  const line = {
    text: () => text,
    param: () => formatParams(params),
    model: () => formatModel(mName, mType, mParams),
    meas: () => formatMeas(meas),
    ic: () => formatIc(ic),
    temp: () => `.temp ${temp.trim() || "27"}`,
    include: () => formatInclude(inc.lib, inc.path, inc.section),
  }[tab]();

  const browse = async () => {
    const path = await open({ multiple: false, filters: [{ name: "SPICE models", extensions: ["lib", "sub", "mod", "inc", "cir", "txt", "sp", "spi", "bjt", "dio", "jft", "mos"] }] });
    if (typeof path !== "string") return;
    const dir = dirOf(useS.getState().path); // relative to the schematic when inside its folder
    const rel = dir && (path.startsWith(`${dir}/`) || path.startsWith(`${dir}\\`)) ? path.slice(dir.length + 1) : path;
    setInc({ ...inc, path: rel.replaceAll("\\", "/") });
  };
  // a form replaces a directive of its own kind; any other text stays and gets the line added
  const keeps = tab !== "text" && tab !== initial && text.trim() !== "";
  const result = tab === "text" ? text : keeps ? `${text.trimEnd()}\n${line}` : line;
  const invalid = tab === "param" && !params.some(([n]) => n.trim()) ? "Add a parameter name."
    : tab === "ic" && !ic.some(([n]) => n.trim()) ? "Add a node."
    : tab === "include" && !inc.path.trim() ? "Choose a file." : "";
  const done = () => {
    if (invalid) return setTried(true);
    if (!result.trim()) return useS.setState({ dialog: null });
    onDone({ ...item, text: portablePaths(result), directive: tab !== "text" || directive });
  };
  // to the Text tab: what the form built, if it was used (else the text stays as it was)
  const toTab = (k: DirectiveKind) => { if (k === "text" && tab !== "text" && touched[tab]) setText(result); setTab(k); setTried(false); };

  return (
    <Modal title={title} onSubmit={done} wide>
      <CircuitLists />
      <div role="tablist" className="flex flex-wrap border-b border-border">
        {TABS.map(([k, label]) => (
          <button key={k} type="button" role="tab" aria-selected={tab === k} onClick={() => toTab(k)}
            className={cn("-mb-px border-b-2 px-2.5 py-1 text-xs", tab === k ? "border-selection text-text-strong" : "border-transparent text-text-secondary hover:text-text-strong")}>
            {label}
          </button>
        ))}
      </div>
      {tab === "text" && (
        <>
          <textarea data-autofocus rows={6} className={cn(FIELD_BASE, FIELD_FOCUS, "w-full p-2 font-mono")} value={text} spellCheck={false}
            placeholder={".tran 1m\n.param R=1k\n.model D1N4148 D(...)"} onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && (e.preventDefault(), done())} />
          <label className="flex items-center gap-2 text-xs text-text-secondary">
            <input type="checkbox" checked={directive} onChange={(e) => setDirective(e.target.checked)} />
            SPICE directive (otherwise a comment, ignored by the simulation)
          </label>
          <p className="text-2xs text-text-tertiary">Any SPICE directive, one per line · {keys("⌘↵")} to confirm · the other tabs build the common ones for you</p>
          <Button type="button" size="sm" className="self-start" onClick={() => useS.setState({ dialog: { k: "sim" } })}>Simulation command (.tran, .ac …)</Button>
        </>
      )}
      <div className="contents" onInputCapture={() => setTouched({ ...touched, [tab]: true })} onClickCapture={(e) => (e.target as HTMLElement).closest("button") && setTouched({ ...touched, [tab]: true })}>
      {tab === "param" && (
        <Field label="Parameters" hint="use them in values as {name}, e.g. {R*2}">
          <Rows rows={params} set={setParams} a="name" b="value" />
        </Field>
      )}
      {tab === "model" && (
        <>
          <TextField label="Model name" value={mName} set={setMName} placeholder="MyDiode" autofocus hint="the Value of the parts that use it" />
          <Select label="Device type" value={mType} set={setMType} options={MODEL_TYPES.map((t) => [t, { D: "D – diode", NPN: "NPN – bipolar", PNP: "PNP – bipolar", NMOS: "NMOS – MOSFET", PMOS: "PMOS – MOSFET", NJF: "NJF – JFET", PJF: "PJF – JFET", SW: "SW – voltage-controlled switch", CSW: "CSW – current-controlled switch" }[t] ?? t])} />
          <Section>Parameters (empty: SPICE default)</Section>
          <div className="flex flex-col gap-1">
            {known.map(([k, desc, unit]) => <ParamRow key={k} name={k} desc={desc} unit={unit} value={mp(k)} set={(v) => setMp(k, v)} />)}
          </div>
          {extra.length > 0 && <Section>Other parameters</Section>}
          {extra.length > 0 && <Rows rows={extra} set={(rows) => setMParams([...mParams.filter(([n]) => known.some(([k]) => k === n.toUpperCase())), ...rows])} a="name" b="value" />}
          {!extra.length && <Button type="button" size="sm" className="self-start" onClick={() => setMParams([...mParams, ["", ""]])}>Add another parameter</Button>}
        </>
      )}
      {tab === "meas" && (
        <>
          <p className="text-xs text-text-secondary">Measures a number after the run; the result is listed in the Log.</p>
          <Select label="Analysis" value={meas.an} set={(an) => setMeas({ ...meas, an })} options={[["tran", "Transient"], ["ac", "AC"], ["dc", "DC sweep"]]} />
          <TextField label="Name" value={meas.name} set={(name) => setMeas({ ...meas, name: name.replace(/\s/g, "_") })} placeholder="result" autofocus />
          <Select label="Measure" value={meas.type} set={(type) => setMeas({ ...meas, type: type as Meas["type"] })} options={MEAS_TYPES} />
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
              {mf("edge") && <NumField label="Number" value={mf("n")} set={setMf("n")} placeholder="1" hint="1 = the first one" />}
            </>
          )}
          {meas.type === "delay" && (
            <>
              <Section>From (trigger)</Section>
              <TextField label="Signal" value={mf("expr")} set={setMf("expr")} list="vexprs" placeholder="V(in)" />
              <NumField label="Reaches" value={mf("val")} set={setMf("val")} placeholder="0.5" />
              <Select label="Crossing" value={mf("edge")} set={setMf("edge")} options={EDGES} />
              {mf("edge") && <NumField label="Number" value={mf("n")} set={setMf("n")} placeholder="1" />}
              <Section>To (target)</Section>
              <TextField label="Signal" value={mf("expr2")} set={setMf("expr2")} list="vexprs" placeholder="V(out)" />
              <NumField label="Reaches" value={mf("val2")} set={setMf("val2")} placeholder="0.5" />
              <Select label="Crossing" value={mf("edge2")} set={setMf("edge2")} options={EDGES} />
              {mf("edge2") && <NumField label="Number" value={mf("n2")} set={setMf("n2")} placeholder="1" />}
            </>
          )}
        </>
      )}
      {tab === "ic" && (
        <Field label="Node voltages" hint="the transient starts from these (with uic: without an operating point)">
          <Rows rows={ic} set={setIc} a="node" b="voltage" list="nets" unit="V" />
        </Field>
      )}
      {tab === "temp" && <NumField label="Temperature" value={temp} set={setTemp} unit="°C" placeholder="27" autofocus hint="circuit temperature for all models" />}
      {tab === "include" && (
        <>
          <Field label="File">
            <span className="flex gap-1">
              <input className={FIELD} value={inc.path} placeholder="models/mymodels.lib" data-autofocus onChange={(e) => setInc({ ...inc, path: e.target.value })} />
              <Button type="button" onClick={browse}>Browse…</Button>
            </span>
          </Field>
          <Select label="Load as" value={inc.lib ? "lib" : "include"} set={(k) => setInc({ ...inc, lib: k === "lib" })} options={[["include", ".include – the whole file"], ["lib", ".lib – a library (optionally one section)"]]} />
          {inc.lib && <TextField label="Section" value={inc.section} set={(section) => setInc({ ...inc, section })} hint="empty: the whole file" />}
        </>
      )}
      </div>
      {tab !== "text" && <SpiceLine>{line}</SpiceLine>}
      {keeps && <p className="text-2xs text-text-tertiary">Added as a new line; the directive keeps {text.split("\n").filter((l) => l.trim()).length} line(s): {text.trim().split("\n")[0]}{text.trim().includes("\n") ? " …" : ""}</p>}
      {tried && invalid && <p className="text-2xs text-status-danger">{invalid}</p>}
    </Modal>
  );
}
