import { memo, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-dialog";
import type { Item, LibEntry, LibSource } from "./types";
import { addTrace, commit, loadSymbols, placePart, placeSymbol, refreshLibrarySymbols, useS } from "./store";
import { defOf, vexpr } from "./geom";
import { Button } from "./ui/button";
import { closeDialog, Field, FIELD, Modal, numHelp, TextField } from "./ui/dialog";
import { SourceEditor } from "./SourceEditor";
import { SimDialog } from "./SimDialog";
import { DirectiveDialog } from "./DirectiveDialog";
import { analysisLine, analysisRoundTrips, holdsAnalysis, tokens } from "./spice";
import { cn } from "@/lib/utils";
import thirdParty from "./third-party.json";
import { checkForUpdate } from "./updater";

const close = closeDialog;

/** Edits a part or a net label; `onDone` receives the result (commit in place, or start placing it).
 *  Comments and directives use DirectiveDialog. */
function ItemForm({ item, title, onDone }: { item: Item; title: string; onDone: (it: Item) => void }) {
  const lib = useS((s) => s.lib);
  const [it, setIt] = useState(item);
  const attr = (k: string) => (it.t === "sym" ? it.attrs[k] ?? "" : "");
  const setAttr = (k: string, v: string) => it.t === "sym" && setIt({ ...it, attrs: { ...it.attrs, [k]: v } });
  const done = () => {
    if (it.t === "sym") onDone({ ...it, attrs: Object.fromEntries(Object.entries(it.attrs).filter(([, v]) => v.trim())) });
    else if (it.t === "flag" && !it.name.trim()) close();
    else onDone(it);
  };
  const def = it.t === "sym" ? defOf(lib, it) : undefined;
  // independent sources (V, I; not the controlled E/G/F/H) get the waveform form
  const source = it.t === "sym" && /^[VI]$/i.test(def?.prefix ?? "");
  // R, C, L: show how SPICE reads a single value (1M is a milliohm, 10F ten femtofarad)
  const unit = ({ R: "Ω", C: "F", L: "H" } as Record<string, string>)[def?.prefix?.[0]?.toUpperCase() ?? ""];
  const valueHelp = unit && !/\s/.test(attr("Value").trim()) ? numHelp(attr("Value"), unit, (v) => setAttr("Value", v)) : null;
  return (
    <Modal title={title} onSubmit={done} wide={source ? "xl" : false}>
      {it.t === "sym" && (
        <>
          <div className="text-2xs text-text-tertiary">{def?.desc ?? it.sym} · {it.sym}</div>
          <TextField label="Instance name" value={attr("InstName")} set={(v) => setAttr("InstName", v)} />
          {source ? <SourceEditor attrs={it.attrs} setAttrs={(attrs) => setIt({ ...it, attrs })} current={def?.prefix?.[0]?.toUpperCase() === "I"} /> : (
            <>
              <TextField label="Value" value={attr("Value")} set={(v) => setAttr("Value", v)} autofocus unit={unit}
                {...(valueHelp ?? {})} hint={unit ? "e.g. 4.7k, 100n, 1Meg; LTspice extras like Rser=0.1 may follow" : undefined} />
              <TextField label="Value2" value={attr("Value2")} set={(v) => setAttr("Value2", v)} />
              <TextField label="SpiceLine" value={attr("SpiceLine")} set={(v) => setAttr("SpiceLine", v)} placeholder={def?.spice_line} />
            </>
          )}
        </>
      )}
      {it.t === "flag" && (
        <Field label="Net name" hint="0 = ground">
          <input data-autofocus className={FIELD} value={it.name} onChange={(e) => setIt({ ...it, name: e.target.value.replace(/\s/g, "_") })} />
        </Field>
      )}
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
  // braces: Chromium's (WebView2) scrollIntoView returns a Promise, which React would call as the cleanup
  useEffect(() => { listRef.current?.children[i]?.scrollIntoView({ block: "nearest" }); }, [i]);
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

/** The directive that holds the schematic's analysis (the toolbar's "Edit simulation command"). */
const analysisItem = (items: Item[]) => items.findIndex((x) => holdsAnalysis(x));

export const Dialogs = memo(function Dialogs() {
  const dialog = useS((s) => s.dialog);
  const items = useS((s) => s.items);
  if (!dialog) return null;
  const commitAt = (index: number) => (next: Item) => (commit(items.map((x, i) => (i === index ? next : x))), close());
  /** A directive: the simulation form if it holds an analysis the form writes back unchanged, else the directive forms. */
  const directive = (index: number, raw?: boolean, draft?: string) => {
    const it = items[index] as Extract<Item, { t: "text" }>;
    const line = analysisLine(it.text);
    if (!raw && line && analysisRoundTrips(line)) return <SimDialog key={index} index={index} />;
    return <DirectiveDialog key={`${index}${raw || line ? "raw" : ""}`} item={draft === undefined ? it : { ...it, text: draft }} title="SPICE Directive" raw={raw || !!line} onDone={commitAt(index)} />;
  };
  switch (dialog.k) {
    case "edit": {
      const it = items[dialog.index];
      if (!it) return null;
      if (it.t === "text") return it.directive ? directive(dialog.index, dialog.raw, dialog.draft) : <DirectiveDialog key={dialog.index} item={it} title="Comment" raw onDone={commitAt(dialog.index)} />;
      return <ItemForm key={dialog.index} item={it} title={it.t === "sym" ? `Edit ${it.attrs.InstName ?? it.sym}` : "Net Name"} onDone={commitAt(dialog.index)} />;
    }
    case "new": {
      const at = dialog.at;
      const place = (it: Item) => (at && it.t !== "wire" && it.t !== "raw" ? (commit([...items, { ...it, x: at[0], y: at[1] }]), close()) : useS.setState({ dialog: null, tool: { k: "place", item: it } }));
      if (dialog.item.t === "text") return <DirectiveDialog item={dialog.item} title={dialog.item.directive ? "SPICE Directive" : "Comment"} raw={!dialog.item.directive} onDone={place} />;
      return <ItemForm item={dialog.item} title="Net Name" onDone={place} />;
    }
    case "sim": {
      const i = analysisItem(items);
      return i >= 0 ? directive(i) : <SimDialog />;
    }
    case "parts": return <PartsDialog replace={dialog.replace} kind0={dialog.kind} />;
    case "trace": return <TraceDialog />;
    case "about": return <AboutDialog />;
  }
});
