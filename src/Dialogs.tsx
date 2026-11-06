import { memo, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-dialog";
import type { Item, LibEntry, LibSource } from "./types";
import { addTrace, commit, loadSymbols, placePart, placeSymbol, refreshLibrarySymbols, useS } from "./store";
import { defOf, vexpr } from "./geom";
import { Button } from "./ui/button";
import { FIELD_BASE, FIELD_FOCUS } from "./ui/focus";
import { cn, keys } from "@/lib/utils";
import thirdParty from "./third-party.json";

const close = () => useS.setState({ dialog: null });
const FIELD = cn(FIELD_BASE, FIELD_FOCUS, "h-[26px] w-full px-2 font-mono");

function Modal({ title, children, onSubmit, wide, ok = "OK", cancel = true }: { title: string; children: React.ReactNode; onSubmit: () => void; wide?: boolean | "xl"; ok?: string; cancel?: boolean }) {
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
      onClose={close}
      className={cn("m-auto rounded-float border border-menu-border bg-menu-bg p-0 text-text shadow-dialog backdrop:bg-scrim", wide === "xl" ? "w-[860px]" : wide ? "w-[560px]" : "w-[420px]")}
    >
      <form onSubmit={(e) => { e.preventDefault(); onSubmit(); }}>
        <header className="border-b border-border px-3 py-2 text-sm font-medium text-text-strong">{title}</header>
        <div className="flex flex-col gap-2.5 p-3">{children}</div>
        <footer className="flex justify-end gap-2 border-t border-border px-3 py-2">
          {cancel && <Button type="button" variant="ghost" onClick={close}>Cancel</Button>}
          <Button type="submit" variant="primary">{ok}</Button>
        </footer>
      </form>
    </dialog>
  );
}

function Field({ label, children, hint }: { label: string; children: React.ReactNode; hint?: string }) {
  return (
    <label className="grid grid-cols-[110px_1fr] items-center gap-2 text-xs text-text-secondary">
      <span>{label}</span>
      {children}
      {hint && <span className="col-start-2 text-2xs text-text-tertiary">{hint}</span>}
    </label>
  );
}

const SOURCE_PRESETS: [string, string][] = [
  ["DC", "5"],
  ["SINE", "SINE(0 1 1k)"],
  ["PULSE", "PULSE(0 5 0 1n 1n 0.5m 1m)"],
  ["AC", "AC 1"],
  ["PWL", "PWL(0 0 1m 1 2m 0)"],
];

/** Edits an item; `onDone` receives the result (commit in place, or start placing it). */
function ItemForm({ item, title, onDone }: { item: Item; title: string; onDone: (it: Item) => void }) {
  const lib = useS((s) => s.lib);
  const [it, setIt] = useState(item);
  const attr = (k: string) => (it.t === "sym" ? it.attrs[k] ?? "" : "");
  const setAttr = (k: string, v: string) => it.t === "sym" && setIt({ ...it, attrs: { ...it.attrs, [k]: v } });
  const done = () => {
    if (it.t === "sym") onDone({ ...it, attrs: Object.fromEntries(Object.entries(it.attrs).filter(([, v]) => v.trim())) });
    else if ((it.t === "flag" && !it.name.trim()) || (it.t === "text" && !it.text.trim())) close();
    else if (it.t === "text" && it.directive) {
      // forward slashes in include paths: portable, and safe from the .asc `\n` newline escape
      onDone({ ...it, text: it.text.split("\n").map((l) => (/^\s*\.(inc|lib)/i.test(l) ? l.replaceAll("\\", "/") : l)).join("\n") });
    } else onDone(it);
  };
  const def = it.t === "sym" ? defOf(lib, it) : undefined;
  return (
    <Modal title={title} onSubmit={done} wide={it.t === "text"}>
      {it.t === "sym" && (
        <>
          <div className="text-2xs text-text-tertiary">{def?.desc ?? it.sym} · {it.sym}</div>
          <Field label="Instance name"><input className={FIELD} value={attr("InstName")} onChange={(e) => setAttr("InstName", e.target.value)} /></Field>
          <Field label="Value" hint={def?.prefix === "V" || def?.prefix === "I" ? "SPICE syntax: 5 · SINE(off amp freq) · PULSE(v1 v2 td tr tf ton per) · AC 1" : undefined}>
            <input data-autofocus className={FIELD} value={attr("Value")} onChange={(e) => setAttr("Value", e.target.value)} />
          </Field>
          {(def?.prefix === "V" || def?.prefix === "I") && (
            <div className="ml-[118px] flex flex-wrap gap-1">
              {SOURCE_PRESETS.map(([name, v]) => (
                <Button key={name} type="button" size="sm" onClick={() => setAttr("Value", name === "AC" ? `${attr("Value").replace(/\s*AC\s+\S+/i, "")} AC 1`.trim() : v)}>{name}</Button>
              ))}
            </div>
          )}
          <Field label="Value2"><input className={FIELD} value={attr("Value2")} onChange={(e) => setAttr("Value2", e.target.value)} /></Field>
          <Field label="SpiceLine"><input className={FIELD} value={attr("SpiceLine")} placeholder={def?.spice_line} onChange={(e) => setAttr("SpiceLine", e.target.value)} /></Field>
        </>
      )}
      {it.t === "flag" && (
        <Field label="Net name" hint="0 = ground">
          <input data-autofocus className={FIELD} value={it.name} onChange={(e) => setIt({ ...it, name: e.target.value.replace(/\s/g, "_") })} />
        </Field>
      )}
      {it.t === "text" && (
        <>
          <textarea
            data-autofocus
            rows={5}
            className={cn(FIELD_BASE, FIELD_FOCUS, "w-full p-2 font-mono")}
            value={it.text}
            placeholder={it.directive ? ".tran 1m\n.param R=1k\n.model D1N4148 D(...)" : "Comment"}
            onChange={(e) => setIt({ ...it, text: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && (e.preventDefault(), done())}
          />
          <label className="flex items-center gap-2 text-xs text-text-secondary">
            <input type="checkbox" checked={it.directive} onChange={(e) => setIt({ ...it, directive: e.target.checked })} />
            SPICE directive (otherwise comment) · {keys("⌘↵")} to confirm
          </label>
        </>
      )}
    </Modal>
  );
}

const ANALYSIS = /^\.(tran|ac|dc|op|noise)\b/i;
/** Numbers and parameter expressions ({T}, 'T*2') are values; `uic` & co. are not. */
const isNum = (t: string) => /^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?[a-zµμ]*$/i.test(t) || /^[{']/.test(t);
/** Whitespace tokens, but `{a * b}` / `'a * b'` stay whole (same rule as schematic.rs). */
const tokens = (s: string) => s.match(/\{[^}]*\}|'[^']*'|[^\s(]*\([^)]*\)|\S+/g) ?? [];

function SimDialog() {
  const items = useS((s) => s.items);
  const idx = items.findIndex((it) => it.t === "text" && it.directive && it.text.split("\n").some((l) => ANALYSIS.test(l.trim())));
  const lines = idx >= 0 && items[idx].t === "text" ? items[idx].text.split("\n") : [];
  const li = lines.findIndex((l) => ANALYSIS.test(l.trim()));
  const tok = tokens(li >= 0 ? lines[li].trim() : ".tran 1m");
  const kind0 = (tok[0] ?? ".tran").slice(1).toLowerCase();
  const [kind, setKind] = useState(kind0);
  const [f, setF] = useState<Record<string, string>>((): Record<string, string> => {
    const t = tok.slice(1);
    if (kind0 === "tran") {
      const n = t.findIndex((x) => !isNum(x));
      const nums = n < 0 ? t : t.slice(0, n), rest = (n < 0 ? [] : t.slice(n)).join(" ");
      return nums.length === 1 ? { stop: nums[0], rest } : { step: nums[0] ?? "", stop: nums[1] ?? "", start: nums[2] ?? "", max: nums[3] ?? "", rest };
    }
    if (kind0 === "ac") return { sweep: t[0] ?? "dec", pts: t[1] ?? "", fstart: t[2] ?? "", fstop: t[3] ?? "", rest: t.slice(4).join(" ") };
    if (kind0 === "dc") return { src: t[0] ?? "", dstart: t[1] ?? "", dstop: t[2] ?? "", incr: t[3] ?? "", rest: t.slice(4).join(" ") };
    if (kind0 === "noise") return { out: t[0] ?? "", nsrc: t[1] ?? "", sweep: t[2] ?? "dec", pts: t[3] ?? "", fstart: t[4] ?? "", fstop: t[5] ?? "", rest: t.slice(6).join(" ") };
    return {};
  });
  const v = (k: string, d = "") => f[k] || d;
  const set = (k: string) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => setF({ ...f, [k]: e.target.value });
  const rest = kind === kind0 ? v("rest") : ""; // e.g. `uic`, a nested .dc sweep: keep what the form does not show
  const line = [
    kind === "tran" ? [".tran", v("step") || (v("start") || v("max") ? "0" : ""), v("stop", "1m"), v("start") || (v("max") ? "0" : ""), v("max")].filter(Boolean).join(" ")
    : kind === "ac" ? `.ac ${v("sweep", "dec")} ${v("pts", "100")} ${v("fstart", "10")} ${v("fstop", "1Meg")}`
    : kind === "dc" ? `.dc ${v("src", "V1")} ${v("dstart", "0")} ${v("dstop", "5")} ${v("incr", "10m")}`
    : kind === "noise" ? `.noise ${v("out", "v(out)")} ${v("nsrc", "V1")} ${v("sweep", "dec")} ${v("pts", "10")} ${v("fstart", "10")} ${v("fstop", "1Meg")}`
    : ".op",
    rest,
  ].filter(Boolean).join(" ");
  const submit = () => {
    if (idx >= 0) {
      const it = items[idx] as Extract<Item, { t: "text" }>;
      commit(items.map((x, i) => (i === idx ? { ...it, text: lines.map((l, j) => (j === li ? line : l)).join("\n") } : x)));
      close();
    } else useS.setState({ dialog: null, tool: { k: "place", item: { t: "text", x: 0, y: 0, text: line, directive: true } } });
  };
  const tabs = [["tran", "Transient"], ["ac", "AC Analysis"], ["dc", "DC Sweep"], ["noise", "Noise"], ["op", "DC op pnt"]];
  return (
    <Modal title="Edit Simulation Command" onSubmit={submit} wide>
      <div role="tablist" className="flex border-b border-border">
        {tabs.map(([k, label]) => (
          <button key={k} type="button" role="tab" aria-selected={kind === k} onClick={() => setKind(k)}
            className={cn("-mb-px border-b-2 px-3 py-1 text-xs", kind === k ? "border-selection text-text-strong" : "border-transparent text-text-secondary hover:text-text-strong")}>
            {label}
          </button>
        ))}
      </div>
      {kind === "tran" && (
        <>
          <Field label="Stop time"><input data-autofocus className={FIELD} value={v("stop")} placeholder="1m" onChange={set("stop")} /></Field>
          <Field label="Time step" hint="optional; default stop/1000"><input className={FIELD} value={v("step")} onChange={set("step")} /></Field>
          <Field label="Start saving"><input className={FIELD} value={v("start")} placeholder="0" onChange={set("start")} /></Field>
          <Field label="Max timestep"><input className={FIELD} value={v("max")} onChange={set("max")} /></Field>
        </>
      )}
      {kind === "ac" && (
        <>
          <Field label="Sweep">
            <select className={FIELD} value={v("sweep", "dec")} onChange={set("sweep")}>
              <option value="dec">Decade</option><option value="oct">Octave</option><option value="lin">Linear</option>
            </select>
          </Field>
          <Field label="Points"><input data-autofocus className={FIELD} value={v("pts")} placeholder="100" onChange={set("pts")} /></Field>
          <Field label="Start freq"><input className={FIELD} value={v("fstart")} placeholder="10" onChange={set("fstart")} /></Field>
          <Field label="Stop freq"><input className={FIELD} value={v("fstop")} placeholder="1Meg" onChange={set("fstop")} /></Field>
        </>
      )}
      {kind === "dc" && (
        <>
          <Field label="Source"><input data-autofocus className={FIELD} value={v("src")} placeholder="V1" onChange={set("src")} /></Field>
          <Field label="Start"><input className={FIELD} value={v("dstart")} placeholder="0" onChange={set("dstart")} /></Field>
          <Field label="Stop"><input className={FIELD} value={v("dstop")} placeholder="5" onChange={set("dstop")} /></Field>
          <Field label="Increment"><input className={FIELD} value={v("incr")} placeholder="10m" onChange={set("incr")} /></Field>
        </>
      )}
      {kind === "noise" && (
        <>
          <Field label="Output"><input data-autofocus className={FIELD} value={v("out")} placeholder="v(out)" onChange={set("out")} /></Field>
          <Field label="Input source"><input className={FIELD} value={v("nsrc")} placeholder="V1" onChange={set("nsrc")} /></Field>
          <Field label="Sweep">
            <select className={FIELD} value={v("sweep", "dec")} onChange={set("sweep")}>
              <option value="dec">Decade</option><option value="oct">Octave</option><option value="lin">Linear</option>
            </select>
          </Field>
          <Field label="Points"><input className={FIELD} value={v("pts")} placeholder="10" onChange={set("pts")} /></Field>
          <Field label="Start freq"><input className={FIELD} value={v("fstart")} placeholder="10" onChange={set("fstart")} /></Field>
          <Field label="Stop freq"><input className={FIELD} value={v("fstop")} placeholder="1Meg" onChange={set("fstop")} /></Field>
        </>
      )}
      {kind === "op" && <p className="text-xs text-text-secondary">Find the DC operating point; hover nets to read voltages.</p>}
      <code className="rounded-control bg-surface-raised px-2 py-1 font-mono text-xs text-text-strong">{line}</code>
    </Modal>
  );
}

const KINDS: [string, string | null][] = [
  ["All", null], ["Symbols", "SYMBOL"], ["Diodes", "D"], ["BJT", "NPN|PNP"], ["MOSFET", "NMOS|PMOS|VDMOS"], ["JFET", "NJF|PJF"], ["Subcircuits", "SUBCKT"],
];

/** Component library: the symbols plus every model and subcircuit of the configured libraries
 *  (LTspice, Micro-Cap 12, own folders), searchable; places a part with the matching symbol. */
function PartsDialog({ replace, kind0 }: { replace?: number; kind0?: string }) {
  const lib = useS((s) => s.lib);
  const [q, setQ] = useState("");
  const [kind, setKind] = useState<string | null>(kind0 ?? null);
  const [hits, setHits] = useState<LibEntry[]>([]);
  const [i, setI] = useState(0);
  const [text, setText] = useState("");
  const [view, setView] = useState<"parts" | "sources">("parts");
  const listRef = useRef<HTMLUListElement>(null);
  // only real pointer movement highlights: rows scrolled under a resting pointer must not steal it
  const last = useRef<[number, number]>([0, 0]);
  const hover = (e: React.MouseEvent, j: number) => {
    if (e.clientX === last.current[0] && e.clientY === last.current[1]) return;
    last.current = [e.clientX, e.clientY];
    setI(j);
  };
  useEffect(() => listRef.current?.children[i]?.scrollIntoView({ block: "nearest" }), [i]);
  const symbols = useMemo<LibEntry[]>(
    () => Object.values(lib).filter((d) => !d.name.startsWith("subckt:")).map((d) => ({ name: d.name, kind: "SYMBOL", pins: d.pins.map((p) => p.name), source: d.desc, file: "", symbol: d.name })),
    [lib],
  );
  useEffect(() => {
    let live = true;
    const t = setTimeout(async () => {
      const ql = q.trim().toLowerCase();
      const syms = kind && kind !== "SYMBOL" ? [] : symbols.filter((s) => `${s.name} ${s.source}`.toLowerCase().includes(ql)).sort((a, b) => a.name.localeCompare(b.name));
      const found = kind === "SYMBOL" || (!ql && !kind) ? [] : await invoke<LibEntry[]>("library_search", { query: ql, kind, limit: 300 });
      if (live) setHits([...syms, ...found]), setI(0);
    }, 60);
    return () => ((live = false), clearTimeout(t));
  }, [q, kind, symbols, view]); // view: back from the library list, results reflect its changes
  const cur = hits[i];
  useEffect(() => {
    let live = true;
    if (cur && cur.kind !== "SYMBOL") invoke<string | null>("library_text", { name: cur.name }).then((t) => live && setText(t ?? ""), () => live && setText(""));
    else setText("");
    return () => void (live = false);
  }, [cur]);
  const pick = async (e = cur) => {
    if (!e || !e.symbol) return; // nothing, or a model type without a symbol (switch, core ...): stay open
    close();
    const { items } = useS.getState();
    const target = replace != null ? items[replace] : undefined;
    if (target?.t === "sym" && e.kind !== "SYMBOL") {
      // "choose model": the part keeps its place; a subcircuit brings its own box symbol
      const sym = e.kind === "SUBCKT" ? e.symbol : target.sym;
      await loadSymbols([{ ...target, sym }]);
      commit(items.map((it) => (it === target ? { ...target, sym, attrs: { ...target.attrs, Value: e.name } } : it)));
    } else if (e.kind === "SYMBOL") placeSymbol(e.name);
    else placePart(e.symbol, e.name);
  };
  // distinct keys: switching views mounts a fresh dialog, so focus lands in the new view
  if (view === "sources") return <Modal key="sources" title="Model Libraries" wide="xl" ok="Done" onSubmit={() => setView("parts")}><Sources /></Modal>;
  return (
    <Modal key="parts" title={replace != null ? "Choose Model" : "Component Library"} wide="xl" ok={replace != null ? "Use model" : "Place"} onSubmit={() => pick()}>
      <div className="flex gap-2">
        <input
          data-autofocus className={FIELD} placeholder="Search symbols, models, subcircuits… (e.g. 2N2222, 1N4148, TL072)" value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") e.preventDefault(), setI(Math.min(i + 1, hits.length - 1));
            if (e.key === "ArrowUp") e.preventDefault(), setI(Math.max(i - 1, 0));
          }}
        />
        <Button type="button" onClick={() => setView("sources")}>Libraries…</Button>
      </div>
      <div className="flex flex-wrap gap-1">
        {KINDS.map(([label, k]) => (
          <button key={label} type="button" onClick={() => setKind(k)}
            className={cn("h-5 rounded-control border px-2 text-2xs", kind === k ? "border-selection bg-selection-soft text-text-strong" : "border-border-control text-text-secondary hover:bg-surface-hover")}>
            {label}
          </button>
        ))}
      </div>
      <div className="grid h-80 grid-cols-[1fr_1.1fr] gap-2">
        <ul ref={listRef} className="overflow-auto rounded-control border border-border">
          {hits.map((e, j) => (
            <li key={`${e.kind}:${e.name}`}>
              <button type="button" onClick={() => pick(e)} onMouseMove={(ev) => hover(ev, j)}
                className={cn("flex h-[24px] w-full items-center gap-2 px-2 text-left text-xs", j === i ? "bg-menu-highlight text-text-strong" : "text-text")}>
                <span className="min-w-0 flex-1 truncate font-mono">{e.name}</span>
                <span className="w-14 shrink-0 font-mono text-2xs text-text-tertiary">{e.kind === "SYMBOL" ? "symbol" : e.kind.toLowerCase()}</span>
                <span className="w-24 shrink-0 truncate text-2xs text-text-tertiary">{e.source}</span>
              </button>
            </li>
          ))}
          {!hits.length && <li className="p-3 text-xs text-text-tertiary">{q || kind ? "Nothing found." : "Type to search the symbols and model libraries."}</li>}
        </ul>
        <div className="min-w-0 overflow-auto rounded-control border border-border bg-surface-raised p-2">
          {cur ? (
            <>
              <div className="font-mono text-sm text-text-strong">{cur.name}</div>
              <div className="text-2xs text-text-tertiary">
                {cur.kind === "SYMBOL" ? `symbol · ${cur.source}` : `${cur.kind === "SUBCKT" ? "subcircuit" : `${cur.kind} model`} · ${cur.source} · ${cur.symbol ? `placed as ${cur.symbol.replace("subckt:", "box ")}` : "no symbol for this model type: use it via a directive"}`}
              </div>
              {cur.pins.length > 0 && <div className="mt-1 text-2xs text-text-secondary">pins: {cur.pins.join(" · ")}</div>}
              {cur.file && <div className="mt-1 truncate text-2xs text-text-tertiary" title={cur.file}>{cur.file}</div>}
              {text && <pre className="mt-2 font-mono text-2xs leading-4 whitespace-pre-wrap text-text">{text.length > 6000 ? text.slice(0, 6000) + "\n…" : text}</pre>}
            </>
          ) : <div className="text-xs text-text-tertiary">No selection</div>}
        </div>
      </div>
    </Modal>
  );
}

/** Library folders: detected (LTspice, Micro-Cap 12), added by the user, or downloaded. */
function Sources() {
  const [list, setList] = useState<LibSource[]>([]);
  const [busy, setBusy] = useState("");
  useEffect(() => void invoke<LibSource[]>("library_sources").then(setList), []);
  const save = async (next: LibSource[]) => {
    setList(next);
    await invoke("library_set_sources", { sources: next }).then(refreshLibrarySymbols, (e) => setBusy(String(e)));
  };
  const add = async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") await save([...list, { name: dir.split(/[\\/]/).pop() || dir, path: dir, enabled: true }]);
  };
  const microcap = async () => {
    setBusy("Downloading Micro-Cap 12 (60 MB) from the Internet Archive…");
    try {
      setList(await invoke<LibSource[]>("library_fetch_microcap"));
      setBusy("");
      await refreshLibrarySymbols();
    } catch (e) {
      setBusy(`Download failed: ${e}`);
    }
  };
  const hasMc = list.some((s) => /micro-cap/i.test(s.name));
  return (
    <>
      <p className="text-xs text-text-secondary">
        Models (<code>.model</code>) and subcircuits (<code>.subckt</code>) in these folders are searchable in the component library and
        are pulled into the netlist automatically when a part uses them. Earlier entries win when names repeat.
      </p>
      <ul className="divide-y divide-border rounded-control border border-border">
        {list.map((s, j) => (
          <li key={s.path} className="flex items-center gap-2 px-2 py-1.5 text-xs">
            <input type="checkbox" checked={s.enabled} onChange={(e) => save(list.map((x, k) => (k === j ? { ...x, enabled: e.target.checked } : x)))} />
            <span className="w-36 shrink-0 truncate text-text-strong">{s.name}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-2xs text-text-tertiary" title={s.path}>{s.path}</span>
            <Button type="button" size="sm" variant="ghost" onClick={() => save(list.filter((_, k) => k !== j))}>Remove</Button>
          </li>
        ))}
        {!list.length && <li className="p-2 text-xs text-text-tertiary">No libraries yet.</li>}
      </ul>
      <div className="flex flex-wrap gap-2">
        <Button type="button" onClick={add}>Add folder…</Button>
        {!hasMc && <Button type="button" onClick={microcap} disabled={!!busy && busy.startsWith("Downloading")}>Get Micro-Cap 12 library</Button>}
      </div>
      {!hasMc && (
        <p className="text-2xs text-text-tertiary">
          Micro-Cap 12 is freeware (Spectrum Software, 2019): its original package is downloaded from the Internet Archive and its
          LIBRARY folder (about 45,000 models and subcircuits) unpacked for local use.
        </p>
      )}
      {busy && <p className={cn("text-xs", busy.startsWith("Download failed") ? "text-status-danger" : "text-text-secondary")}>{busy}</p>}
    </>
  );
}

function TraceDialog() {
  const run = useS((s) => s.sim?.run);
  const vectors = run?.vectors ?? [];
  const noise = !!run?.plot.startsWith("noise"); // spectra, no node voltages: plain names
  const raw = noise || !!run?.plot.startsWith("sens"); // sensitivities: no node voltages either
  const items = useS((s) => s.items);
  // .noise <output> <source> ...: onoise is in the output's unit, inoise in the source's
  const nz = items.flatMap((it) => (it.t === "text" && it.directive ? it.text.split("\n") : [])).map((l) => tokens(l.trim())).find((t) => /^\.noise$/i.test(t[0] ?? ""));
  const noiseUnit = (e: string) => (/^inoise/i.test(e) ? (/^i/i.test(nz?.[2] ?? "") ? "A" : "V") : /^i\(/i.test(nz?.[1] ?? "") ? "A" : "V") + "/√Hz";
  const [expr, setExpr] = useState("");
  const submit = () => {
    const e = expr.trim();
    close();
    // a unit only for a single plain quantity (or a function of one); products like i(v1)*v(in)
    // get none rather than a wrong one. ngspice ph()/cph()/vp() are in radians.
    const plain = (x: string) => (raw && !noise ? "" : /^v\([^()]*\)$/i.test(x) ? "V" : /^i\([^()]*\)$/i.test(x) ? "A" : noise && /^\w+$/.test(x) ? noiseUnit(x) : "");
    const f = e.match(/^(\w+)\((.*)\)$/);
    const unit = f && /^v?db$/i.test(f[1]) ? "dB" : f && /^(c?ph|vp)$/i.test(f[1]) ? "rad"
      : f && /^(mag|vm|real|imag|re|im|abs)$/i.test(f[1]) ? plain(f[2]) : plain(e);
    if (e) addTrace(e, e, unit);
  };
  return (
    <Modal title="Add Trace" onSubmit={submit}>
      <Field label="Expression" hint="ngspice syntax, e.g. v(out)-v(in) · i(vsense_r1) · db(v(out)) · i(v1)*v(in)">
        <input data-autofocus list="vectors" className={FIELD} value={expr} onChange={(e) => setExpr(e.target.value)} />
      </Field>
      <datalist id="vectors">
        {vectors.filter((v) => v !== run?.scale).map((v) => (
          <option key={v} value={raw ? v : /^[\w+\-.]+#branch$/.test(v) ? `i(${v.replace("#branch", "")})` : /^[\w+\-.]+$/.test(v) ? vexpr(v) : v} />
        ))}
      </datalist>
    </Modal>
  );
}

/** Version, license, and every third-party component with its license (same list as NOTICE.md). */
function AboutDialog() {
  const [version, setVersion] = useState("");
  const [texts, setTexts] = useState<string | null>(null);
  useEffect(() => void getVersion().then(setVersion, () => {}), []);
  const table = (list: typeof thirdParty.included) => (
    <table className="w-full table-fixed text-xs">
      <thead className="text-left text-text-tertiary">
        <tr className="border-b border-border"><th className="w-[38%] py-1 font-normal">Component</th><th className="font-normal">Used for</th><th className="w-[30%] font-normal">License</th></tr>
      </thead>
      <tbody>
        {list.map((c) => (
          <tr key={c.name} className="border-b border-border-subtle align-top">
            <td className="py-1 pr-2 text-text-strong">{c.name}{c.url && <div className="select-text truncate font-mono text-2xs text-text-tertiary">{c.url}</div>}</td>
            <td className="py-1 pr-2 text-text-secondary">{c.use}</td>
            <td className="py-1 font-mono text-2xs">{c.license}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
  return (
    <Modal title="About nomospice" wide="xl" ok="Close" cancel={false} onSubmit={close}>
      <div className="flex max-h-[70vh] flex-col gap-3 overflow-auto pr-1 text-xs">
        <p className="leading-5 text-text-secondary">
          <span className="font-medium text-text-strong">nomospice {version}</span> is free software under the GNU Affero General
          Public License v3.0 or later. Source code: <span className="select-text font-mono">https://github.com/mprotogerakis/nomospice</span>
        </p>
        <h3 className="font-medium text-text-strong">Built with</h3>
        {table(thirdParty.included)}
        <h3 className="font-medium text-text-strong">Used, but not included</h3>
        {table(thirdParty.external)}
        {texts == null ? (
          <Button type="button" className="self-start" onClick={() => Promise.all([import("../LICENSE?raw"), import("../THIRD-PARTY-LICENSES.txt?raw")]).then(([own, third]) => setTexts(`${third.default}\n\n${"=".repeat(80)}\nnomospice - GNU Affero General Public License v3.0\n${"=".repeat(80)}\n\n${own.default}`))}>
            Show full license texts
          </Button>
        ) : (
          <pre className="max-h-[40vh] select-text overflow-auto rounded-control border border-border bg-surface-panel p-2 font-mono text-2xs leading-4 text-text-secondary">{texts}</pre>
        )}
      </div>
    </Modal>
  );
}

export const Dialogs = memo(function Dialogs() {
  const dialog = useS((s) => s.dialog);
  const items = useS((s) => s.items);
  if (!dialog) return null;
  switch (dialog.k) {
    case "edit": {
      const it = items[dialog.index];
      if (!it) return null;
      return <ItemForm key={dialog.index} item={it} title={it.t === "sym" ? `Edit ${it.attrs.InstName ?? it.sym}` : it.t === "flag" ? "Net Name" : "Edit Text"}
        onDone={(next) => (commit(items.map((x, i) => (i === dialog.index ? next : x))), close())} />;
    }
    case "new": {
      const at = dialog.at;
      return <ItemForm item={dialog.item} title={dialog.item.t === "flag" ? "Net Name" : dialog.item.t === "text" && dialog.item.directive ? "SPICE Directive" : "Comment"}
        onDone={(it) => at && it.t !== "wire" && it.t !== "raw" ? (commit([...items, { ...it, x: at[0], y: at[1] }]), close()) : useS.setState({ dialog: null, tool: { k: "place", item: it } })} />;
    }
    case "sim": return <SimDialog />;
    case "parts": return <PartsDialog replace={dialog.replace} kind0={dialog.kind} />;
    case "trace": return <TraceDialog />;
    case "about": return <AboutDialog />;
  }
});
