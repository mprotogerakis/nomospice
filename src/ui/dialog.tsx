// Dialog building blocks shared by the editors: modal frame, labelled fields, SPICE number fields,
// and suggestion lists (nets, sources) from the open schematic.
import { useEffect, useMemo, useRef } from "react";
import { currentNets, useS } from "../store";
import { defOf, type Lib } from "../geom";
import type { Item, Netlist } from "../types";
import { numInfo } from "../spice";
import { Button } from "./button";
import { FIELD_BASE, FIELD_FOCUS } from "./focus";
import { cn } from "@/lib/utils";

export const closeDialog = () => useS.setState({ dialog: null });
export const FIELD = cn(FIELD_BASE, FIELD_FOCUS, "h-[26px] w-full px-2 font-mono");

export function Modal({ title, children, onSubmit, wide, ok = "OK", cancel = true }: { title: string; children: React.ReactNode; onSubmit: () => void; wide?: boolean | "xl"; ok?: string; cancel?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current!;
    d.showModal();
    // React runs autoFocus while the dialog is still closed (no effect), and showModal() then
    // focuses the first control: focus the intended field now, text selected for overwriting
    const f = d.querySelector<HTMLElement>("[data-autofocus]") ?? d.querySelector<HTMLElement>("button[type=submit]");
    f?.focus();
    if (f instanceof HTMLInputElement) f.select();
  }, []);
  return (
    <dialog
      ref={ref}
      onClose={closeDialog}
      className={cn("m-auto rounded-float border border-menu-border bg-menu-bg p-0 text-text shadow-dialog backdrop:bg-scrim", wide === "xl" ? "w-[860px]" : wide ? "w-[560px]" : "w-[420px]")}
    >
      <form onSubmit={(e) => { e.preventDefault(); onSubmit(); }}>
        <header className="border-b border-border px-3 py-2 text-sm font-medium text-text-strong">{title}</header>
        <div className="flex max-h-[75vh] flex-col gap-2.5 overflow-y-auto p-3">{children}</div>
        <footer className="flex justify-end gap-2 border-t border-border px-3 py-2">
          {cancel && <Button type="button" variant="ghost" onClick={closeDialog}>Cancel</Button>}
          <Button type="submit" variant="primary">{ok}</Button>
        </footer>
      </form>
    </dialog>
  );
}

export function Field({ label, children, hint, warn, compact }: { label: string; children: React.ReactNode; hint?: React.ReactNode; warn?: React.ReactNode; compact?: boolean }) {
  return (
    <label className={cn("grid items-center gap-x-2 gap-y-0.5 text-xs text-text-secondary", compact ? "grid-cols-[70px_1fr]" : "grid-cols-[130px_1fr]")}>
      <span>{label}</span>
      {children}
      {warn ? <span className="col-start-2 text-2xs text-status-warning">⚠ {warn}</span>
        : hint ? <span className="col-start-2 text-2xs text-text-tertiary">{hint}</span> : null}
    </label>
  );
}

/** A text input in a Field with a unit slot and an info slot (how SPICE reads it), aligned across fields. */
export function TextField({ label, value, set, placeholder, list, unit, info, hint, warn, autofocus, compact }: {
  label: string; value: string; set: (v: string) => void; placeholder?: string; list?: string; unit?: string; info?: string; hint?: React.ReactNode; warn?: React.ReactNode; autofocus?: boolean; compact?: boolean;
}) {
  const input = <input className={FIELD} value={value} placeholder={placeholder} list={list} data-autofocus={autofocus || undefined} spellCheck={false} onChange={(e) => set(e.target.value)} />;
  // compact (narrow panels): no side slots, the reading goes below
  if (compact) return <Field label={label} compact hint={[info, hint].filter(Boolean).join(" · ") || undefined} warn={warn}>{input}</Field>;
  return (
    <Field label={label} hint={hint} warn={warn}>
      <span className="flex items-center gap-2">
        {input}
        <span className="w-8 shrink-0 text-2xs text-text-tertiary">{unit}</span>
        <span className="w-[72px] shrink-0 truncate font-mono text-2xs text-text-tertiary" title={info}>{info}</span>
      </span>
    </Field>
  );
}

/** How SPICE reads `value` (only worth showing for suffixes: 1k, 1M ...) and its warning, with a
 *  one-click fix for the milli/mega trap (1MHz -> 1MegHz). */
export function numHelp(value: string, unit: string | undefined, set: (v: string) => void) {
  const info = numInfo(value, unit);
  const mega = /milli/.test(info?.warn ?? "") ? value.replace(/M(?!eg)/i, "Meg") : null;
  return {
    info: info && /[a-zµμ{']/i.test(value) ? info.text : "",
    warn: info?.warn && (
      <>
        {info.warn}
        {mega && <button type="button" className="ml-2 underline hover:text-text-strong" onClick={() => set(mega)}>use {mega}</button>}
      </>
    ),
  };
}

/** A SPICE number field: unit, how SPICE reads it, and the M/F warnings. */
export function NumField(p: { label: string; value: string; set: (v: string) => void; unit?: string; placeholder?: string; hint?: string; list?: string; autofocus?: boolean; compact?: boolean }) {
  return <TextField {...p} unit={p.unit ?? ""} {...numHelp(p.value, p.unit, p.set)} />;
}

export function Select({ label, value, set, options, hint, compact }: { label: string; value: string; set: (v: string) => void; options: [string, string][]; hint?: string; compact?: boolean }) {
  const select = (
    <select className={FIELD} value={value} onChange={(e) => set(e.target.value)}>
      {options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
    </select>
  );
  return (
    <Field label={label} hint={hint} compact={compact}>
      {compact ? select : (
        <span className="flex items-center gap-2">
          {select}
          <span className="w-8 shrink-0" />
          <span className="w-[72px] shrink-0" />
        </span>
      )}
    </Field>
  );
}

/** A row of tabs (simulation command, directive forms). */
export function Tabs<K extends string>({ tabs, value, set }: { tabs: [K, string][]; value: K; set: (k: K) => void }) {
  return (
    <div role="tablist" className="flex flex-wrap border-b border-border">
      {tabs.map(([k, label]) => (
        <button key={k} type="button" role="tab" aria-selected={value === k} onClick={() => set(k)}
          className={cn("-mb-px border-b-2 px-2.5 py-1 text-xs", value === k ? "border-selection text-text-strong" : "border-transparent text-text-secondary hover:text-text-strong")}>
          {label}
        </button>
      ))}
    </div>
  );
}

/** Rows of two text cells with add/remove: parameters (name = value), initial conditions, PWL points.
 *  `units` are shown after each cell; `sep` between them. */
export function Rows({ rows, set, a, b, list, sep = "=", units = ["", ""], add = "Add" }: {
  rows: string[][]; set: (r: string[][]) => void; a: string; b: string; list?: string; sep?: string; units?: [string, string]; add?: string;
}) {
  const put = (r: number, c: number, v: string) => set(rows.map((row, i) => (i === r ? (c ? [row[0], v] : [v, row[1]]) : row)));
  return (
    <div className="flex flex-col gap-1">
      {rows.map(([x, y], r) => (
        <div key={r} className="flex items-center gap-1">
          <input className={FIELD} value={x} placeholder={a} list={list} data-autofocus={r === 0 || undefined} aria-label={`${a} ${r + 1}`} onChange={(e) => put(r, 0, e.target.value)} />
          {units[0] && <span className="text-2xs text-text-tertiary">{units[0]}</span>}
          {sep && <span className="text-text-tertiary">{sep}</span>}
          <input className={FIELD} value={y} placeholder={b} aria-label={`${b} ${r + 1}`} onChange={(e) => put(r, 1, e.target.value)} />
          {units[1] && <span className="w-3 text-2xs text-text-tertiary">{units[1]}</span>}
          <button type="button" aria-label="Remove row" className="px-1 text-text-tertiary hover:text-status-danger" onClick={() => set(rows.filter((_, i) => i !== r))}>×</button>
        </div>
      ))}
      <Button type="button" size="sm" className="self-start" onClick={() => set([...rows, ["", ""]])}>{add}</Button>
    </div>
  );
}

/** Section heading inside a form. */
export const Section = ({ children }: { children: React.ReactNode }) => (
  <div className="mt-1 border-b border-border pb-0.5 text-2xs font-medium uppercase tracking-wide text-text-tertiary">{children}</div>
);

/** The SPICE text a form produces, always visible. */
export const SpiceLine = ({ children }: { children: React.ReactNode }) => (
  <code className="select-text whitespace-pre-wrap break-all rounded-control border border-border bg-surface-raised px-2 py-1 font-mono text-xs text-text-strong">{children}</code>
);

/** The parts of the schematic: SPICE instance name (from the netlist when current) and prefix letter. */
const circuitInsts = (items: Item[], nets: Netlist | null, lib: Lib) =>
  items.flatMap((it, i) => (it.t === "sym" ? [{ inst: nets?.insts[i] || it.attrs.InstName || "", prefix: defOf(lib, it)?.prefix?.[0]?.toUpperCase() ?? "" }] : [])).filter((x) => x.inst);

/** <datalist>s for the fields: "nets", "sources" (V/I), "sweepable" (sources, R, TEMP), "vexprs" (V(net)). */
export function CircuitLists() {
  const items = useS((s) => s.items);
  const nets = useS(currentNets); // null while the netlist lags the items (names are index-aligned)
  const lib = useS((s) => s.lib);
  const { names, sources, sweepable } = useMemo(() => {
    const names = [...new Set((nets?.nets ?? []).flat().filter(Boolean))].sort((a, b) => (a === "0" ? -1 : b === "0" ? 1 : a.localeCompare(b, undefined, { numeric: true })));
    const insts = circuitInsts(items, nets, lib);
    const sources = insts.filter((x) => x.prefix === "V" || x.prefix === "I").map((x) => x.inst);
    const sweepable = [...sources, ...insts.filter((x) => x.prefix === "R").map((x) => x.inst), "TEMP"];
    return { names, sources, sweepable };
  }, [items, nets, lib]);
  return (
    <>
      <datalist id="nets">{names.map((n) => <option key={n} value={n} />)}</datalist>
      <datalist id="sources">{sources.map((n) => <option key={n} value={n} />)}</datalist>
      <datalist id="sweepable">{sweepable.map((n) => <option key={n} value={n} />)}</datalist>
      <datalist id="vexprs">{names.filter((n) => n !== "0").map((n) => <option key={n} value={`V(${n})`} />)}{sources.map((n) => <option key={n} value={`I(${n})`} />)}</datalist>
    </>
  );
}
