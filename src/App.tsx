import { useEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { ask, message, open, save } from "@tauri-apps/plugin-dialog";
import {
  Code, Component, Copy, FilePlus, FlipHorizontal2, FolderOpen, Moon, MousePointer2, Move, Play, Redo2, RotateCw, Save,
  CircleHelp, Info, Plus, Scissors, X, ScrollText, SlidersHorizontal, Spline, Square, Sun, Tag, Type, Undo2, Zap,
} from "lucide-react";
import type { Item } from "./types";
import { Schematic } from "./Schematic";
import { Measurements, Waveform } from "./Waveform";
import { PoleZero } from "./PoleZero";
import { checkForUpdate, installUpdate, useUpdate } from "./updater";
import { Dialogs } from "./Dialogs";
import { Boundary } from "./Boundary";
import { analysesOf, closeTab, currentTab, dirOf, fileName, docId, isSymbolDoc, viewOf, loadDoc, loadSymbols, placeSymbol, plotKind, probeMode, redo, refreshGit, refreshNets, relToDoc, selectTab, simulate, undo, useS, useStatus } from "./store";
import { History } from "./History";
import { mirror, rotate } from "./geom";
import { ANALYSES, si, tokens } from "./spice";
import { copySelection, deleteSelection, duplicate, newItem, pasteClipboard, placeFlag, transform } from "./actions";
import { ContextMenu, openMenu } from "./Menu";
import { Toolbar, ToolbarButton, ToolbarSeparator, ToolbarSpacer } from "./ui/toolbar";
import { StatusBar, StatusSegment } from "./ui/status-bar";
import { initializeTheme, updateThemePreference, type ThemeMode } from "./lib/theme";
import { cn, keys, MAC } from "./lib/utils";

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

/** Netlists the app opens (as schematics of them); the core's is_netlist decides, this list offers. */
const NETLIST_EXT = ["cir", "net", "sp", "spi", "ckt"];
const NETLIST = new RegExp(`\\.(${NETLIST_EXT.join("|")})$`, "i");
const OPEN_FILTERS = [{ name: "Schematic, netlist or symbol", extensions: ["asc", ...NETLIST_EXT, "asy"] }, { name: "LTspice schematic", extensions: ["asc"] }, { name: "SPICE netlist", extensions: NETLIST_EXT }, { name: "Symbol", extensions: ["asy"] }];
/** What a document saves as: a symbol as a symbol, else a schematic. */
const saveAs = (symbol: boolean) => (symbol ? { ext: "asy", filters: [{ name: "Symbol", extensions: ["asy"] }] } : { ext: "asc", filters: [{ name: "LTspice schematic", extensions: ["asc"] }] });
/** Typing into a text field (a focused checkbox or button must not swallow the hotkeys). */
const typing = (e: KeyboardEvent) =>
  e.target instanceof HTMLElement && e.target.closest("input:not([type=checkbox]):not([type=radio]):not([type=button]),textarea,select,dialog") !== null;

/** Unsaved-changes guard for everything that closes the document (or quits): save, don't, or stay.
 *  (custom buttons answer with their label; some platforms with Yes/No) */
async function discardOk(): Promise<boolean> {
  const { dirty, path } = useS.getState();
  if (!dirty) return true;
  const name = fileName(path);
  const r = await message("Your changes are lost if you don't save them.", { title: `Save the changes to “${name}”?`, kind: "warning", buttons: { yes: "Save", no: "Don't Save", cancel: "Cancel" } });
  // (not written: stay; written, but changed meanwhile: asked again about those changes)
  if (r === "Save" || r === "Yes") return (await saveFile()) && (!useS.getState().dirty || discardOk());
  return r === "Don't Save" || r === "No";
}
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

/** The model library folders (app menu, ⌘/Ctrl+,): the component dialog's library view. */
const openLibraries = () => useS.setState({ dialog: { k: "parts", libraries: true } });

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
  // menu shortcuts bypass the keydown guards: with a dialog open, ⌘W closes the dialog (as Esc),
  // ⌘, waits (it would replace the dialog and lose its edits)
  listen("open-libraries", () => !useS.getState().dialog && openLibraries());
  // (⌘W: the document goes, the app stays, as an empty untitled schematic)
  listen("close-requested", () => (useS.getState().dialog ? useS.setState({ dialog: null }) : newFile()));
  // mirror unsaved work for exits we cannot intercept (Dock > Quit, logout, crashes)
  useS.subscribe((s, prev) => {
    if (s.items === prev.items && s.dirty === prev.dirty && s.path === prev.path) return;
    clearTimeout(mirrorTimer);
    mirrorTimer = setTimeout(() => (s.dirty ? recover("recovery_save", { doc: { path: s.path, items: s.items, unwritten: useS.getState().unwritten } }) : recover("recovery_clear")), 800);
  });
}

const fail = (what: string) => (e: unknown) => message(`${what}: ${e}`, { title: "nomospice", kind: "error" });

/** Open a schematic: `path`, or one picked in the file dialog. */
async function openFile(path?: string) {
  if (!(await discardOk())) return;
  if (path === undefined) {
    const picked = await open({ filters: OPEN_FILTERS, multiple: false });
    if (typeof picked !== "string") return;
    path = picked;
  }
  try {
    await openPath(path);
  } catch (e) {
    await fail(`Cannot open ${path}`)(e);
  }
}
/** Open a schematic, or a netlist as a new schematic of it: beside it (its .includes found from
 *  there), never saved over it: name.asc, or name-2.asc … if that is taken. */
async function openPath(path: string) {
  const [items, netlist] = await invoke<[Item[], boolean]>("open_asc", { path });
  if (!netlist) return loadDoc(items, path);
  const base = path.replace(/\.[^./\\]*$/, "");
  let asc = `${base}.asc`;
  for (let k = 2; await invoke<boolean>("path_exists", { path: asc }); k++) asc = `${base}-${k}.asc`;
  await loadDoc(items, asc, true); // (its first save asks where, offering this name: a file made there since is not overwritten unasked)
  useS.setState({ dirty: true });
}
let saveQueue: Promise<unknown> = Promise.resolve();

const MODEL_FILE = /\.(lib|sub|mod|inc|cir|sp|spi|bjt|dio|jft|mos)$/i; // (not .net: a whole circuit, LTspice's netlist of a schematic)
/** What dropping these files does (null: nothing). */
const dropAction = (paths: string[]) =>
  paths.some((p) => /\.asc$/i.test(p)) ? "Open schematic" : paths.some(isSymbolDoc) ? "Open symbol" : paths.some((p) => MODEL_FILE.test(p)) ? "Add as .include directive" : null;
/** Dropped files: a schematic opens, model files become `.include` directives (relative to the
 *  schematic's folder when inside it, as LTspice writes them). */
async function dropFiles(paths: string[]) {
  if (useS.getState().dialog) return;
  // (a dropped .cir is a model to include, mostly: a netlist opens by File → Open)
  const asc = paths.find((p) => /\.asc$/i.test(p)) ?? paths.find(isSymbolDoc);
  if (asc) return openFile(asc);
  const models = paths.filter((p) => MODEL_FILE.test(p));
  if (models.length) newItem({ t: "text", x: 0, y: 0, directive: true, text: models.map((p) => `.include "${relToDoc(p)}"`).join("\n") });
}

/** Save (as): true once the file is written. */
async function saveFile(as = false): Promise<boolean> {
  const prev = useS.getState().path;
  const { ext, filters } = saveAs(isSymbolDoc(prev));
  let path = prev;
  if (as || !path || path === useS.getState().unwritten) path = await save({ filters, defaultPath: path ?? `untitled.${ext}` });
  if (!path) return false;
  if (/\.as[cy]$/i.test(path) && isSymbolDoc(path) !== isSymbolDoc(prev ?? "")) {
    // (a schematic is no symbol, nor the other way round: written as one, the other's lines are lost)
    await message(`A ${isSymbolDoc(prev ?? "") ? "symbol saves as .asy" : "schematic saves as .asc"}.`, { title: "nomospice" });
    return false;
  }
  if (!path.toLowerCase().endsWith(`.${ext}`)) {
    // GTK save dialogs do not add the filter's extension, and checked for overwrite without it
    path += `.${ext}`;
    if ((await invoke<boolean>("path_exists", { path })) && !(await ask(`${fileName(path)} already exists. Replace it?`, { title: "nomospice", kind: "warning" }))) return false;
  }
  const items = useS.getState().items;
  // (a symbol holds drawing, pins, texts: parts or directives placed on it have no place in its file)
  const lost = isSymbolDoc(path) ? items.filter((it) => it.t === "sym" || (it.t === "text" && it.directive)).length : 0;
  if (lost && !(await ask(`A symbol holds no parts or SPICE directives: ${lost} of them on it ${lost > 1 ? "are" : "is"} not saved. Save the rest?`, { title: "nomospice", kind: "warning" }))) return false;
  const doc = docId();
  // another folder: symbols from next to the old file go along on request (else its parts lose
  // them). Asked before the path changes, so no run (live mode) meanwhile looks in the new folder.
  const [from, to] = [dirOf(prev), dirOf(path)], names = items.flatMap((it) => (it.t === "sym" ? [it.sym] : []));
  const found = prev && from && to && from !== to
    ? await invoke<[string, boolean][]>("folder_symbols", { from, to, names, copy: false, doc: prev }).catch(async (e) => (await fail("Cannot check the old folder's symbols")(e), []))
    : [];
  const [own, clash] = [found.filter(([, c]) => !c).map(([f]) => f), found.filter(([, c]) => c).map(([f]) => f)];
  // a different file of the same name over there: it would be used instead (and is not overwritten)
  const differ = clash.length ? `\n\nThe new folder has its own, different ${clash.join(", ")}: the schematic will use ${clash.length > 1 ? "those" : "that one"} there.` : "";
  // decided now (Cancel stops the Save As), copied once the schematic is written (a failed save
  // leaves no copies behind); custom buttons answer with their label
  let carry = false;
  if (own.length) {
    const r = await message(`The schematic uses ${own.some((f) => /\.asc$/i.test(f)) ? "symbols and block schematics" : "symbols"} from its old folder: ${own.join(", ")}. Copy them to the new folder?${differ}`, { title: "nomospice", buttons: { yes: "Copy", no: "Don't copy", cancel: "Cancel" } });
    if (r === "Cancel") return false;
    carry = r === "Copy" || r === "Yes";
  }
  if (!own.length && clash.length) await message(differ.trim(), { title: "nomospice" });
  if (doc !== docId()) return false;
  try {
    // saves are written in the order they were requested: a newer snapshot never loses to an older one
    const write = (saveQueue = saveQueue.catch(() => {}).then(() => invoke("save_asc", { path, items })));
    await write;
    if (doc !== docId()) return false; // (written, but another document is open now: not the one asked about)
    if (carry) await invoke("folder_symbols", { from, to, names, copy: true, doc: prev }).catch(fail("Cannot copy all symbols"));
    const moved = path !== useS.getState().path; // relative .include paths now resolve elsewhere
    // another folder: includes and folder symbols resolve elsewhere, for every tab's results
    const tabs = Object.fromEntries(Object.entries(useS.getState().tabs).map(([k, t]) => [k, { ...t, stale: true }]));
    useS.setState({ path, dirty: useS.getState().items !== items, ...(moved && { tabs, ...(useS.getState().sim && { stale: true }) }) });
    if (moved) void loadSymbols(useS.getState().items).catch(() => {}).finally(refreshNets); // the new folder's symbols
    useS.setState({ unwritten: null }); // (a file now: saved without asking from here on)
    refreshGit();
    return true;
  } catch (e) {
    await fail(`Cannot save ${path}`)(e);
    return false;
  }
}
async function newFile() {
  if (await discardOk()) await loadDoc([], null);
}
async function openExample(items: Item[]) {
  if (await discardOk()) await loadDoc(items, null);
}
/** A new symbol to draw: its first save asks where (beside the schematics that place it). */
async function newSymbol() {
  if (!(await discardOk())) return;
  // (a name no file there has: the new symbol is no other one's)
  // (in the folder's own separator; a root's ends in one)
  const dir = dirOf(useS.getState().path), sep = dir?.includes("\\") ? "\\" : "/";
  const base = dir ? `${dir.endsWith(sep) ? dir : dir + sep}untitled` : "untitled";
  let path = `${base}.asy`;
  for (let k = 2; dir && (await invoke<boolean>("path_exists", { path })); k++) path = `${base}-${k}.asy`;
  await loadDoc([{ t: "raw", line: "SymbolType CELL" }, { t: "raw", line: "SYMATTR Prefix X" }], path, true);
}

/** A block's schematic in place of this one (`up`: the one above); the way back is kept. */
async function openSheet(path: string, up = false) {
  if (!(await discardOk())) return;
  // (after the save it may have asked for: where it went; a never written document has no way back)
  const { path: from, parents, unwritten } = useS.getState();
  try {
    await openPath(path);
    useS.setState({ parents: up ? parents.slice(0, -1) : from && from !== unwritten ? [...parents, from] : [] });
  } catch (e) {
    await fail(`Cannot open ${path}`)(e);
  }
}
/** This schematic's block symbol (`name.asy` beside it): placed in another schematic of the folder,
 *  it stands for this one, its port labels the pins. */
async function blockSymbol() {
  // (saved first: a block runs the file, whose labels its pins must match)
  if (useS.getState().dirty && !(await saveFile())) return;
  const { path, unwritten, items } = useS.getState();
  if (!path || path === unwritten) return void message("Save the schematic first: its symbol goes beside it.", { title: "nomospice" });
  const name = fileName(path).replace(/\.asc$/i, "");
  try {
    const there = await invoke<string | null>("block_symbol", { path, items, replace: false });
    if (there) {
      if (!(await ask(`${there} already exists. Replace it with a new block symbol?`, { title: "nomospice", kind: "warning" }))) return;
      await invoke("block_symbol", { path, items, replace: true });
    }
  } catch (e) {
    return void message(String(e), { title: "No block symbol", kind: "warning" });
  }
  await loadSymbols(useS.getState().items, undefined, { force: true }).catch(() => {});
  await message(`The block symbol ${name} is written. Place it (Component, ${name}) in a schematic in this folder: it stands for this one, its port labels are the pins.`, { title: "nomospice" });
}


let booted = false;

export function App() {
  const s = useS(useShallow((x) => ({
    hasItems: x.items.length > 0, sim: x.sim, tool: x.tool, selCount: x.sel.length, canUndo: x.past.length > 0, canRedo: x.future.length > 0,
    running: x.running, live: x.live, stale: x.stale, panel: x.panel, path: x.path, dirty: x.dirty, warnings: x.nets?.warnings, git: x.git, parents: x.parents,
  })));
  const [mode, setMode] = useState<ThemeMode>(() => initializeTheme().mode);
  const [split, setSplit] = useState(0.45);
  const [drop, setDrop] = useState<string | null>(null); // what releasing the dragged files would do

  useEffect(() => {
    const t = setTimeout(() => void checkForUpdate(), 4000); // a newer release on GitHub? (quietly, after startup)
    return () => clearTimeout(t);
  }, []);

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
  const view = viewOf(s.sim);
  const pz = view === "pz";
  const showWave = view === "plot" || pz;

  useEffect(() => {
    if (booted) return; // StrictMode mounts twice in dev: restore/open/run exactly once
    booted = true;
    (async () => {
      await loadSymbols();
      setExamples(await invoke<[string, Item[]][]>("examples"));
      const args = await invoke<string[]>("cli_args");
      const file = args.find((a) => /\.asc$/i.test(a) || isSymbolDoc(a) || NETLIST.test(a));
      const rec = await invoke<{ path: string | null; items: Item[]; unwritten?: string | null } | null>("recovery_take");
      // an empty document is a valid snapshot too (the user deleted everything)
      const restore = Array.isArray(rec?.items) && (await ask(`Restore unsaved changes${rec.path ? ` to ${rec.path}` : ""} from the last session?`, { title: "nomospice", kind: "info" }));
      await invoke("recovery_resolve", { keep: restore }).catch(() => {}); // restore: becomes our recovery file
      if (restore) {
        // (a document never written, an opened netlist's .asc: its first save asks, as it would have;
        // as the snapshot says, else by whether its file is there)
        const unwritten = !!rec!.path && (rec!.unwritten === rec!.path || !(await invoke<boolean>("path_exists", { path: rec!.path })));
        await loadDoc(rec!.items, rec!.path, unwritten);
        useS.setState({ dirty: true });
        if (file) await message(`${file} was not opened because the previous session was restored.`, { title: "nomospice" });
        return;
      }
      try {
        if (file) await openPath(file);
        if (args.includes("--run")) await simulate();
      } catch (e) {
        await fail(`Cannot open ${file}`)(e);
      }
    })();
  }, []);

  useEffect(() => {
    // (from the schematic's menus) a block's schematic, the one above, a block symbol of this one
    const sheet = (e: Event) => void openSheet((e as CustomEvent<string>).detail);
    const up = () => void (useS.getState().parents.length && openSheet(useS.getState().parents.at(-1)!, true));
    const sym = () => void blockSymbol();
    const newSym = () => void newSymbol();
    window.addEventListener("nomospice:new-symbol", newSym);
    window.addEventListener("nomospice:sheet", sheet);
    window.addEventListener("nomospice:up", up);
    window.addEventListener("nomospice:block-symbol", sym);
    return () => (window.removeEventListener("nomospice:new-symbol", newSym), window.removeEventListener("nomospice:sheet", sheet), window.removeEventListener("nomospice:up", up), window.removeEventListener("nomospice:block-symbol", sym));
  }, []);

  useEffect(() => {
    const onFocus = () => refreshGit(); // commits made in a terminal
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  useEffect(() => {
    const name = fileName(s.path);
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
      if (e.repeat && mod && "sonwv".includes(k)) return e.preventDefault(); // held ⌘S/⌘O/⌘N/⌘V: once
      // deferred: a dialog opened by this key must not receive the same keystroke
      const run = (f: () => unknown) => (e.preventDefault(), setTimeout(f));
      if (mod) {
        if (k === "z") return run(() => (e.shiftKey ? redo() : undo()));
        if (k === "y") return run(redo);
        if (k === "r") return run(() => transform(rotate));
        if (k === "e") return run(() => transform(mirror));
        if (k === "a") return run(() => useS.setState({ sel: useS.getState().items.flatMap((it, i) => (it.t === "raw" ? [] : [i])) }));
        if (k === "d") return run(duplicate);
        // copy, cut, paste of items, by key (WebKit sends no copy event without a text selection);
        // selected text anywhere (the log, the netlist): its own copy
        const text = !!sel?.toString();
        if ((k === "c" || k === "x") && !text) return void (copySelection(k === "x") && e.preventDefault());
        if (k === "v") return e.preventDefault(), pasteClipboard(); // (a selection does not matter: typing does, see above)
        if (k === "s") return run(() => saveFile(e.shiftKey));
        if (k === "o") return run(openFile);
        if (k === "n") return run(newFile);
        if (k === "k") return run(() => useS.setState({ dialog: { k: "parts" } }));
        if (k === "f" && !e.shiftKey && !e.altKey) return run(() => useS.setState({ dialog: { k: "find" } }));
        if (k === ",") return run(openLibraries); // macOS: the app menu's item has it
        if (k === "w" && !MAC) return run(newFile); // Ctrl+W closes the document (macOS: File → Close takes ⌘W)
        if (k === "l") return run(togglePanel);
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
        // (F7/F8 as LTspice has them; on a Mac they are media keys unless fn is held: M too)
        // (F1; on a Mac a brightness key unless fn is held: ? too)
        f1: () => useS.setState({ dialog: { k: "help" } }), "?": () => useS.setState({ dialog: { k: "help" } }),
        f7: moveTool, m: moveTool, f8: () => useS.setState({ tool: { k: "select" } }),
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
    : tool.k === "paste" ? "Click to paste · Esc / right-click cancel"
    : tool.k === "delete" ? "Click items to delete · Esc done"
    : pz ? "Poles × and zeros ○ in the s-plane: hover for f₀ and damping · root locus in the panel on the right"
    : showWave && s.sim?.run?.plot.startsWith("noise") ? "Noise spectra: Add trace → onoise_spectrum / inoise_spectrum · totals in Results"
    : showWave && s.sim?.run?.plot.startsWith("sens") ? "Sensitivities: Add trace → a part name (e.g. r1) · right-click to edit"
    : tool.k === "move" ? `Move: drag the selection off the rest (unselected wires and labels stay) · Esc / F8: drag with wires again${probeMode(s.sim) ? " · click a wire or part to plot it" : ""}`
    : showWave ? "Click a wire to plot V · click a part to plot I · right-click to edit"
    : "Drag to move · shift-drag box select · double/right-click to edit · ⌘↵ run";

  const startSplit = (e: React.PointerEvent) => {
    const r = mainRef.current!.getBoundingClientRect(), max = Math.min(0.8, roomFor(e.currentTarget.previousElementSibling as HTMLElement) / r.height);
    const grab = e.clientY - e.currentTarget.getBoundingClientRect().top; // (where in the handle: no jump)
    drag(e, (ev) => setSplit(Math.min(max, Math.max(0.15, (ev.clientY - grab - r.top) / r.height))));
  };

  const run = s.sim?.run;
  const warnings = s.warnings ?? [];
  const errors = s.sim && !s.sim.run;

  const symbolDoc = isSymbolDoc(s.path); // (a symbol holds no parts, no ground)
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
          <ToolbarButton label="Run" hotkey="⌘↵" icon={<Play />} disabled={symbolDoc} onClick={simulate} className="text-status-success">Run</ToolbarButton>
        )}
        <ToolbarButton label="Edit simulation command" icon={<SlidersHorizontal />} onClick={() => useS.setState({ dialog: { k: "sim" } })} />
        <ToolbarButton label="Live: re-run on every edit" icon={<Zap />} pressable active={s.live} onClick={() => useS.setState({ live: !s.live })} />
        <ToolbarSeparator />
        <ToolbarButton label="Select (drag: wires follow)" hotkey="Esc / F8" icon={<MousePointer2 />} pressable active={tool.k === "select"} onClick={() => useS.setState({ tool: { k: "select" } })} />
        <ToolbarButton label="Move (wires stay)" hotkey="M / F7" icon={<Move />} pressable active={tool.k === "move"} onClick={moveTool} />
        <ToolbarButton label="Wire" hotkey="W / F3" icon={<Spline />} pressable active={tool.k === "wire"} onClick={() => useS.setState({ tool: { k: "wire" } })} />
        <ToolbarButton label="Ground" disabled={symbolDoc} hotkey="G" icon={G.gnd} pressable active={tool.k === "place" && tool.item.t === "flag" && tool.item.name === "0"} onClick={() => placeFlag("0")} />
        <ToolbarButton label="Net label" hotkey="N / F4" icon={<Tag />} onClick={() => newItem({ t: "flag", x: 0, y: 0, name: "" })} />
        <ToolbarButton label="Resistor" disabled={symbolDoc} hotkey="R" icon={G.res} pressable active={placing("res")} onClick={() => placeSymbol("res")} />
        <ToolbarButton label="Capacitor" disabled={symbolDoc} hotkey="C" icon={G.cap} pressable active={placing("cap")} onClick={() => placeSymbol("cap")} />
        <ToolbarButton label="Inductor" disabled={symbolDoc} hotkey="L" icon={G.ind} pressable active={placing("ind")} onClick={() => placeSymbol("ind")} />
        <ToolbarButton label="Diode" disabled={symbolDoc} hotkey="D" icon={G.diode} pressable active={placing("diode")} onClick={() => placeSymbol("diode")} />
        <ToolbarButton label="Voltage source" disabled={symbolDoc} hotkey="V" icon={G.volt} pressable active={placing("voltage")} onClick={() => placeSymbol("voltage")} />
        <ToolbarButton label="Component" disabled={symbolDoc} hotkey="P / F2 / ⌘K" icon={<Component />} onClick={() => useS.setState({ dialog: { k: "parts" } })} />
        <ToolbarSeparator />
        <ToolbarButton label="Comment" hotkey="T" icon={<Type />} onClick={() => newItem({ t: "text", x: 0, y: 0, text: "", directive: false })} />
        <ToolbarButton label="SPICE directive" hotkey="S" icon={<Code />} onClick={() => newItem({ t: "text", x: 0, y: 0, text: "", directive: true })} />
        <ToolbarSeparator />
        <ToolbarButton label="Rotate" hotkey="⌃R" icon={<RotateCw />} onClick={() => transform(rotate)} />
        <ToolbarButton label="Mirror" hotkey="⌃E" icon={<FlipHorizontal2 />} onClick={() => transform(mirror)} />
        <ToolbarButton label="Duplicate" hotkey="⌘D / F6" icon={<Copy />} disabled={s.selCount !== 1} onClick={duplicate} />
        <ToolbarButton label="Delete" hotkey="Del / F5" icon={<Scissors />} pressable active={tool.k === "delete"} onClick={() => (s.selCount ? deleteSelection() : useS.setState({ tool: { k: "delete" } }))} />
        <ToolbarSpacer />
        <ToolbarButton label="Log & netlist" hotkey="⌘L" icon={<ScrollText />} pressable active={!!s.panel} onClick={togglePanel} />
        <ToolbarButton label="Toggle theme" icon={mode === "dark" ? <Sun /> : <Moon />} onClick={() => setMode(updateThemePreference(mode === "dark" ? "light" : "dark").mode)} />
        <ToolbarButton label="Quick reference" hotkey="? / F1" icon={<CircleHelp />} onClick={() => useS.setState({ dialog: { k: "help" } })} />
        <ToolbarButton label="About & licenses" icon={<Info />} onClick={() => useS.setState({ dialog: { k: "about" } })} />
      </Toolbar>

      <AnalysisTabs />{/* outside mainRef: the splitter measures from the waveform pane's top */}
      <div ref={mainRef} className="flex min-h-0 flex-1 flex-col">
        {showWave && (
          <>
            <div style={{ height: `${split * 100}%` }} className="min-h-0">{pz ? <PoleZero run={s.sim!.run!} /> : <Waveform key={`${mode}:${plotKind(s.sim)}`} />}</div>
            <div onPointerDown={startSplit} className="h-[5px] shrink-0 cursor-row-resize touch-none select-none border-y border-border bg-surface-panel-head hover:bg-selection/40" />
          </>
        )}
        {/* the schematic keeps a share: when the panes need more (a smaller window, the plot opening
            next to a tall panel) they give way (flex-shrink), the bottom panel first */}
        <div data-fill style={{ minHeight: `${FILL_MIN * 100}%` }} className="relative flex-1">
          <Schematic />
          {!s.hasItems && (tool.k === "select" || tool.k === "move") && (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
              <div className="pointer-events-auto max-h-full w-[440px] overflow-auto rounded-float border border-border bg-surface-panel p-4 shadow-float">
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
        {s.parents.length > 0 && (
          <StatusSegment onClick={() => window.dispatchEvent(new Event("nomospice:up"))} title={`Back to ${s.parents.at(-1)}, which this schematic is a block of`}>
            ↑ {fileName(s.parents.at(-1)!)}
          </StatusSegment>
        )}
        <StatusSegment flex sans>{keys(hint)}</StatusSegment>
        {s.git && (
          <StatusSegment onClick={() => useS.setState({ panel: "history" })} title="Version history (git)">
            {s.git.repo ? <>⎇ {s.git.branch}{s.git.file !== "clean" && <span className={s.git.file === "modified" ? "text-status-warning" : "text-text-tertiary"}> · {s.git.file}</span>}</> : "no git"}
          </StatusSegment>
        )}
        <CursorStatus />
        <UpdateStatus />
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

/** One tab per analysis in the schematic (commented alternatives too); each keeps its own results,
 *  traces, zoom and cursors. "+" adds an analysis; double-click edits the tab's. */
function AnalysisTabs() {
  const items = useS((s) => s.items), tab = useS((s) => s.tab), running = useS((s) => s.running);
  const opts = analysesOf(items); // (cached per items array)
  if (!opts.length) return null;
  const cur = currentTab({ tab, items }, opts);
  const name = (k: string) => ANALYSES.find(([a]) => a === k)?.[1] ?? k;
  const more = ANALYSES.filter(([k]) => !opts.some((o) => o.kind === k));
  return (
    <div role="tablist" className="flex shrink-0 items-end gap-0.5 border-b border-border bg-surface-panel-head px-2 pt-1 text-xs">
      {opts.map((o) => (
        <div key={o.kind} className={cn("group -mb-px flex items-center rounded-t-control border", o.kind === cur ? "border-border border-b-surface-panel bg-surface-panel font-medium text-text-strong" : "border-transparent text-text-secondary hover:text-text-strong")}>
          <button
            type="button" role="tab" aria-selected={o.kind === cur} disabled={running && o.kind !== cur}
            title={`${o.line}${o.active ? "" : "\nAlternative analysis (a comment in the schematic)"}\nDouble-click to edit`}
            onClick={() => selectTab(o.kind)} onDoubleClick={() => useS.setState({ dialog: { k: "sim" } })}
            className={cn("py-1 pl-3 disabled:opacity-50", opts.length > 1 ? "pr-1" : "pr-3")}
          >
            {name(o.kind)}
          </button>
          {opts.length > 1 && (
            // (the analysis leaves the schematic; ⌘Z brings it back)
            <button type="button" disabled={running} aria-label={`Close ${name(o.kind)}`} title={`Remove this analysis from the schematic (${keys("⌘Z")} brings it back)`}
              onClick={() => closeTab(o.kind)}
              className={cn("mr-1 flex h-4 w-4 items-center justify-center rounded-control text-text-tertiary hover:bg-surface-hover hover:text-text-strong disabled:opacity-50", o.kind !== cur && "opacity-0 focus-visible:opacity-100 group-hover:opacity-100")}>
              <X className="h-3 w-3" />
            </button>
          )}
        </div>
      ))}
      {!!more.length && (
        <button type="button" disabled={running} title="Add an analysis (it gets its own result tab)" aria-label="Add an analysis" className="disabled:opacity-50 mb-0.5 rounded-control px-1.5 py-0.5 text-text-secondary hover:bg-surface-hover hover:text-text-strong"
          onClick={(e) => openMenu(e.clientX, e.clientY, more.map(([k, label]) => ({ label, run: () => useS.setState({ dialog: { k: "sim", add: k } }) })))}>
          <Plus className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}

/** "Update x.y.z" when a newer release is out; click to install (or the download progress). */
function UpdateStatus() {
  const { update, busy } = useUpdate();
  if (!update) return null;
  return (
    <StatusSegment className="text-status-info" onClick={() => void installUpdate()} title={`nomospice ${update.version} is available: click to install and restart`}>
      {busy || `Update ${update.version}`}
    </StatusSegment>
  );
}

function CursorStatus() {
  const { x, y, net, probe, zoom } = useStatus();
  return (
    <>
      {probe && <StatusSegment className="text-text-strong">click: plot {probe}</StatusSegment>}
      {net && <StatusSegment className="text-text-strong">net {net}</StatusSegment>}
      <StatusSegment>{x}, {y}</StatusSegment>
      <StatusSegment>{Math.round(zoom * 100)}%</StatusSegment>
    </>
  );
}

const moveTool = () => useS.setState({ tool: { k: "move" } });

/** ⌘L: the bottom panel closed, or open again on the tab last shown (a run's results only while
 *  there are any). */
let lastPanel: Exclude<ReturnType<typeof useS.getState>["panel"], null> = "netlist";
useS.subscribe((s) => void (s.panel && (lastPanel = s.panel)));
const togglePanel = () => {
  const s = useS.getState();
  useS.setState({ panel: s.panel ? null : lastPanel === "op" && !s.sim?.run?.op.length ? "log" : lastPanel });
};

/** A drag on a splitter handle (left button): `move` gets the pointer (captured by the handle, so
 *  none once it is unmounted) until the button is up or the gesture cancelled; then `done`. */
function drag(e: React.PointerEvent, move: (ev: PointerEvent) => void, done?: () => void) {
  if (e.button !== 0) return;
  const el = e.currentTarget as HTMLElement, id = e.pointerId;
  el.setPointerCapture(id);
  // (not a second finger's; no button held: the release went elsewhere, e.g. another window)
  const mine = (ev: PointerEvent) => ev.pointerId === id && (ev.buttons ? move(ev) : end(ev));
  const end = (ev: PointerEvent) => {
    if (ev.pointerId !== id) return;
    if (el.hasPointerCapture(id)) el.releasePointerCapture(id); // (released elsewhere: not by itself)
    el.removeEventListener("pointermove", mine);
    window.removeEventListener("pointerup", end);
    window.removeEventListener("pointercancel", end);
    done?.();
  };
  el.addEventListener("pointermove", mine);
  window.addEventListener("pointerup", end);
  window.addEventListener("pointercancel", end);
}

/** The schematic's minimum share of the area below the toolbar; the bottom panel's minimum height
 *  and default share. */
const FILL_MIN = 0.25, PANEL_MIN = 80, PANEL_SHARE = 0.32;

/** How tall pane `el` may get by dragging (so its edge stays under the pointer): its height plus the
 *  room the schematic ([data-fill]) has above its share and, for the plot, the room the bottom panel
 *  ([data-panel], which shrinks first) has above its minimum. */
const roomFor = (el: HTMLElement) => {
  const box = el.parentElement!, fill = box.querySelector<HTMLElement>(":scope > [data-fill]")!, panel = box.querySelector<HTMLElement>(":scope > [data-panel]");
  return el.offsetHeight + Math.max(0, fill.offsetHeight - FILL_MIN * box.clientHeight) + (panel && panel !== el ? Math.max(0, panel.offsetHeight - PANEL_MIN) : 0);
};

/** ngspice's .tf results (Transfer_function, output_impedance_at_V(out), v1#Input_impedance) named
 *  for what they are, with units from the statement `.tf <output> <input source>`. */
function tfName(n: string, line: string): [string, string] | null {
  const [out = "the output", src = "the input"] = tokens(line).slice(1);
  const fromI = /^i/i.test(src), toI = /^i\(/i.test(out);
  if (/^transfer_function$/i.test(n)) return [`Gain ${out}/${src}`, fromI === toI ? "" : toI ? "S" : "Ω"];
  if (/^output_impedance/i.test(n)) return [`Output resistance at ${out}`, "Ω"];
  if (/#input_impedance$/i.test(n)) return [`Input resistance at ${src}`, "Ω"];
  return null;
}

function BottomPanel() {
  const { panel, sim, nets } = useS(useShallow((s) => ({ panel: s.panel, sim: s.sim, nets: s.nets })));
  const [simulated, setSimulated] = useState(false);
  const isOpPlot = !!sim?.run?.plot.startsWith("op");
  const phasors = !!sim?.run?.plot.startsWith("ac"); // a table from .ac means one frequency: phasors
  // .tf: its statement (output, input source) names the results (only that line: no re-render per edit)
  // (the line that ran, from the deck: an edit since does not relabel these numbers)
  const tf = useMemo(() => (sim?.run?.plot.startsWith("tf") ? /^[ \t]*(\.tf[ \t].*)$/im.exec(sim.simulated)?.[1].trim() ?? ".tf" : null), [sim]);
  const tabs: [typeof panel, string][] = [["log", "Log"], ["netlist", "Netlist"], ...(sim?.run?.op.length ? [["op", isOpPlot ? "Operating point" : phasors ? "Phasors" : "Results"] as [typeof panel, string]] : []), ["history", "History"]];
  // its share of the area (it scales with the window, like the plot's split): dragged at the top
  // edge, kept across sessions
  const [share, setShare] = useState(() => {
    const v = Number(localStorage.getItem("nomospice.panelShare"));
    return v > 0 && v < 1 ? v : PANEL_SHARE; // (more than fits: the panel gives way)
  });
  const ref = useRef<HTMLDivElement>(null);
  const resize = (e: React.PointerEvent) => {
    const el = ref.current!, room = el.parentElement!.clientHeight, bottom = el.getBoundingClientRect().bottom, max = roomFor(el);
    const grab = e.clientY - (bottom - el.offsetHeight); // (where in the handle: no jump)
    let f = 0;
    drag(e, (ev) => setShare((f = Math.max(PANEL_MIN, Math.min(max, bottom - ev.clientY + grab)) / room)),
      () => f && localStorage.setItem("nomospice.panelShare", String(f)));
  };
  return (
    <div ref={ref} data-panel style={{ height: `${share * 100}%`, minHeight: PANEL_MIN }} className="flex shrink-[1000] flex-col border-t border-border bg-surface-panel">
      <div onPointerDown={resize} onDoubleClick={() => (setShare(PANEL_SHARE), localStorage.removeItem("nomospice.panelShare"))} title="Drag to resize · double-click: default height"
        className="relative z-10 -mt-[3px] h-[5px] shrink-0 cursor-row-resize touch-none select-none hover:bg-selection/40" />
      <div className="flex h-[26px] shrink-0 items-center border-b border-border bg-surface-panel-head px-1">
        {tabs.map(([k, label]) => (
          <button key={k} onClick={() => useS.setState({ panel: k })}
            className={cn("h-full border-b-2 px-3 text-xs", panel === k ? "border-selection text-text-strong" : "border-transparent text-text-secondary hover:text-text-strong")}>
            {label}
          </button>
        ))}
        <button type="button" aria-label="Close the panel" title={`Close (${keys("⌘L")} opens it again)`} onClick={togglePanel}
          className="ml-auto flex h-5 w-5 items-center justify-center rounded-control text-text-tertiary hover:bg-surface-hover hover:text-text-strong">
          <X className="h-3.5 w-3.5" />
        </button>
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
        {panel === "op" && <Measurements xUnit="" />}
        {panel === "op" && tf && (
          <p className="mb-1 max-w-[70ch] font-sans text-2xs text-text-tertiary">
            Small-signal DC transfer (.tf): the circuit linearised at its operating point. The gain is how much the output changes per unit change of the input source; the output resistance is what a load at the output sees, the input resistance what the input source sees. No frequency dependence (capacitors open, inductors shorted): for that, use .ac.
          </p>
        )}
        {panel === "op" && (
          <table className="w-auto">
            <tbody>
              {sim?.run?.op.map(([full, v, im, unit0]) => {
                // ngspice's vector type gives the unit; @dev[...] and the like stay as written
                // (a .step run: `out [R=1k]`, named as without its step, the step after it)
                const [, n, step] = /^(.*?)( \[[^\]]*\])?$/.exec(full)!;
                const named = tf && tfName(n, tf);
                const unit = named ? named[1] : unit0;
                const name = (named ? named[0] : !isOpPlot && !phasors ? n : unit === "A" && n.endsWith("#branch") ? `I(${((b) => nets?.insts.find((x) => x.toLowerCase() === b) ?? b)(n.replace("#branch", ""))})` /* a source's, as drawn */ : unit === "V" && !/[@(]/.test(n) ? `V(${n})` : n) + (step ?? "");
                return (
                  <tr key={full}>
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
