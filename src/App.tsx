import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { ask, message, open, save } from "@tauri-apps/plugin-dialog";
import {
  Code, Component, Copy, FilePlus, FlipHorizontal2, FolderOpen, Moon, MousePointer2, Play, Redo2, RotateCw, Save,
  Info, Scissors, ScrollText, SlidersHorizontal, Spline, Square, Sun, Tag, Type, Undo2, Zap,
} from "lucide-react";
import type { Item } from "./types";
import { Schematic } from "./Schematic";
import { Waveform } from "./Waveform";
import { PoleZero } from "./PoleZero";
import { Dialogs } from "./Dialogs";
import { Boundary } from "./Boundary";
import { dirOf, docId, isOp, loadDoc, loadSymbols, placeSymbol, plotKind, redo, refreshGit, refreshNets, simulate, undo, useS, useStatus } from "./store";
import { History } from "./History";
import { currentLabel, mirror, rotate, si } from "./geom";
import { deleteSelection, duplicate, newItem, placeFlag, transform } from "./actions";
import { ContextMenu } from "./Menu";
import { Toolbar, ToolbarButton, ToolbarSeparator, ToolbarSpacer } from "./ui/toolbar";
import { StatusBar, StatusSegment } from "./ui/status-bar";
import { initializeTheme, updateThemePreference, type ThemeMode } from "./lib/theme";
import { cn, keys } from "./lib/utils";

/** Phasor as magnitude ∠ phase in degrees, the way the lecture writes it (230 V ∠ −120.0°). */
const polar = (re: number, im: number, unit: string) => {
  const deg = (Math.atan2(im, re) * 180) / Math.PI;
  return `${si(Math.hypot(re, im), unit, 4)} ∠ ${(Math.abs(deg) < 0.05 ? 0 : deg).toFixed(1).replace("-", "−")}°`;
};

const glyph = (d: string) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
);
const G = {
  res: glyph("M2 12h4l1.5-4 3 8 3-8 3 8 1.5-4h4"),
  cap: glyph("M2 12h8M10 5v14M14 5v14M14 12h8"),
  ind: glyph("M2 12h2.5a2.5 2.5 0 0 1 5 0a2.5 2.5 0 0 1 5 0a2.5 2.5 0 0 1 5 0H22"),
  diode: glyph("M2 12h6M16 12h6M8 6v12l8-6zM16 6v12"),
  gnd: glyph("M12 3v9M4 12h16l-8 9z"),
  volt: glyph("M12 2v3M12 19v3M12 5a7 7 0 1 0 0 14a7 7 0 1 0 0-14M9 12c1-2.5 2-2.5 3 0s2 2.5 3 0"),
};

const FILTERS = [{ name: "LTspice schematic", extensions: ["asc"] }];
/** Typing into a text field (a focused checkbox or button must not swallow the hotkeys). */
const typing = (e: KeyboardEvent) =>
  e.target instanceof HTMLElement && e.target.closest("input:not([type=checkbox]):not([type=radio]):not([type=button]),textarea,select,dialog") !== null;

/** Unsaved-changes guard for everything that replaces the document or quits. */
const discardOk = async () =>
  !useS.getState().dirty || (await ask("Discard unsaved changes?", { title: "nomospice", kind: "warning" }));
/** Recovery mirror: debounce timer and the last save in flight (a clear must come after it). */
let mirrorTimer: ReturnType<typeof setTimeout> | undefined;
let mirrorWrite: Promise<unknown> = Promise.resolve();
const recover = (cmd: "recovery_save" | "recovery_clear", args = {}) =>
  (mirrorWrite = mirrorWrite.then(() => invoke(cmd, args)).catch(() => {}));

/** Leaving for good: the recovery copy goes too (loads clear it via the dirty -> false mirror). */
const leave = async () => {
  if (!(await discardOk())) return false;
  clearTimeout(mirrorTimer);
  await recover("recovery_clear");
  return true;
};

/** Installed once, outside React: quitting must keep working even if the UI tree crashed. */
export function installGuards() {
  // a mouse click must not leave focus on a button (Windows/Linux webviews focus it): Space/Enter
  // would then "click" it again instead of reaching the app hotkeys (e.g. Run turned Halt).
  // Keyboard (Tab) focus is unaffected; dialogs keep normal focus handling.
  document.addEventListener("mousedown", (e) => {
    const t = e.target as Element | null;
    if (t?.closest?.("button, input[type=checkbox], input[type=radio]") && !t.closest("dialog")) e.preventDefault();
  });
  getCurrentWindow().onCloseRequested(async (e) => {
    if (!(await leave())) e.preventDefault();
  });
  listen("quit-requested", async () => (await leave()) && invoke("quit"));
  // mirror unsaved work for exits we cannot intercept (Dock > Quit, logout, crashes)
  useS.subscribe((s, prev) => {
    if (s.items === prev.items && s.dirty === prev.dirty && s.path === prev.path) return;
    clearTimeout(mirrorTimer);
    mirrorTimer = setTimeout(() => (s.dirty ? recover("recovery_save", { doc: { path: s.path, items: s.items } }) : recover("recovery_clear")), 800);
  });
}

const fail = (what: string) => (e: unknown) => message(`${what}: ${e}`, { title: "nomospice", kind: "error" });

/** Open a schematic: `path`, or one picked in the file dialog. */
async function openFile(path?: string) {
  if (!(await discardOk())) return;
  if (path === undefined) {
    const picked = await open({ filters: FILTERS, multiple: false });
    if (typeof picked !== "string") return;
    path = picked;
  }
  try {
    await loadDoc(await invoke<Item[]>("open_asc", { path }), path);
  } catch (e) {
    await fail(`Cannot open ${path}`)(e);
  }
}
let saveQueue: Promise<unknown> = Promise.resolve();

const MODEL_FILE = /\.(lib|sub|mod|inc|cir|sp|spi|bjt|dio|jft|mos)$/i;
/** What dropping these files does (null: nothing). */
const dropAction = (paths: string[]) =>
  paths.some((p) => /\.asc$/i.test(p)) ? "Open schematic" : paths.some((p) => MODEL_FILE.test(p)) ? "Add as .include directive" : null;
/** Dropped files: a schematic opens, model files become `.include` directives (relative to the
 *  schematic's folder when inside it, as LTspice writes them). */
async function dropFiles(paths: string[]) {
  if (useS.getState().dialog) return;
  const asc = paths.find((p) => /\.asc$/i.test(p));
  if (asc) return openFile(asc);
  const dir = dirOf(useS.getState().path);
  const rel = (p: string) => (dir && (p.startsWith(`${dir}/`) || p.startsWith(`${dir}\\`)) ? p.slice(dir.length + 1) : p);
  const models = paths.filter((p) => MODEL_FILE.test(p));
  if (models.length) newItem({ t: "text", x: 0, y: 0, directive: true, text: models.map((p) => `.include "${rel(p)}"`).join("\n") });
}

async function saveFile(as = false) {
  let path = useS.getState().path;
  if (as || !path) path = await save({ filters: FILTERS, defaultPath: path ?? "untitled.asc" });
  if (!path) return;
  if (!/\.asc$/i.test(path)) {
    // GTK save dialogs do not add the filter's extension, and checked for overwrite without it
    path += ".asc";
    const name = path.split(/[\\/]/).pop();
    if ((await invoke<boolean>("path_exists", { path })) && !(await ask(`${name} already exists. Replace it?`, { title: "nomospice", kind: "warning" }))) return;
  }
  const items = useS.getState().items;
  const doc = docId();
  try {
    // saves are written in the order they were requested: a newer snapshot never loses to an older one
    const write = (saveQueue = saveQueue.catch(() => {}).then(() => invoke("save_asc", { path, items })));
    await write;
    if (doc !== docId()) return; // another document was loaded meanwhile
    const moved = path !== useS.getState().path; // relative .include paths now resolve elsewhere
    useS.setState({ path, dirty: useS.getState().items !== items, ...(moved && useS.getState().sim && { stale: true }) });
    if (moved) refreshNets();
    refreshGit();
  } catch (e) {
    await fail(`Cannot save ${path}`)(e);
  }
}
async function newFile() {
  if (await discardOk()) await loadDoc([], null);
}
async function openExample(items: Item[]) {
  if (await discardOk()) await loadDoc(items, null);
}


let booted = false;

export function App() {
  const s = useS(useShallow((x) => ({
    hasItems: x.items.length > 0, sim: x.sim, tool: x.tool, selCount: x.sel.length, canUndo: x.past.length > 0, canRedo: x.future.length > 0,
    running: x.running, live: x.live, stale: x.stale, panel: x.panel, path: x.path, dirty: x.dirty, warnings: x.nets?.warnings, git: x.git,
  })));
  const [mode, setMode] = useState<ThemeMode>(() => initializeTheme().mode);
  const [split, setSplit] = useState(0.45);
  const [drop, setDrop] = useState<string | null>(null); // what releasing the dragged files would do

  useEffect(() => {
    // files dragged in from the Finder / Explorer (Tauri's native drag & drop)
    const un = getCurrentWebview().onDragDropEvent(({ payload: p }) => {
      if (p.type === "enter") setDrop(dropAction(p.paths));
      else if (p.type === "leave") setDrop(null);
      else if (p.type === "drop") (setDrop(null), void dropFiles(p.paths));
    });
    return () => void un.then((f) => f());
  }, []);
  const [examples, setExamples] = useState<[string, Item[]][]>([]);
  const mainRef = useRef<HTMLDivElement>(null);
  // pole-zero results have no scale (a table) but get their own view: the s-plane
  const pz = !!s.sim?.run?.plot.startsWith("pz");
  const showWave = !!s.sim?.run && (!isOp(s.sim) || pz);

  useEffect(() => {
    if (booted) return; // StrictMode mounts twice in dev: restore/open/run exactly once
    booted = true;
    (async () => {
      await loadSymbols();
      setExamples(await invoke<[string, Item[]][]>("examples"));
      const args = await invoke<string[]>("cli_args");
      const file = args.find((a) => a.toLowerCase().endsWith(".asc"));
      const rec = await invoke<{ path: string | null; items: Item[] } | null>("recovery_take");
      // an empty document is a valid snapshot too (the user deleted everything)
      const restore = Array.isArray(rec?.items) && (await ask(`Restore unsaved changes${rec.path ? ` to ${rec.path}` : ""} from the last session?`, { title: "nomospice", kind: "info" }));
      await invoke("recovery_resolve", { keep: restore }).catch(() => {}); // restore: becomes our recovery file
      if (restore) {
        await loadDoc(rec!.items, rec!.path);
        useS.setState({ dirty: true });
        if (file) await message(`${file} was not opened because the previous session was restored.`, { title: "nomospice" });
        return;
      }
      try {
        if (file) await loadDoc(await invoke<Item[]>("open_asc", { path: file }), file);
        if (args.includes("--run")) await simulate();
      } catch (e) {
        await fail(`Cannot open ${file}`)(e);
      }
    })();
  }, []);

  useEffect(() => {
    const onFocus = () => refreshGit(); // commits made in a terminal
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  useEffect(() => {
    const name = s.path?.split(/[\\/]/).pop() ?? "untitled.asc";
    getCurrentWindow().setTitle(`${s.dirty ? "● " : ""}${name} — nomospice`).catch(() => {});
  }, [s.path, s.dirty]);

  // Live mode: re-simulate shortly after every edit (and again if edits landed during a run)
  useEffect(() => {
    if (!s.live || !s.stale || s.running || !s.hasItems) return;
    const t = setTimeout(simulate, 350);
    return () => clearTimeout(t);
  }, [s.live, s.hasItems, s.stale, s.running]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (typing(e) || useS.getState().dialog || useS.getState().menu) return;
      const modKey = e.metaKey || e.ctrlKey;
      // plain Space/Enter on a focused button or checkbox operate that control (⌘↵ still runs)
      if (!modKey && (e.key === " " || e.key === "Enter") && e.target instanceof HTMLElement && e.target.closest("button,input,a")) return;
      // with text selected in the log/netlist panel, ⌘A selects that panel's text and Delete is the text's
      const sel = window.getSelection();
      const anchor = sel?.anchorNode;
      const panelText = sel && !sel.isCollapsed ? (anchor instanceof Element ? anchor : anchor?.parentElement)?.closest("[data-selectable]") : null;
      if (panelText && modKey && e.key.toLowerCase() === "a") return e.preventDefault(), sel!.selectAllChildren(panelText);
      if (panelText && (e.key === "Delete" || e.key === "Backspace")) return;
      const mod = e.metaKey || e.ctrlKey, k = e.key.toLowerCase();
      if (e.repeat && mod && "son".includes(k)) return e.preventDefault(); // held ⌘S/⌘O/⌘N: once
      // deferred: a dialog opened by this key must not receive the same keystroke
      const run = (f: () => unknown) => (e.preventDefault(), setTimeout(f));
      if (mod) {
        if (k === "z") return run(() => (e.shiftKey ? redo() : undo()));
        if (k === "y") return run(redo);
        if (k === "r") return run(() => transform(rotate));
        if (k === "e") return run(() => transform(mirror));
        if (k === "a") return run(() => useS.setState({ sel: useS.getState().items.flatMap((it, i) => (it.t === "raw" ? [] : [i])) }));
        if (k === "d") return run(duplicate);
        if (k === "s") return run(() => saveFile(e.shiftKey));
        if (k === "o") return run(openFile);
        if (k === "n") return run(newFile);
        if (k === "k") return run(() => useS.setState({ dialog: { k: "parts" } }));
        if (k === "l") return run(() => useS.setState({ panel: useS.getState().panel ? null : "log" }));
        if (k === "enter") return run(simulate);
        return;
      }
      const map: Record<string, () => unknown> = {
        escape: () => (window.dispatchEvent(new Event("nomospice:cancel")), useS.setState({ sel: [] })),
        r: () => placeSymbol("res"), c: () => placeSymbol("cap"), l: () => placeSymbol("ind"), d: () => placeSymbol("diode"),
        v: () => placeSymbol("voltage"), i: () => placeSymbol("current"), g: () => placeFlag("0"),
        w: () => useS.setState({ tool: { k: "wire" }, sel: [] }), f3: () => useS.setState({ tool: { k: "wire" }, sel: [] }),
        n: () => newItem({ t: "flag", x: 0, y: 0, name: "" }), f4: () => newItem({ t: "flag", x: 0, y: 0, name: "" }),
        s: () => newItem({ t: "text", x: 0, y: 0, text: "", directive: true }),
        t: () => newItem({ t: "text", x: 0, y: 0, text: "", directive: false }),
        p: () => useS.setState({ dialog: { k: "parts" } }), f2: () => useS.setState({ dialog: { k: "parts" } }),
        delete: deleteSelection, backspace: deleteSelection,
        f5: () => (useS.getState().sel.length ? deleteSelection() : useS.setState({ tool: { k: "delete" } })),
        f6: duplicate, f9: e.shiftKey ? redo : undo,
        " ": () => window.dispatchEvent(new Event("nomospice:fit")),
      };
      if (map[k]) run(map[k]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const tool = s.tool;
  const placing = (name: string) => tool.k === "place" && tool.item.t === "sym" && tool.item.sym === name;
  const hint =
    tool.k === "wire" ? "Wire: click to start / add corner · ends on pins and wires · right-click cancels · Esc done"
    : tool.k === "place" ? "Click to place · ⌃R rotate · ⌃E mirror · Esc / right-click done"
    : tool.k === "delete" ? "Click items to delete · Esc done"
    : pz ? "Poles × and zeros ○ in the s-plane: hover for f₀ and damping · root locus in the panel on the right"
    : showWave && s.sim?.run?.plot.startsWith("noise") ? "Noise spectra: Add trace → onoise_spectrum / inoise_spectrum · totals in Results"
    : showWave && s.sim?.run?.plot.startsWith("sens") ? "Sensitivities: Add trace → a part name (e.g. r1) · right-click to edit"
    : showWave ? "Click a wire to plot V · click a part to plot I · right-click to edit"
    : "Drag to move · shift-drag box select · double/right-click to edit · ⌘↵ run";

  const startSplit = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const r = mainRef.current!.getBoundingClientRect();
      setSplit(Math.min(0.8, Math.max(0.15, (ev.clientY - r.top) / r.height)));
    };
    const up = () => (window.removeEventListener("pointermove", move), window.removeEventListener("pointerup", up));
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const run = s.sim?.run;
  const warnings = s.warnings ?? [];
  const errors = s.sim && !s.sim.run;

  return (
    <div className="flex h-full flex-col">
      <Toolbar className="gap-1 px-2">
        <span className="mr-3 font-mono text-sm font-medium tracking-tight text-text-strong">nomo<span className="text-selection">spice</span></span>
        <ToolbarButton label="New" hotkey="⌘N" icon={<FilePlus />} onClick={newFile} />
        <ToolbarButton label="Open" hotkey="⌘O" icon={<FolderOpen />} onClick={() => openFile()} />
        <ToolbarButton label="Save" hotkey="⌘S" icon={<Save />} onClick={() => saveFile()} />
        <ToolbarSeparator />
        <ToolbarButton label="Undo" hotkey="⌘Z" icon={<Undo2 />} disabled={!s.canUndo} onClick={undo} />
        <ToolbarButton label="Redo" hotkey="⇧⌘Z" icon={<Redo2 />} disabled={!s.canRedo} onClick={redo} />
        <ToolbarSeparator />
        {s.running ? (
          <ToolbarButton label="Halt" icon={<Square />} onClick={() => invoke("halt")} className="text-status-danger">Halt</ToolbarButton>
        ) : (
          <ToolbarButton label="Run" hotkey="⌘↵" icon={<Play />} onClick={simulate} className="text-status-success">Run</ToolbarButton>
        )}
        <ToolbarButton label="Edit simulation command" icon={<SlidersHorizontal />} onClick={() => useS.setState({ dialog: { k: "sim" } })} />
        <ToolbarButton label="Live: re-run on every edit" icon={<Zap />} pressable active={s.live} onClick={() => useS.setState({ live: !s.live })} />
        <ToolbarSeparator />
        <ToolbarButton label="Select" hotkey="Esc" icon={<MousePointer2 />} pressable active={tool.k === "select"} onClick={() => useS.setState({ tool: { k: "select" } })} />
        <ToolbarButton label="Wire" hotkey="W / F3" icon={<Spline />} pressable active={tool.k === "wire"} onClick={() => useS.setState({ tool: { k: "wire" } })} />
        <ToolbarButton label="Ground" hotkey="G" icon={G.gnd} pressable active={tool.k === "place" && tool.item.t === "flag" && tool.item.name === "0"} onClick={() => placeFlag("0")} />
        <ToolbarButton label="Net label" hotkey="N / F4" icon={<Tag />} onClick={() => newItem({ t: "flag", x: 0, y: 0, name: "" })} />
        <ToolbarButton label="Resistor" hotkey="R" icon={G.res} pressable active={placing("res")} onClick={() => placeSymbol("res")} />
        <ToolbarButton label="Capacitor" hotkey="C" icon={G.cap} pressable active={placing("cap")} onClick={() => placeSymbol("cap")} />
        <ToolbarButton label="Inductor" hotkey="L" icon={G.ind} pressable active={placing("ind")} onClick={() => placeSymbol("ind")} />
        <ToolbarButton label="Diode" hotkey="D" icon={G.diode} pressable active={placing("diode")} onClick={() => placeSymbol("diode")} />
        <ToolbarButton label="Voltage source" hotkey="V" icon={G.volt} pressable active={placing("voltage")} onClick={() => placeSymbol("voltage")} />
        <ToolbarButton label="Component" hotkey="P / F2 / ⌘K" icon={<Component />} onClick={() => useS.setState({ dialog: { k: "parts" } })} />
        <ToolbarSeparator />
        <ToolbarButton label="Comment" hotkey="T" icon={<Type />} onClick={() => newItem({ t: "text", x: 0, y: 0, text: "", directive: false })} />
        <ToolbarButton label="SPICE directive" hotkey="S" icon={<Code />} onClick={() => newItem({ t: "text", x: 0, y: 0, text: "", directive: true })} />
        <ToolbarSeparator />
        <ToolbarButton label="Rotate" hotkey="⌃R" icon={<RotateCw />} onClick={() => transform(rotate)} />
        <ToolbarButton label="Mirror" hotkey="⌃E" icon={<FlipHorizontal2 />} onClick={() => transform(mirror)} />
        <ToolbarButton label="Duplicate" hotkey="⌘D / F6" icon={<Copy />} disabled={s.selCount !== 1} onClick={duplicate} />
        <ToolbarButton label="Delete" hotkey="Del / F5" icon={<Scissors />} pressable active={tool.k === "delete"} onClick={() => (s.selCount ? deleteSelection() : useS.setState({ tool: { k: "delete" } }))} />
        <ToolbarSpacer />
        <ToolbarButton label="Log & netlist" hotkey="⌘L" icon={<ScrollText />} pressable active={!!s.panel} onClick={() => useS.setState({ panel: s.panel ? null : "netlist" })} />
        <ToolbarButton label="Toggle theme" icon={mode === "dark" ? <Sun /> : <Moon />} onClick={() => setMode(updateThemePreference(mode === "dark" ? "light" : "dark").mode)} />
        <ToolbarButton label="About & licenses" icon={<Info />} onClick={() => useS.setState({ dialog: { k: "about" } })} />
      </Toolbar>

      <div ref={mainRef} className="flex min-h-0 flex-1 flex-col">
        {showWave && (
          <>
            <div style={{ height: `${split * 100}%` }} className="min-h-0 shrink-0">{pz ? <PoleZero run={s.sim!.run!} /> : <Waveform key={`${mode}:${plotKind(s.sim)}`} />}</div>
            <div onPointerDown={startSplit} className="h-[5px] shrink-0 cursor-row-resize border-y border-border bg-surface-panel-head hover:bg-selection/40" />
          </>
        )}
        <div className="relative min-h-0 flex-1">
          <Schematic />
          {!s.hasItems && tool.k === "select" && (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
              <div className="pointer-events-auto w-[440px] rounded-float border border-border bg-surface-panel p-4 shadow-float">
                <div className="text-sm font-medium text-text-strong">Start a schematic</div>
                <p className="mt-1 text-xs text-text-secondary">
                  Place parts with <kbd>R</kbd> <kbd>C</kbd> <kbd>L</kbd> <kbd>D</kbd> <kbd>V</kbd> <kbd>G</kbd>, draw wires with <kbd>W</kbd>, add <kbd>S</kbd> a directive like <code>.tran 1m</code>, then run with {keys("⌘↵")}. Or open an example:
                </p>
                <div className="mt-3 grid grid-cols-2 gap-1.5">
                  {examples.map(([name, items]) => (
                    <button key={name} onClick={() => openExample(items)} className="rounded-control border border-border-control px-2 py-1.5 text-left font-mono text-xs text-text hover:bg-surface-hover hover:text-text-strong">
                      {name}.asc
                    </button>
                  ))}
                  <button onClick={() => openFile()} className="rounded-control border border-dashed border-border-control px-2 py-1.5 text-left text-xs text-text-secondary hover:bg-surface-hover">Open .asc…</button>
                </div>
              </div>
            </div>
          )}
        </div>
        {s.panel && <BottomPanel />}
      </div>

      <StatusBar>
        <StatusSegment flex sans>{keys(hint)}</StatusSegment>
        {s.git && (
          <StatusSegment onClick={() => useS.setState({ panel: "history" })} title="Version history (git)">
            {s.git.repo ? <>⎇ {s.git.branch}{s.git.file !== "clean" && <span className={s.git.file === "modified" ? "text-status-warning" : "text-text-tertiary"}> · {s.git.file}</span>}</> : "no git"}
          </StatusSegment>
        )}
        <CursorStatus />
        {warnings.length > 0 && (
          <StatusSegment className="text-status-warning" onClick={() => useS.setState({ panel: "netlist" })} title={warnings.join("\n")}>
            {warnings.length} warning{warnings.length > 1 ? "s" : ""}
          </StatusSegment>
        )}
        <StatusSegment
          className={cn((errors || run?.aborted) && "text-status-danger", s.running && "text-status-info")}
          onClick={() => useS.setState({ panel: "log" })}
        >
          {s.running ? "simulating…" : errors ? "simulation failed" : run ? `${run.plot}${run.aborted ? " (aborted, partial)" : ""}${s.stale ? " (stale)" : ""}` : "ngspice ready"}
        </StatusSegment>
      </StatusBar>
      <Boundary onReset={() => useS.setState({ dialog: null })}>
        <Dialogs />
      </Boundary>
      {drop && (
        <div className="pointer-events-none fixed inset-0 z-40 flex items-center justify-center bg-scrim">
          <div className="rounded-float border-2 border-dashed border-selection bg-surface-panel px-6 py-4 text-sm font-medium text-text-strong shadow-float">{drop}</div>
        </div>
      )}
      <ContextMenu />
    </div>
  );
}

function CursorStatus() {
  const { x, y, net, zoom } = useStatus();
  return (
    <>
      {net && <StatusSegment className="text-text-strong">net {net}</StatusSegment>}
      <StatusSegment>{x}, {y}</StatusSegment>
      <StatusSegment>{Math.round(zoom * 100)}%</StatusSegment>
    </>
  );
}

function BottomPanel() {
  const { panel, sim, nets } = useS(useShallow((s) => ({ panel: s.panel, sim: s.sim, nets: s.nets })));
  const [simulated, setSimulated] = useState(false);
  const isOpPlot = !!sim?.run?.plot.startsWith("op");
  const phasors = !!sim?.run?.plot.startsWith("ac"); // a table from .ac means one frequency: phasors
  const tabs: [typeof panel, string][] = [["log", "Log"], ["netlist", "Netlist"], ...(sim?.run?.op.length ? [["op", isOpPlot ? "Operating point" : phasors ? "Phasors" : "Results"] as [typeof panel, string]] : []), ["history", "History"]];
  return (
    <div className="flex h-[32%] min-h-[120px] shrink-0 flex-col border-t border-border bg-surface-panel">
      <div className="flex h-[26px] shrink-0 items-center border-b border-border bg-surface-panel-head px-1">
        {tabs.map(([k, label]) => (
          <button key={k} onClick={() => useS.setState({ panel: k })}
            className={cn("h-full border-b-2 px-3 text-xs", panel === k ? "border-selection text-text-strong" : "border-transparent text-text-secondary hover:text-text-strong")}>
            {label}
          </button>
        ))}
      </div>
      <div data-selectable className="min-h-0 flex-1 overflow-auto p-2 font-mono text-xs leading-[18px] select-text">
        {panel === "log" && (sim?.log.length
          ? sim.log.map((l, i) => <div key={i} className={cn("whitespace-pre-wrap", /error|fatal|singular/i.test(l) ? "text-status-danger" : /warning/i.test(l) ? "text-status-warning" : "text-text-secondary")}>{l}</div>)
          : <div className="text-text-tertiary">No simulation output yet.</div>)}
        {panel === "history" && <History />}
        {panel === "netlist" && (
          <>
            {nets?.warnings.map((w, i) => <div key={i} className="text-status-warning">⚠ {w}</div>)}
            {sim?.simulated && (
              <label className="mb-1 flex items-center gap-1.5 font-sans text-2xs text-text-tertiary">
                {/* blur: a label click focuses the box on Windows/Linux and Space would re-toggle it */}
                <input type="checkbox" checked={simulated} onChange={(e) => (setSimulated(e.target.checked), e.target.blur())} />
                as simulated (current sensors, inlined includes: the log's line numbers refer to this)
              </label>
            )}
            <pre className="text-text">{simulated && sim?.simulated ? sim.simulated : nets?.text ?? ""}</pre>
          </>
        )}
        {panel === "op" && (
          <table className="w-auto">
            <tbody>
              {sim?.run?.op.map(([n, v, im, unit]) => {
                // ngspice's vector type gives the unit; @dev[...] and the like stay as written
                const name = !isOpPlot && !phasors ? n : unit === "A" && n.endsWith("#branch") ? currentLabel(n.replace("#branch", "")) : unit === "V" && !/[@(]/.test(n) ? `V(${n})` : n;
                return (
                  <tr key={n}>
                    <td className="pr-6 text-text-secondary">{name}</td>
                    <td className="text-right text-text-strong">{phasors ? (n === "frequency" ? si(v, "Hz") : polar(v, im, unit)) : `${si(v, unit, 5)}${im ? ` ${im < 0 ? "−" : "+"} j${si(Math.abs(im), unit, 5)}` : ""}`}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
