// "Edit Simulation Command": every analysis as a form (LTspice's dialog, plus .tf, .pz, .sens).
import { useState } from "react";
import type { Item } from "./types";
import { commit, useS } from "./store";
import { ANALYSIS, formatAnalysis, parseAnalysis, type Analysis, type Fields } from "./spice";
import { CircuitLists, closeDialog, Field, Modal, NumField, Section, Select, SpiceLine, TextField } from "./ui/dialog";
import { Button } from "./ui/button";
import { cn } from "@/lib/utils";

const TABS: [Analysis, string][] = [
  ["tran", "Transient"], ["ac", "AC"], ["dc", "DC sweep"], ["noise", "Noise"], ["op", "DC op pnt"],
  ["tf", "Transfer fn"], ["pz", "Poles/zeros"], ["sens", "Sensitivity"],
];
const SWEEPS: [string, string][] = [["dec", "Decade (log)"], ["oct", "Octave (log)"], ["lin", "Linear"]];
const PER: Record<string, string> = { dec: "Points per decade", oct: "Points per octave", lin: "Points (total)" };

/** Edits the analysis line of directive `index` (or the first analysis directive), else places a new one. */
export function SimDialog({ index }: { index?: number }) {
  const items = useS((s) => s.items);
  const isAnalysis = (it?: Item) => it?.t === "text" && it.directive && it.text.split("\n").some((l) => ANALYSIS.test(l.trim()));
  const idx = index !== undefined && isAnalysis(items[index]) ? index : items.findIndex((it) => isAnalysis(it));
  const lines = idx >= 0 && items[idx].t === "text" ? items[idx].text.split("\n") : [];
  const li = lines.findIndex((l) => ANALYSIS.test(l.trim()));
  const [init] = useState(() => parseAnalysis(li >= 0 ? lines[li] : ".tran 1m"));
  const [kind, setKind] = useState<Analysis>(init.kind);
  const [forms, setForms] = useState<Partial<Record<Analysis, Fields>>>({ [init.kind]: init.f });
  const f = forms[kind] ?? {};
  const v = (k: string) => f[k] ?? "";
  const set = (k: string) => (val: string) => setForms({ ...forms, [kind]: { ...f, [k]: val } });
  const line = formatAnalysis(kind, f);

  const submit = () => {
    if (idx >= 0) {
      const it = items[idx] as Extract<Item, { t: "text" }>;
      commit(items.map((x, i) => (i === idx ? { ...it, text: lines.map((l, j) => (j === li ? line : l)).join("\n") } : x)));
      closeDialog();
    } else useS.setState({ dialog: null, tool: { k: "place", item: { t: "text", x: 0, y: 0, text: line, directive: true } } });
  };

  const sweep = (pts: string) => (
    <>
      <Select label="Sweep" value={v("sweep") || "dec"} set={set("sweep")} options={SWEEPS} />
      <NumField label={PER[v("sweep") || "dec"]} value={v("pts")} set={set("pts")} placeholder={pts} />
      <NumField label="Start frequency" value={v("fstart")} set={set("fstart")} unit="Hz" placeholder="10" />
      <NumField label="Stop frequency" value={v("fstop")} set={set("fstop")} unit="Hz" placeholder="1Meg" />
    </>
  );
  /** Output variable: a node voltage (against ground or a reference node) or the current through a source. */
  const output = (voltageOnly = false) => (
    <>
      {!voltageOnly && <Select label="Output" value={v("okind") || "v"} set={set("okind")} options={[["v", "Voltage V(node)"], ["i", "Current through a source I(V…)"]]} />}
      {v("okind") === "i" && !voltageOnly ? <TextField label="Source" value={v("isrc")} set={set("isrc")} list="sources" placeholder="V1" autofocus />
        : (
          <>
            <TextField label="Output node" value={v("node")} set={set("node")} list="nets" placeholder="out" autofocus />
            <TextField label="Reference node" value={v("ref")} set={set("ref")} list="nets" placeholder="0" hint="empty: ground" />
          </>
        )}
    </>
  );
  const unitOf = (src: string) => (/^i/i.test(src) ? "A" : /^r/i.test(src) ? "Ω" : /^temp$/i.test(src) ? "°C" : "V");

  return (
    <Modal title="Edit Simulation Command" onSubmit={submit} wide>
      <CircuitLists />
      <div role="tablist" className="flex flex-wrap border-b border-border">
        {TABS.map(([k, label]) => (
          <button key={k} type="button" role="tab" aria-selected={kind === k} onClick={() => setKind(k)}
            className={cn("-mb-px border-b-2 px-2.5 py-1 text-xs", kind === k ? "border-selection text-text-strong" : "border-transparent text-text-secondary hover:text-text-strong")}>
            {label}
          </button>
        ))}
      </div>
      {kind === "tran" && (
        <>
          <NumField label="Stop time" value={v("stop")} set={set("stop")} unit="s" placeholder="1m" autofocus />
          <NumField label="Time step" value={v("step")} set={set("step")} unit="s" hint="optional, default stop/1000" />
          <NumField label="Start saving at" value={v("start")} set={set("start")} unit="s" placeholder="0" />
          <NumField label="Max time step" value={v("max")} set={set("max")} unit="s" hint="optional: limits the solver's step" />
          <Field label="">
            <span className="flex items-center gap-2">
              <input type="checkbox" checked={!!v("uic")} onChange={(e) => set("uic")(e.target.checked ? "1" : "")} />
              Skip the operating point, start from .ic values (uic)
            </span>
          </Field>
        </>
      )}
      {kind === "ac" && sweep("100")}
      {kind === "dc" && (
        <>
          <TextField label="Sweep source" value={v("src")} set={set("src")} list="sweepable" placeholder="V1" autofocus hint="a voltage/current source, a resistor or TEMP" />
          <NumField label="Start" value={v("start")} set={set("start")} unit={unitOf(v("src") || "V")} placeholder="0" />
          <NumField label="Stop" value={v("stop")} set={set("stop")} unit={unitOf(v("src") || "V")} placeholder="5" />
          <NumField label="Increment" value={v("incr")} set={set("incr")} unit={unitOf(v("src") || "V")} placeholder="10m" />
          <Section>Second sweep (optional): one curve per value</Section>
          <TextField label="Source" value={v("src2")} set={set("src2")} list="sweepable" hint="empty: single sweep" />
          {v("src2") && (
            <>
              <NumField label="Start" value={v("start2")} set={set("start2")} unit={unitOf(v("src2"))} placeholder="0" />
              <NumField label="Stop" value={v("stop2")} set={set("stop2")} unit={unitOf(v("src2"))} placeholder="5" />
              <NumField label="Increment" value={v("incr2")} set={set("incr2")} unit={unitOf(v("src2"))} placeholder="1" />
            </>
          )}
        </>
      )}
      {kind === "noise" && (
        <>
          {output(true)}
          <TextField label="Input source" value={v("src")} set={set("src")} list="sources" placeholder="V1" hint="noise is also referred to this input" />
          {sweep("10")}
        </>
      )}
      {kind === "op" && <p className="text-xs text-text-secondary">Find the DC operating point: node voltages and branch currents appear under Results; hover nets to read them.</p>}
      {kind === "tf" && (
        <>
          <p className="text-xs text-text-secondary">Small-signal DC gain from an input source to an output, plus input and output resistance.</p>
          {output()}
          <TextField label="Input source" value={v("src")} set={set("src")} list="sources" placeholder="V1" />
        </>
      )}
      {kind === "pz" && (
        <>
          <p className="text-xs text-text-secondary">Poles and zeros of the transfer function between an input and an output port (node pairs; 0 is ground).</p>
          <TextField label="Input node +" value={v("inp")} set={set("inp")} list="nets" placeholder="in" autofocus />
          <TextField label="Input node −" value={v("inn")} set={set("inn")} list="nets" placeholder="0" />
          <TextField label="Output node +" value={v("outp")} set={set("outp")} list="nets" placeholder="out" />
          <TextField label="Output node −" value={v("outn")} set={set("outn")} list="nets" placeholder="0" />
          <Select label="Transfer function" value={v("tf") || "vol"} set={set("tf")} options={[["vol", "Voltage gain V(out)/V(in)"], ["cur", "Transimpedance V(out)/I(in)"]]} />
          <Select label="Compute" value={v("what") || "pz"} set={set("what")} options={[["pz", "Poles and zeros"], ["pol", "Poles only"], ["zer", "Zeros only"]]} />
        </>
      )}
      {kind === "sens" && (
        <>
          <p className="text-xs text-text-secondary">How strongly the output depends on every component value and model parameter.</p>
          {output()}
          <Select label="Analysis" value={v("mode") || "dc"} set={set("mode")} options={[["dc", "DC (operating point)"], ["ac", "AC (over frequency)"]]} />
          {v("mode") === "ac" && sweep("10")}
        </>
      )}
      {v("rest") && <TextField label="Other options" value={v("rest")} set={set("rest")} hint="kept as written" />}
      <SpiceLine>{line}</SpiceLine>
      {idx >= 0 && (
        <Button type="button" size="sm" className="self-start" onClick={() => useS.setState({ dialog: { k: "edit", index: idx, raw: true } })}>Edit as text</Button>
      )}
    </Modal>
  );
}
