import { memo, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { message, open } from "@tauri-apps/plugin-dialog";
import type { Item, LibEntry, LibSource } from "./types";
import { addTrace, analysisItem, commit, currentTab, fileName, forDoc, isSymbolDoc, loadSymbols, placePart, placeSymbol, setYAxis, useS } from "./store";
import { defOf, pinLine, portOf, prefixOf, vexpr } from "./geom";
import { Button } from "./ui/button";
import { closeDialog as close, Field, FIELD, Modal, numHelp, NumField, Rows, TextField } from "./ui/dialog";
import { SourceEditor } from "./SourceEditor";
import { SimDialog } from "./SimDialog";
import { DirectiveDialog } from "./DirectiveDialog";
import { analysisEditable, analysisEditableAt, analysisLine, analysisLineIndex, analysisOptions, balanced, exprUnit, getParam, paramValue, setParam, spiceNum, tokens, xUnitOf } from "./spice";
import { cn, keys as shortcut } from "@/lib/utils";
import thirdParty from "./third-party.json";
import { checkForUpdate } from "./updater";


/** The attributes the part form shows as fields (the rest: "Other attributes"), and the other
 *  LTspice attributes in their spelling (rows typed in another case are saved so). */
const SHOWN = ["InstName", "Value", "Value2", "SpiceLine"];
const MORE = ["Prefix", "SpiceLine2", "SpiceModel", "Description"];

/** A semiconductor's common instance parameters, kept where LTspice keeps them, in Value2
 *  (`l=0.18u w=1u`): [key, label, unit, hint]. The rest stays as typed in Value2. */
const AREA_M: [string, string, string, string][] = [["area", "Area factor", "", "relative to the model's"], ["m", "Multiplier", "", "devices in parallel"]];
const INSTANCE: Record<string, [string, string, string, string][]> = {
  M: [["l", "Length L", "m", "channel length, e.g. 180n (not for power MOSFETs: VDMOS)"], ["w", "Width W", "m", "channel width, e.g. 1u"], ["m", "Multiplier", "", "devices in parallel"]],
  Q: AREA_M,
  D: AREA_M,
};

/** Edits a part or a net label; `onDone` receives the result (commit in place, or start placing it).
 *  Comments and directives use DirectiveDialog. */
function ItemForm({ item, title, onDone }: { item: Item; title: string; onDone: (it: Item) => void }) {
  const lib = useS((s) => s.lib);
  // (a label in a symbol is one of its pins: a PIN line always, where its name shows)
  const symbol = useS((s) => isSymbolDoc(s.path));
  const [it, setIt] = useState(item.t === "flag" && symbol ? forDoc(item) ?? item : item);
  const attr = (k: string) => (it.t === "sym" ? it.attrs[k] ?? "" : "");
  const setAttr = (k: string, v: string) => it.t === "sym" && setIt({ ...it, attrs: { ...it.attrs, [k]: v } });
  const def = it.t === "sym" ? defOf(lib, it) : undefined;
  // the other attributes (LTspice's attribute editor: Prefix X to call a subcircuit, SpiceModel …)
  const [extra, setExtra] = useState<string[][]>(() => (item.t === "sym" ? Object.entries(item.attrs).filter(([k]) => !SHOWN.includes(k)) : []));
  const done = () => {
    // (a shown field's value not written yet: said under it; one of a field no longer shown, another prefix: dropped)
    if (Object.entries(drafts).some(([k, d]) => d != null && INSTANCE[prefix]?.some(([f]) => f === k))) return;
    // empty attributes go, except an emptied SpiceLine over a symbol default (missing = the default);
    // sorted as the .asc reader gives them, so an unchanged part compares equal
    if (it.t === "sym") {
      const own = Object.entries(it.attrs).filter(([k, v]) => SHOWN.includes(k) && (v.trim() || (k === "SpiceLine" && def?.spice_line)));
      // a row naming one of the fields above (Value …) sets it: rows come last and win; LTspice's
      // attribute names in their own spelling (`prefix` is Prefix), as the netlister reads them
      // (only the other names: a row spelled `value` from a file is not the Value field, which the
      // user may have edited above; a row named exactly Value is, and wins)
      const canon = (k: string) => MORE.find((n) => n.toLowerCase() === k.toLowerCase()) ?? k;
      const more = extra.map(([k, v]) => [canon(k.trim()), v] as [string, string]).filter(([k, v]) => k && v.trim());
      onDone({ ...it, attrs: Object.fromEntries([...new Map([...own, ...more])].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) });
    }
    else if (it.t === "flag" && !it.name.trim() && !symbol) close(); // (a symbol's pin may have no name)
    else onDone(it);
  };
  // independent sources (V, I; not the controlled E/G/F/H) get the waveform form
  // the part's own Prefix wins, as in the netlist — as typed in the rows below, live
  const row = (name: string) => extra.filter(([k]) => k.trim().toLowerCase() === name).at(-1)?.[1] ?? ""; // the last row wins, as saved
  const prefixRow = row("prefix");
  const [drafts, setDrafts] = useState<Record<string, string | undefined>>({});
  const prefix = it.t === "sym" ? prefixOf({ ...it, attrs: { ...it.attrs, Prefix: prefixRow } }, def) : "";
  const source = prefix === "V" || prefix === "I";
  // R, C, L: show how SPICE reads a single value (1M is a milliohm, 10F ten femtofarad)
  const unit = ({ R: "Ω", C: "F", L: "H" } as Record<string, string>)[prefix];
  // a plain value only: not `10µ Rser=0.1`, not LTspice's behavioural `R=V(a)*1k`
  const valueHelp = unit && !/[\s=(]/.test(attr("Value").trim()) ? numHelp(attr("Value"), unit, (v) => setAttr("Value", v)) : null;
  return (
    <Modal title={title} onSubmit={done} wide={source ? "xl" : false}>
      {it.t === "sym" && (
        <>
          <div className="text-2xs text-text-tertiary">{def?.desc ?? it.sym} · {it.sym}</div>
          <TextField label="Instance name" value={attr("InstName")} set={(v) => setAttr("InstName", v)} />
          {source ? <SourceEditor attrs={it.attrs} setAttrs={(attrs) => setIt({ ...it, attrs })} current={prefix === "I"} spiceLine={def?.spice_line} /> : (
            <>
              <TextField label="Value" value={attr("Value")} set={(v) => setAttr("Value", v)} autofocus unit={unit}
                {...(valueHelp ?? {})} hint={unit ? "e.g. 4.7k, 100n, 1Meg; LTspice extras like Rser=0.1 may follow" : def?.sheet ? "parameters for its schematic, e.g. R=10k (a .param R there is the default)" : undefined} />
              {/* (D, Q, M: their common instance parameters as fields; Value2 holds them, as SPICE reads them) */}
              {/* (Value2 itself with an open brace or quote: its items cannot be told apart, edited there) */}
              {INSTANCE[prefix] && !balanced(attr("Value2")) && <div className="text-2xs text-status-warning">⚠ Value2 has an unclosed brace or quote: edit it there.</div>}
              {balanced(attr("Value2")) && INSTANCE[prefix]?.map(([k, label, u, hint]) => {
                // (a value being typed, `{2*` or `'a`: shown as typed, written once it is one closed
                // item, so the open brace does not swallow the rest of Value2; till then said)
                const v = drafts[k] ?? getParam(attr("Value2"), k);
                const set = (x: string) => {
                  const ok = !paramValue(x);
                  setDrafts({ ...drafts, [k]: ok ? undefined : x });
                  if (ok) setAttr("Value2", setParam(attr("Value2"), k, x));
                };
                const pending = drafts[k] != null ? `${paramValue(drafts[k]!)}: not in Value2` : undefined;
                // (the netlist line is Value Value2 SpiceLine SpiceLine2, and SPICE takes the last of a
                // key: one in SpiceLine (or the symbol's) or SpiceLine2 wins over this field, said)
                const has = (n: string, line: string) => ((x) => (x ? `${n} has ${k}=${x}` : ""))(getParam(line, k));
                // (the symbol's Value if the part's is empty, as the netlist does)
                const before = has("Value", it.attrs.Value?.trim() ? it.attrs.Value : def?.value ?? "");
                const after = has("SpiceLine2", row("spiceline2")) || has("SpiceLine", it.attrs.SpiceLine ?? def?.spice_line ?? ""); // (the last wins)
                let note = hint;
                if (after) note = `${after}, which comes later on the line and wins`;
                else if (before) note = getParam(attr("Value2"), k) ? `${before}; this one wins` : `${before}, in effect`;
                // (not yet written: said first, over a number's own warning)
                const help = numHelp(v, u, set);
                return <TextField key={k} label={label} value={v} set={set} unit={u} placeholder={k === "m" || k === "area" ? "1" : "100µ (ngspice's default)"} info={help.info} warn={pending ?? (help.warn || undefined)} hint={note} />;
              })}
              <TextField label="Value2" value={attr("Value2")} set={(v) => (setDrafts({}), setAttr("Value2", v))} hint={INSTANCE[prefix] ? "the instance parameters as SPICE reads them (others too: off, temp=…)" : undefined} />
              {/* the symbol's default shows as the value: emptying it visibly removes it (an empty attribute) */}
              <TextField label="SpiceLine" value={it.attrs.SpiceLine ?? def?.spice_line ?? ""} set={(v) => setAttr("SpiceLine", v)} hint={def?.spice_line ? `symbol default: ${def.spice_line}` : undefined} />
            </>
          )}
          <Field label="Other attributes" hint={`e.g. Prefix X to call a subcircuit model instead of the ${def?.prefix || "symbol's"} device`}>
            <Rows rows={extra} set={setExtra} a="attribute" b="value" list="sym-attrs" add="Add attribute" autofocus={false} />
          </Field>
          <datalist id="sym-attrs">{MORE.slice(0, 2).map((n) => <option key={n} value={n} />)}</datalist>
        </>
      )}
      {it.t === "flag" && symbol && (
        <>
          <Field label="Pin name" hint="the name a schematic's netlist and its block's labels go by">
            <input data-autofocus className={FIELD} value={it.name} onChange={(e) => setIt({ ...it, name: e.target.value.replace(/\s/g, "_") })} />
          </Field>
          {/* (its PIN line: where the name shows on the placed part, and how far from the pin) */}
          <Field label="Name shown" hint="on the placed part, from the pin">
            <select className={FIELD} value={pinJust(it.extra)} onChange={(e) => setIt({ ...it, extra: [`PIN ${it.x} ${it.y} ${e.target.value} ${pinOff(it.extra, e.target.value)}`, ...(it.extra ?? []).filter((l) => !l.startsWith("PIN "))] })}>
              <option value="NONE">not</option>
              <option value="LEFT">right of it (LEFT)</option>
              <option value="RIGHT">left of it (RIGHT)</option>
              <option value="TOP">below it (TOP)</option>
              <option value="BOTTOM">above it (BOTTOM)</option>
              <option value="VTOP">right of it, upright (VTOP)</option>
              <option value="VBOTTOM">left of it, upright (VBOTTOM)</option>
              <option value="VLEFT">above it, upright (VLEFT)</option>
              <option value="VRIGHT">below it, upright (VRIGHT)</option>
            </select>
          </Field>
        </>
      )}
      {it.t === "flag" && !symbol && (
        <>
          <Field label="Net name" hint="0 = ground">
            <input data-autofocus className={FIELD} value={it.name} onChange={(e) => setIt({ ...it, name: e.target.value.replace(/\s/g, "_") })} />
          </Field>
          {/* (LTspice's IOPIN: a pin of the schematic's block symbol) */}
          <Field label="Port" hint="a pin of this schematic's block symbol (Create block symbol: inputs left, outputs right)">
            <select className={FIELD} value={portOf(it.extra)} onChange={(e) => setIt({ ...it, extra: [...(it.extra ?? []).filter((l) => !l.startsWith("IOPIN ")), ...(e.target.value ? [`IOPIN ${it.x} ${it.y} ${e.target.value}`] : [])] })}>
              <option value="">none</option>
              <option value="In">input</option>
              <option value="Out">output</option>
              <option value="BiDir">bidirectional</option>
            </select>
          </Field>
        </>
      )}
    </Modal>
  );
}

/** Where a pin's name shows (its PIN line's justification: NONE … ; `none` if it has no PIN line). */
const pinJust = (extra?: string[]) => pinLine(extra)?.just ?? "NONE";
/** How far from the pin its name shows once `just`: as it was (a hidden one's 0, LTspice's, made 8). */
const pinOff = (extra: string[] | undefined, just: string) => ((p) => (p && (p.off || just === "NONE") ? p.off : 8))(pinLine(extra));

/** A symbol's attributes (SYMATTR lines: Prefix, Value, Description, SpiceModel …) and its type:
 *  a part (CELL) or a hierarchical block (BLOCK, its schematic of the same name). */
function SymbolAttrs() {
  const items = useS((s) => s.items);
  const raw = (it: Item) => (it.t === "raw" ? it.line : "");
  const [kind, setKind] = useState(() => items.map(raw).find((l) => l.startsWith("SymbolType "))?.slice(11).trim().toUpperCase() || "CELL");
  // (the value as written after the name and its one space; one without a value kept as such)
  const [rows, setRows] = useState<string[][]>(() => items.map(raw).flatMap((l) => {
    const m = /^SYMATTR (\S+) ?(.*)$/.exec(l);
    return m ? [[m[1], m[2]]] : [];
  }));
  const done = () => {
    const keep = items.filter((it) => !(it.t === "raw" && (it.line.startsWith("SYMATTR ") || it.line.startsWith("SymbolType "))));
    const attrs = rows.filter(([k]) => k.trim()).map(([k, v]): Item => ({ t: "raw", line: `SYMATTR ${k.trim().replace(/\s+/g, "")}${v ? ` ${v}` : ""}` }));
    commit([{ t: "raw", line: `SymbolType ${kind}` }, ...keep, ...attrs], []); // (the items moved: nothing selected)
    close();
  };
  return (
    <Modal title="Symbol attributes" onSubmit={done}>
      <Field label="Type" hint="a block stands for the schematic of its name beside it">
        <select className={FIELD} value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="CELL">part (CELL)</option>
          <option value="BLOCK">hierarchical block (BLOCK)</option>
        </select>
      </Field>
      <Field label="Attributes" hint="Prefix (R, C, X …), Value, Description, SpiceModel, ModelFile …: a placed part takes them">
        <Rows rows={rows} set={setRows} a="attribute" b="value" list="sym-attr-names" add="Add attribute" />
      </Field>
      <datalist id="sym-attr-names">{["Prefix", "Value", "Description", "SpiceModel", "ModelFile", "SpiceLine", "Value2"].map((n) => <option key={n} value={n} />)}</datalist>
    </Modal>
  );
}

const KINDS: [string, string | null][] = [
  ["All", null], ["Symbols", "SYMBOL"], ["Diodes", "D"], ["BJT", "NPN|PNP"], ["MOSFET", "NMOS|PMOS|VDMOS"], ["JFET", "NJF|PJF"], ["Subcircuits", "SUBCKT"],
];

/** Component library: the symbols plus every model and subcircuit of the configured libraries
 *  (LTspice, Micro-Cap 12, own folders), searchable; places a part with the matching symbol. */
function PartsDialog({ replace, kind0, libraries }: { replace?: number; kind0?: string; libraries?: boolean }) {
  const lib = useS((s) => s.lib);
  const [q, setQ] = useState("");
  const [kind, setKind] = useState<string | null>(kind0 ?? null);
  const [hits, setHits] = useState<LibEntry[]>([]);
  const [i, setI] = useState(0);
  const [text, setText] = useState("");
  const [view, setView] = useState<"parts" | "sources">(libraries ? "sources" : "parts");
  const [start, setStart] = useState<"add" | "microcap">(); // the empty-library hint's buttons act at once
  // no library in use: say where they come from (the settings are otherwise one click away)
  const [srcs, setSrcs] = useState<LibSource[] | null>(null);
  useEffect(() => void invoke<LibSource[]>("library_sources").then(setSrcs, () => {}), [view]);
  const listRef = useRef<HTMLUListElement>(null);
  // only real pointer movement highlights: rows scrolled under a resting pointer must not steal it
  const last = useRef<[number, number]>([0, 0]);
  const hover = (e: React.MouseEvent, j: number) => {
    if (e.clientX === last.current[0] && e.clientY === last.current[1]) return;
    last.current = [e.clientX, e.clientY];
    setI(j);
  };
  // braces: Chromium's (WebView2) scrollIntoView returns a Promise, which React would call as the cleanup
  useEffect(() => { listRef.current?.children[i]?.scrollIntoView({ block: "nearest" }); }, [i]);
  const symbols = useMemo<LibEntry[]>(
    () => Object.values(lib).filter((d) => !d.name.startsWith("subckt:")).map((d) => ({ name: d.name, kind: "SYMBOL", pins: d.pins.map((p) => p.name), source: d.desc || (d.sheet ? `block: ${fileName(d.sheet)}` : ""), file: "", symbol: d.name })),
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
    if (isSymbolDoc(useS.getState().path) && replace == null) return void message("A symbol holds no parts: open a schematic to place one.", { title: "nomospice" });
    close();
    const { items } = useS.getState();
    const target = replace != null ? items[replace] : undefined;
    if (target?.t === "sym" && e.kind !== "SYMBOL") {
      // "choose model": the part keeps its place; a subcircuit brings its own box symbol — unless the
      // part calls subcircuits through its own Prefix (X on a MOSFET symbol): its symbol stays
      const sym = e.kind === "SUBCKT" && !target.attrs.Prefix?.trim() ? e.symbol : target.sym;
      await loadSymbols([{ ...target, sym }]);
      commit(items.map((it) => (it === target ? { ...target, sym, attrs: { ...target.attrs, Value: e.name } } : it)));
    } else if (e.kind === "SYMBOL") placeSymbol(e.name);
    else placePart(e.symbol, e.name);
  };
  // distinct keys: switching views mounts a fresh dialog, so focus lands in the new view
  if (view === "sources") return <Modal key="sources" title="Model Libraries" wide="xl" ok="Done" onSubmit={() => (libraries ? close() : (setStart(undefined), setView("parts")))}><Sources start={start} /></Modal>;
  const setUp = (s: "add" | "microcap") => (setStart(s), setView("sources"));
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
      {srcs && !srcs.some((s) => s.enabled) && (
        <div className="flex flex-wrap items-center gap-2 rounded-control border border-border bg-surface-panel p-2 text-xs text-text-secondary">
          <span className="min-w-0 flex-1">No model library in use: besides the symbols, only nomospice's default models (1N4148, 2N3904, 2N3906, switches) are here. An installed LTspice is found by itself.</span>
          <Button type="button" size="sm" onClick={() => setUp("microcap")}>Get Micro-Cap 12 library</Button>
          <Button type="button" size="sm" onClick={() => setUp("add")}>Add folder…</Button>
        </div>
      )}
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
function Sources({ start }: { start?: "add" | "microcap" }) {
  const [list, setList] = useState<LibSource[]>([]);
  const [busy, setBusy] = useState("");
  useEffect(() => void invoke<LibSource[]>("library_sources").then(setList), []);
  const started = useRef(false); // once (StrictMode runs effects twice in development)
  useEffect(() => {
    if (started.current || !start) return;
    started.current = true;
    void (start === "add" ? add() : microcap());
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const save = async (next: LibSource[]) => {
    setList(next);
    await invoke("library_set_sources", { sources: next }).catch((e) => setBusy(String(e))); // (every window then reloads its parts)
  };
  const add = async () => {
    const dir = await open({ directory: true, multiple: false });
    // the list as stored (the shown one may not have loaded yet)
    if (typeof dir === "string") await save([...(await invoke<LibSource[]>("library_sources")), { name: dir.split(/[\\/]/).pop() || dir, path: dir, enabled: true }]);
  };
  const microcap = async () => {
    // already downloaded (still on disk), only switched off: switch it on again instead of fetching 60 MB
    const cur = await invoke<LibSource[]>("library_sources");
    // (the download, or a detected install switched off)
    const mine = cur.find((s) => /micro-cap/i.test(s.name) && !s.enabled);
    if (mine && (await invoke<boolean>("path_exists", { path: mine.path }))) return save(cur.map((s) => (s === mine ? { ...s, enabled: true } : s)));
    setBusy("Downloading Micro-Cap 12 (60 MB) from the Internet Archive…");
    try {
      setList(await invoke<LibSource[]>("library_fetch_microcap"));
      setBusy("");
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
    // the unit by dimensional analysis (V·A = W, V/A = Ω; none rather than a wrong one); a plain
    // vector name: a node is volts, a branch or device current amperes. ngspice ph()/cph()/vp()
    // are in radians.
    const vectorUnit = (n: string) => {
      const v = vectors.find((x) => x.toLowerCase() === n.toLowerCase());
      // (a device parameter @m1[gm] … has a unit of its own: none rather than a wrong one)
      // (a part current by name, Ig(M1): amperes)
      return !v ? "" : v === run?.scale ? xUnitOf(v) : /#branch$|^@.*\[i\]$|^I\w*\(.*\)$/i.test(v) ? "A" : v.startsWith("@") ? "" : "V";
    };
    const f = e.match(/^(\w+)\((.*)\)$/);
    const unit = f && /^v?db$/i.test(f[1]) ? "dB" : f && /^(c?ph|vp)$/i.test(f[1]) ? "rad"
      : noise ? ((a) => (a === run?.scale ? xUnitOf(a) : /^\w+$/.test(a) ? noiseUnit(a) : ""))(f && /^(mag|vm|real|re|abs)$/i.test(f[1]) ? f[2] : e) // (a spectrum, or a magnitude of one)
      : raw ? "" : ((u) => (u === "W" && run?.plot.startsWith("ac") ? "VA" : u))(exprUnit(e, vectorUnit, xUnitOf(run?.scale ?? ""))); // (V·I in .ac: complex, VA)
    if (e) addTrace(e, e, unit);
  };
  return (
    <Modal title="Add Trace" onSubmit={submit}>
      <Field label="Expression" hint="ngspice syntax and part currents, e.g. V(out)-V(in) · I(R1)-I(R2) · Ib(Q1) · P(R1)+P(R2) · db(V(out))">
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

/** What a search finds in an item: the fields it is known by (each searched on its own, plus the
 *  row as listed, so "R1 10k" finds "R1  10k"), and how it is listed. */
function findable(it: Item): { fields: string[]; shown: string; kind: string } | null {
  if (it.t === "sym") {
    const a = it.attrs, kind = /^subckt:/i.test(it.sym) ? it.sym : it.sym.split(/[\\/]/).pop()!; // (the symbol's name, not its library's folders; a subckt: name whole)
    const shown = [a.InstName, a.Value].filter(Boolean).join("  ") || kind;
    // (every attribute: SpiceModel, Prefix …; and the row as listed, "R1  10k")
    return { fields: [...Object.values(a), kind, shown].filter(Boolean), shown, kind };
  }
  if (it.t === "flag") return it.name === "0" ? null : { fields: [it.name], shown: it.name, kind: "net label" }; // (grounds: everywhere, found by every 0)
  if (it.t === "text") return it.text.trim() ? { fields: [it.text], shown: it.text.split("\n").find((l) => l.trim())!, kind: it.directive ? "directive" : "comment" } : null;
  return null;
}

/** Find parts (name, value, model), net labels and texts: click one to select it and bring it into
 *  view, Enter for all of them. */
function FindDialog() {
  const items = useS((s) => s.items);
  const [q, setQ] = useState("");
  const norm = (x: string) => x.toLowerCase().replace(/\s+/g, " "); // ("R1 10k" finds the row "R1  10k")
  // (once per schematic, not per keystroke)
  const all = useMemo(() => items.flatMap((it, i) => ((f) => (f ? [{ i, ...f, words: f.fields.map(norm) }] : []))(findable(it))), [items]);
  const hits = useMemo(() => {
    const t = norm(q.trim());
    return t ? all.filter((h) => h.words.some((w) => w.includes(t))) : [];
  }, [all, q]);
  const show = (idx: number[]) => {
    close();
    useS.setState({ sel: idx, tool: useS.getState().tool.k === "move" ? { k: "move" } : { k: "select" } }); // (not a part still being placed)
    setTimeout(() => window.dispatchEvent(new CustomEvent("nomospice:show", { detail: idx })));
  };
  return (
    <Modal title="Find" ok={hits.length > 1 ? `Select all ${hits.length}` : hits.length ? "Select" : "Close"} cancel={!!hits.length} onSubmit={() => (hits.length ? show(hits.map((h) => h.i)) : close())}>
      <Field label="Name, value or text" hint={q.trim() ? (hits.length ? undefined : "nothing found") : "a part's name or value (R1, 10k, BC547), a net label, a directive"}>
        <input data-autofocus className={FIELD} value={q} onChange={(e) => setQ(e.target.value)} spellCheck={false} />
      </Field>
      {!!hits.length && (
        <div className="flex max-h-[40vh] flex-col overflow-y-auto rounded-control border border-border">
          {hits.slice(0, 200).map((h) => (
            <button key={h.i} type="button" className="flex items-baseline gap-2 px-2 py-1 text-left text-xs hover:bg-surface-hover" onClick={() => show([h.i])}>
              <span className="truncate font-mono text-text">{h.shown}</span>
              <span className="ml-auto shrink-0 text-2xs text-text-tertiary">{h.kind}</span>
            </button>
          ))}
          {hits.length > 200 && <div className="px-2 py-1 text-2xs text-text-tertiary">… {hits.length - 200} more (Enter selects all)</div>}
        </div>
      )}
    </Modal>
  );
}

/** A pane's fixed y range; both fields empty: automatic again. */
function YRangeDialog({ pane, unit, log }: { pane: string; unit: string; log: boolean }) {
  const y = useS((s) => s.plot.y[pane]);
  const [lo, setLo] = useState(y?.lo == null ? "" : String(y.lo)), [hi, setHi] = useState(y?.hi == null ? "" : String(y.hi));
  const [a, b] = [spiceNum(lo.replace("°", "")), spiceNum(hi.replace("°", ""))], empty = !lo.trim() && !hi.trim(); // (spiceNum knows units of letters only)
  const bad = empty ? null : a == null || b == null ? "two numbers" : !(a < b) ? "the first the smaller" : log && a <= 0 ? "above 0 (a log axis)" : null;
  const submit = () => {
    if (bad) return;
    close();
    setYAxis(pane, { ...y, lo: empty ? undefined : a!, hi: empty ? undefined : b! });
  };
  return (
    <Modal title="Y Range" onSubmit={submit}>
      {/* (degrees: no SPICE reading to show; ° typed is fine) */}
      {((Num) => <>
        <Num label="From" value={lo} set={setLo} unit={unit} autofocus />
        <Num label="To" value={hi} set={setHi} unit={unit} hint="both empty: automatic" />
      </>)(unit === "°" ? TextField : NumField)}
      {bad && <p className="text-2xs text-status-warning">⚠ {bad}</p>}
    </Modal>
  );
}

/** The quick reference (F1): what the keys, clicks and menus do, by task. ⌘ is Ctrl off a Mac. */
const HELP: [string, [string, string][]][] = [
  ["Drawing", [
    ["R C L D V I", "place a resistor, capacitor, inductor, diode, voltage or current source"],
    ["P, F2, ⌘K", "any part: the component library (symbols, models, subcircuits)"],
    ["G · W · N", "ground · wire (click, click; Esc ends) · net label"],
    ["S · T", "SPICE directive (.tran, .param, .step …) · comment"],
    ["⌃R · ⌃E", "rotate · mirror (the part being placed, or the selection)"],
    ["drag · M or F7", "move parts, their wires following · move them off their wires"],
    ["⌘C ⌘X ⌘V · ⌘D", "copy, cut, paste (also between windows) · duplicate"],
    ["⌘Z · ⇧⌘Z · ⌘A", "undo · redo · select all"],
    ["Del, ⌫ · F5", "delete the selection · the delete tool (click what goes)"],
    ["⌘F · Space", "find parts, labels, texts · fit the view"],
    ["double-click · right-click", "edit a part (value, model, its L/W/area) · its menu"],
  ]],
  ["Simulating", [
    ["⌘↵", "run (the toolbar's lightning: Live, a run after every edit)"],
    ["the toolbar's sliders", "Edit simulation command: the analysis in a form (.tran, .ac, .dc, .op, .noise, .tf, .pz, .sens), each its own result tab"],
    ["⌘L", "the log and the netlist"],
    [".step param R list 1k 2k", "a family of curves, .meas per step; {mc(1k,0.05)} with .step param run 1 100 1: Monte Carlo"],
    ["E1 … Laplace=1/(1+s*tau)", "a transfer function as a source"],
    ["⌘O a .cir, .net, .sp", "a netlist, run as it is (its first save asks where: an .asc, never over it)"],
  ]],
  ["Plotting", [
    ["click a wire · a part", "its voltage · its current (near a pin: that pin's, Ib(Q1))"],
    ["drag from wire to wire", "the voltage between them, V(a,b)"],
    ["Add trace", "an expression: V(out)-V(in), I(R1), P(R1)+P(R2) (a part's power, W), db(), group_delay() …"],
    ["click · shift-click", "cursor A · cursor B; shift-drag: the range A–B (average, RMS, integral)"],
    ["drag · wheel · dbl-click", "zoom · zoom and pan · reset"],
    ["right-click a trace's row", "(in the table beside the plot) right axis, another unit's pane, group delay, remove"],
    ["right-click the plot", "FFT, Nyquist, Nichols, log or fixed y axis, CSV, SVG, .raw"],
  ]],
  ["Files", [
    ["⌘S · ⌘O", "save · open (.asc, netlists): here while this window is empty, else in a window of its own"],
    ["⌘N · ⌘W", "new window · close it (the last one: emptied)"],
    ["the menu bar", "every command, with its key"],
    ["⌘,", "model libraries (Micro-Cap 12, your folders)"],
    ["drop a .lib/.sub on it", "an .include directive for it"],
    ["right-click the canvas", "export the schematic as SVG"],
  ]],
  ["Hierarchy", [
    ["a label's Port", "in, out, bidirectional: a pin of this schematic's block"],
    ["right-click the canvas", "Create block symbol: name.asy beside it, ports as pins; place it in a schematic of that folder"],
    ["right-click a block", "Open its schematic; the status bar's ↑ leads back up"],
    ["a block's Value", "parameters for its schematic: R=10k (its .param R=1k the default)"],
    ["⌘O an .asy · right-click a part", "edit a symbol: W lines, N pins, the canvas menu's Symbol attributes · Edit its symbol"],
  ]],
];

function HelpDialog() {
  return (
    <Modal title="Quick reference" wide="xl" ok="Close" cancel={false} onSubmit={close}>
      <div className="grid max-h-[70vh] grid-cols-2 gap-x-6 gap-y-3 overflow-auto pr-1 text-xs">
        {HELP.map(([section, rows]) => (
          <section key={section}>
            <h3 className="mb-1 font-medium text-text-strong">{section}</h3>
            <table className="w-full table-fixed">
              <tbody>
                {rows.map(([keys, what]) => (
                  <tr key={keys} className="border-b border-border-subtle align-top">
                    <td className="w-[38%] py-1 pr-2 font-mono text-2xs text-text-strong">{shortcut(keys)}</td>
                    <td className="py-1 text-text-secondary">{what}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        ))}
        <p className="col-span-2 text-2xs text-text-tertiary">More in the README: github.com/mprotogerakis/nomospice</p>
      </div>
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
        <Button type="button" className="self-start" onClick={() => void checkForUpdate(true)}>Check for updates</Button>
        <h3 className="font-medium text-text-strong">Built with</h3>
        {table(thirdParty.included)}
        <h3 className="font-medium text-text-strong">Used, but not included</h3>
        {table(thirdParty.external)}
        <p className="leading-5 text-text-tertiary">{thirdParty.notice}</p>
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
  const commitAt = (index: number) => (next: Item) => {
    if (JSON.stringify(next) !== JSON.stringify(items[index])) commit(items.map((x, i) => (i === index ? next : x)));
    close();
  };
  /** A directive: the simulation form if it holds an analysis the form writes back unchanged, else the directive forms. */
  const directive = (index: number, raw?: boolean, draft?: string) => {
    const it = items[index] as Extract<Item, { t: "text" }>;
    const line = analysisLine(it.text);
    if (!raw && line && analysisEditable(it.text)) return <SimDialog key={index} index={index} />;
    return <DirectiveDialog key={`${index}${raw || line ? "raw" : ""}`} item={draft === undefined ? it : { ...it, text: draft }} title="SPICE Directive" raw={raw || !!line} onDone={commitAt(index)} />;
  };
  switch (dialog.k) {
    case "edit": {
      const it = items[dialog.index];
      if (!it || it.t === "draw") return queueMicrotask(close), null; // (gone, or a drawing: nothing to edit; not left open, holding the hotkeys)
      // an alternative analysis (a comment of nothing but analyses: the tabs offer it) opens the form
      const lines = it.t === "text" ? it.text.split("\n") : [];
      if (it.t === "text" && !it.directive && !dialog.raw && !isSymbolDoc(useS.getState().path) && analysisOptions([it]).length && analysisEditableAt(lines, analysisLineIndex(lines, null, false))) return <SimDialog key={dialog.index} index={dialog.index} />;
      if (it.t === "text") return it.directive ? directive(dialog.index, dialog.raw, dialog.draft) : <DirectiveDialog key={dialog.index} item={dialog.draft === undefined ? it : { ...it, text: dialog.draft }} title="Comment" raw onDone={commitAt(dialog.index)} />;
      return <ItemForm key={dialog.index} item={it} title={it.t === "sym" ? `Edit ${it.attrs.InstName ?? it.sym}` : isSymbolDoc(useS.getState().path) ? "Pin" : "Net Name"} onDone={commitAt(dialog.index)} />;
    }
    case "new": {
      const at = dialog.at;
      const place = (it: Item) => (at && it.t !== "wire" && it.t !== "raw" && it.t !== "draw" /* (for the type: only texts and labels come here) */ ? (commit([...items, { ...it, x: at[0], y: at[1] }]), close()) : useS.setState({ dialog: null, tool: { k: "place", item: it } }));
      if (dialog.item.t === "text") return <DirectiveDialog item={dialog.item} title={dialog.item.directive ? "SPICE Directive" : "Comment"} raw={!dialog.item.directive} onDone={place} />;
      return <ItemForm item={dialog.item} title={isSymbolDoc(useS.getState().path) ? "Pin" : "Net Name"} onDone={place} />;
    }
    case "sim": { // the form also for an analysis it does not fully cover: it says so, and OK rewrites the line
      if (dialog.add) return <SimDialog kind0={dialog.add} />;
      const tab = currentTab(useS.getState()), i = analysisItem(items, tab); // what the shown result tab runs
      return <SimDialog key={i} index={i >= 0 ? i : undefined} of={tab} />;
    }
    case "parts": return <PartsDialog replace={dialog.replace} kind0={dialog.kind} libraries={dialog.libraries} />;
    case "trace": return <TraceDialog />;
    case "find": return <FindDialog />;
    case "yrange": return <YRangeDialog pane={dialog.pane} unit={dialog.unit} log={dialog.log} />;
    case "about": return <AboutDialog />;
    case "help": return <HelpDialog />;
    case "symattrs": return <SymbolAttrs />;
  }
});
