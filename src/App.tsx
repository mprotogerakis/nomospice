import { useEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen } from "@tauri-apps/api/event";
import { ask, message, open, save } from "@tauri-apps/plugin-dialog";
import {
  Code, Component, Copy, FilePlus, FlipHorizontal2, FolderOpen, Moon, MousePointer2, Move, Play, Redo2, RotateCw, Save,
  CircleHelp, Info, Plus, Scissors, X, ScrollText, SlidersHorizontal, Spline, Square, Sun, Tag, Type, Undo2, Zap,
} from "lucide-react";
import type { Item } from "./types";
import { exportSchematic, Schematic } from "./Schematic";
import { Measurements, Waveform } from "./Waveform";
import { PoleZero } from "./PoleZero";
import { checkForUpdate, installUpdate, useUpdate } from "./updater";
import { Dialogs } from "./Dialogs";
import { Boundary } from "./Boundary";
import { analysesOf, closeTab, currentTab, dirOf, fileName, docId, isSymbolDoc, viewOf, loadDoc, loadSymbols, placeSymbol, plotKind, probeMode, redo, refreshGit, refreshLibrarySymbols, refreshNets, relToDoc, selectTab, simulate, undo, useS, useStatus } from "./store";
import { saveRaw } from "./svgExport";
import { History } from "./History";
import { mirror, rotate } from "./geom";
import { ANALYSES, si, tokens } from "./spice";
import { copySelection, deleteSelection, duplicate, installClipboardEvents, newItem, pasteClipboard, placeFlag, swallow, TEXT_FIELD, textField, transform } from "./actions";
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
/** Typing into a text field, or in a dialog (a focused checkbox or button must not swallow the hotkeys). */
const typing = (e: KeyboardEvent) => e.target instanceof HTMLElement && e.target.closest(`${TEXT_FIELD},select,dialog`) !== null;

/** Unsaved-changes guard for everything that closes the document (or quits): save, don't, or stay.
 *  (custom buttons answer with their label; some platforms with Yes/No) */
async function discardOk(): Promise<boolean> {
  const { dirty, path } = useS.getState();
  if (!dirty) return true;
  const name = fileName(path);
  prompting++;
  try {
    const r = await message("Your changes are lost if you don't save them.", { title: `Save the changes to “${name}”?`, kind: "warning", buttons: { yes: "Save", no: "Don't Save", cancel: "Cancel" } });
    // (not written: stay; written, but changed meanwhile: asked again about those changes)
    if (r === "Save" || r === "Yes") return (await saveFile()) && (!useS.getState().dirty || discardOk());
    return r === "Don't Save" || r === "No";
  } finally {
    prompting--;
  }
}
/** Asking about unsaved changes now (Quit waits for that answer instead of asking too). */
let prompting = 0;
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
  // Quit asks the windows in turn, in rounds (see quit_all): this one's unsaved changes, in front if
  // it has any; not again in the same Quit about the same ones (Don't Save) unless edited since
  let asking = false, settled: { no: number; items: unknown } | null = null, refused = -1; // (asking: asked again meanwhile)
  void getCurrentWebviewWindow().listen<number>("quit-ask", async ({ payload: no }) => {
    if (asking) return void invoke("quit_waiting"); // (asked again: still deciding)
    if (no === refused) return; // (asked again just before this window's Cancel ended that Quit)
    const s = useS.getState();
    // (asking already, closing it or saving: that answer first, the Quit waits for it, asking again)
    if (prompting) return void invoke("quit_waiting");
    if (!s.dirty || (settled?.no === no && s.items === settled.items)) return void invoke("quit_answer", { ok: true, asked: false });
    asking = true;
    let ok = false;
    try {
      await getCurrentWindow().setFocus().catch(() => {});
      ok = await discardOk();
      if (ok) settled = { no, items: useS.getState().items };
      else refused = no;
    } finally {
      asking = false;
      void invoke("quit_answer", { ok, asked: true });
    }
  });
  // the menu bar's commands, for this window when it is the focused one
  void getCurrentWebviewWindow().listen<string>("menu", (e) => command(e.payload));
  // Windows: WebView2's own browser keys never act, whatever the page does with the key: Ctrl+R
  // and F5 would reload the page and lose the schematic, Ctrl+P print, Ctrl+F find, Ctrl+N a window
  if (!MAC) window.addEventListener("keydown", (e) => void (((e.ctrlKey && e.key.length === 1 && "rpsonfgjuht".includes(e.key.toLowerCase())) || /^F(3|5|7|11|12)$/.test(e.key)) && e.preventDefault()), true);
  listen("libraries-changed", () => void refreshLibrarySymbols().catch(() => {}));
  // a symbol saved (by any window): this folder's schematic draws and netlists it anew
  listen<string>("symbols-changed", ({ payload }) => {
    const { items, path } = useS.getState();
    if (dirOf(payload) === dirOf(path)) void loadSymbols(items, undefined, { force: true }).catch(() => {}).finally(refreshNets);
  });
  installClipboardEvents();
  useS.subscribe((s, prev) => {
    // the file open here (opened again from another window: this one comes forward instead)
    if (s.path !== prev.path || s.unwritten !== prev.unwritten) resyncDoc();
    // mirror unsaved work for exits we cannot intercept (Dock > Quit, logout, crashes)
    if (s.items === prev.items && s.dirty === prev.dirty && s.path === prev.path) return;
    clearTimeout(mirrorTimer);
    mirrorTimer = setTimeout(() => (s.dirty ? recover("recovery_save", { doc: { path: s.path, items: s.items, unwritten: useS.getState().unwritten } }) : recover("recovery_clear")), 800);
  });
}

const fail = (what: string) => (e: unknown) => message(`${what}: ${e}`, { title: "nomospice", kind: "error" });

/** A window of its own, opened for `args` (a file, `--restore`, `--new-symbol <folder>`); none: an
 *  empty schematic. */
const newWindow = (args: string[] = []) => invoke("new_window", { args }).catch(fail("Cannot open a window"));
/** The window that has `path` open, brought to the front (`claim`: none has it, it is this one's
 *  from now on: one window opens it, even if two try at once). */
const docWindow = (path: string, focus = true, claim = false) => invoke<string | null>("doc_window", { path, focus, claim }).catch(() => null);
/** Whether another window has `path` open (brought to the front if `focus`). */
const elsewhere = async (path: string, focus = true) => ((at) => !!at && at !== getCurrentWindow().label)(await docWindow(path, focus));
/** The file this window has open, told again (a claim that came to nothing). */
const resyncDoc = () => ((s) => void invoke("doc_path", { path: s.path && s.path !== s.unwritten ? s.path : null }).catch(() => {}))(useS.getState());
/** This window holds nothing yet (untitled, untouched): what is opened goes here, not into a new one. */
const untouched = () => ((s) => !s.items.length && !s.dirty && !s.path)(useS.getState());

/** Open a schematic: `path`, or one picked in the file dialog. Open already: its window comes
 *  forward; else here if this window holds nothing yet, otherwise in a window of its own. */
async function openFile(path?: string) {
  if (path === undefined) {
    const picked = await open({ filters: OPEN_FILTERS, multiple: false });
    if (typeof picked !== "string") return;
    path = picked;
  }
  // (this window's own file: read again, as asked; another's: shown there; none: here, claimed)
  const here = untouched(), at = await docWindow(path, true, here);
  if (at && at !== getCurrentWindow().label) return;
  if (!at && !here) return void newWindow([path]);
  if (at && !(await discardOk())) return;
  try {
    await openPath(path);
  } catch (e) {
    resyncDoc();
    await fail(`Cannot open ${path}`)(e);
  }
}
/** `base.ext`, or `base-2.ext` … if that is taken: the name of no file there. */
async function freeName(base: string, ext: string) {
  let path = `${base}.${ext}`;
  for (let k = 2; await invoke<boolean>("path_exists", { path }); k++) path = `${base}-${k}.${ext}`;
  return path;
}

/** Open a schematic, or a netlist as a new schematic of it: beside it (its .includes found from
 *  there), never saved over it: name.asc, or name-2.asc … if that is taken. */
async function openPath(path: string) {
  const [items, netlist] = await invoke<[Item[], boolean]>("open_asc", { path });
  if (!netlist) return loadDoc(items, path);
  const asc = await freeName(path.replace(/\.[^./\\]*$/, ""), "asc");
  await loadDoc(items, asc, true); // (its first save asks where, offering this name: a file made there since is not overwritten unasked)
  useS.setState({ dirty: true });
}
let saveQueue: Promise<unknown> = Promise.resolve();

const MODEL_FILE = /\.(lib|sub|mod|inc|cir|sp|spi|bjt|dio|jft|mos)$/i; // (not .net: a whole circuit, LTspice's netlist of a schematic)
/** What dropping these files does (null: nothing). */
const dropAction = (paths: string[]) =>
  paths.some((p) => /\.asc$/i.test(p)) ? "Open schematic" : paths.some(isSymbolDoc) ? "Open symbol"
    : paths.some((p) => MODEL_FILE.test(p)) && !isSymbolDoc(useS.getState().path) ? "Add as .include directive" : null; // (a symbol holds no directives)
/** Dropped files: a schematic opens, model files become `.include` directives (relative to the
 *  schematic's folder when inside it, as LTspice writes them). */
async function dropFiles(paths: string[]) {
  if (useS.getState().dialog) return;
  // (a dropped .cir is a model to include, mostly: a netlist opens by File → Open)
  const asc = paths.find((p) => /\.asc$/i.test(p)) ?? paths.find(isSymbolDoc);
  if (asc) return openFile(asc);
  const models = isSymbolDoc(useS.getState().path) ? [] : paths.filter((p) => MODEL_FILE.test(p));
  if (models.length) newItem({ t: "text", x: 0, y: 0, directive: true, text: models.map((p) => `.include "${relToDoc(p)}"`).join("\n") });
}

/** Save (as): true once the file is written. */
async function saveFile(as = false): Promise<boolean> {
  // (a dialog to come: a Quit waits for it, as for discardOk's; a plain save does not stop a Quit)
  const { path, unwritten } = useS.getState(), asks = as || !path || path === unwritten;
  if (asks) prompting++;
  try {
    return await saveNow(as);
  } finally {
    if (asks) prompting--;
  }
}
async function saveNow(as: boolean): Promise<boolean> {
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
  // GTK save dialogs do not add the filter's extension, and checked for overwrite without it
  const bare = !path.toLowerCase().endsWith(`.${ext}`);
  if (bare) path += `.${ext}`;
  // (open in another window: the two would write over each other)
  if ((path !== prev || prev === useS.getState().unwritten) && (await elsewhere(path, false))) { // (a restored copy's first save too)
    await message(`${fileName(path)} is open in another window: close it there first.`, { title: "nomospice" });
    return false;
  }
  if (bare && (await invoke<boolean>("path_exists", { path })) && !(await ask(`${fileName(path)} already exists. Replace it?`, { title: "nomospice", kind: "warning" }))) return false;
  const items = useS.getState().items;
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
/** ⌘W: this window closes (asking about unsaved changes); the last one stays, as an empty schematic. */
async function closeDoc() {
  if ((await invoke<number>("window_count").catch(() => 1)) > 1) await getCurrentWindow().close();
  else await newFile();
}
async function openExample(items: Item[]) {
  if (await discardOk()) await loadDoc(items, null);
}
/** A new symbol to draw: its first save asks where (beside the schematics that place it, `dir`:
 *  of this window's). This window holds something: in a window of its own. */
async function newSymbol(dir = dirOf(useS.getState().path)) {
  if (!untouched()) return void newWindow(["--new-symbol", dir ?? ""]);
  // (a name no file there has: the new symbol is no other one's)
  // (in the folder's own separator; a root's ends in one)
  const sep = dir?.includes("\\") ? "\\" : "/";
  const path = dir ? await freeName(`${dir.endsWith(sep) ? dir : dir + sep}untitled`, "asy") : "untitled.asy";
  await loadDoc([{ t: "raw", line: "SymbolType CELL" }, { t: "raw", line: "SYMATTR Prefix X" }], path, true);
}

/** A block's schematic in place of this one (`up`: the one above); the way back is kept. */
async function openSheet(path: string, up = false) {
  if (await elsewhere(path)) return; // (open in another window: there)
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
  await message(`The block symbol ${name} is written. Place it (Component, ${name}) in a schematic in this folder: it stands for this one, its port labels are the pins.`, { title: "nomospice" });
}

/** What the menu bar's items do, by id (see build_menu), as the toolbar, hotkeys and context menus. */
const C: Record<string, () => unknown> = {
  open: () => openFile(), close: closeDoc, save: () => saveFile(), "save-as": () => saveFile(true),
  "new-symbol": () => newSymbol(), "block-symbol": blockSymbol,
  "export-svg": () => exportSchematic(),
  "export-plot": () => window.dispatchEvent(new Event("nomospice:export-plot")),
  "export-raw": () => useS.getState().sim?.run && saveRaw(),
  libraries: () => openLibraries(), "check-updates": () => checkForUpdate(true),
  about: () => useS.setState({ dialog: { k: "about" } }), help: () => useS.setState({ dialog: { k: "help" } }),
  undo: () => undo(), redo: () => redo(), cut: () => copySelection(true), copy: () => copySelection(), paste: () => pasteClipboard(),
  "select-all": () => useS.setState({ sel: useS.getState().items.flatMap((it, i) => (it.t === "raw" ? [] : [i])) }),
  duplicate: () => duplicate(), delete: () => deleteSelection(), rotate: () => transform(rotate), mirror: () => transform(mirror),
  find: () => useS.setState({ dialog: { k: "find" } }),
  wire: () => useS.setState({ tool: { k: "wire" }, sel: [] }), ground: () => placeFlag("0"), label: () => newItem({ t: "flag", x: 0, y: 0, name: "" }),
  res: () => placeSymbol("res"), cap: () => placeSymbol("cap"), ind: () => placeSymbol("ind"), diode: () => placeSymbol("diode"),
  voltage: () => placeSymbol("voltage"), current: () => placeSymbol("current"), component: () => useS.setState({ dialog: { k: "parts" } }),
  comment: () => newItem({ t: "text", x: 0, y: 0, text: "", directive: false }), directive: () => newItem({ t: "text", x: 0, y: 0, text: "", directive: true }),
  select: () => useS.setState({ tool: { k: "select" } }), move: () => moveTool(), "delete-tool": () => useS.setState({ tool: { k: "delete" } }),
  run: () => simulate(), halt: () => useS.getState().running && invoke("halt"), "sim-command": () => useS.setState({ dialog: { k: "sim" } }),
  live: () => useS.setState({ live: !useS.getState().live }), "add-trace": () => useS.getState().sim?.run && useS.setState({ dialog: { k: "trace" } }),
  fit: () => window.dispatchEvent(new Event("nomospice:fit")),
  "zoom-in": () => window.dispatchEvent(new CustomEvent("nomospice:zoom", { detail: 1 })), "zoom-out": () => window.dispatchEvent(new CustomEvent("nomospice:zoom", { detail: -1 })),
  panel: () => togglePanel(), history: () => useS.setState({ panel: "history" }),
  theme: () => window.dispatchEvent(new Event("nomospice:theme")), up: () => window.dispatchEvent(new Event("nomospice:up")),
};
/** Commands a symbol has no use for (no parts, directives or analyses; drawn upright). */
const NOT_IN_SYMBOLS = new Set(["component", "directive", "ground", "res", "cap", "ind", "diode", "voltage", "current", "rotate", "mirror", "run", "sim-command", "live", "add-trace", "export-plot", "export-raw", "block-symbol"]);

/** A hotkey: the menu bar's command `id`, by the same rules. */
const go = (id: string) => () => command(id);

/** A menu bar command for this window. Undo, Redo, Select All (Linux: Cut, Copy, Paste) in a text
 *  field are its own. (Its keys reach it only where the page lets them: see the keydown handler.) */
export function command(id: string) {
  const field = textField();
  if (field && (id === "undo" || id === "redo" || id === "cut" || id === "copy" || id === "paste")) return void document.execCommand(id);
  if (field && id === "select-all") return field.select();
  if (field && ["duplicate", "delete", "rotate", "mirror"].includes(id)) return; // (⌃R in a field: no part turns)
  const s = useS.getState();
  if (s.menu) useS.setState({ menu: null }); // (clicked in the menu bar)
  // a dialog open: ⌘W closes it (as Esc); the rest waits (it would replace the dialog and lose its edits)
  if (s.dialog) return void (id === "close" && useS.setState({ dialog: null }));
  if (isSymbolDoc(s.path) && NOT_IN_SYMBOLS.has(id)) return;
  void C[id]?.();
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
    if (getCurrentWindow().label !== "main") return; // (once, not per window)
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
      // (the first window: the command line's; another: what it was opened for)
      const args = await invoke<string[]>("cli_args");
      const file = args.find((a) => /\.asc$/i.test(a) || isSymbolDoc(a) || NETLIST.test(a));
      const main = getCurrentWindow().label === "main";
      if (args.includes("--new-symbol")) return void newSymbol(args[args.indexOf("--new-symbol") + 1] || null);
      if (main || args.includes("--restore")) {
        const rec = await invoke<{ path: string | null; items: Item[]; unwritten?: string | null } | null>("recovery_take");
        // an empty document is a valid snapshot too (the user deleted everything)
        const restore = Array.isArray(rec?.items) && (await ask(`Restore unsaved changes${rec.path ? ` to ${rec.path}` : ""} from the last session?`, { title: "nomospice", kind: "info" }));
        await invoke("recovery_resolve", { keep: restore }).catch(() => {}); // restore: becomes this window's recovery file
        if (restore) {
          // (a document never written, an opened netlist's .asc: its first save asks, as it would have;
          // as the snapshot says, else by whether its file is there)
          const unwritten = !!rec!.path && (rec!.unwritten === rec!.path || !(await invoke<boolean>("path_exists", { path: rec!.path })));
          // (open in another window by now: restored as a copy, whose first save asks where)
          const other = rec!.path && !unwritten ? await docWindow(rec!.path, false, true) : null;
          await loadDoc(rec!.items, rec!.path, unwritten || (!!other && other !== getCurrentWindow().label));
          useS.setState({ dirty: true });
          // (the file asked for: in a window of its own, unless it is the one restored)
          if (file && !(await docWindow(file))) void newWindow([file, ...args.filter((a) => a === "--run")]);
        } else if (!main) return void getCurrentWindow().destroy(); // (opened only for it)
        // more unsaved work (several windows were open): each in a window of its own, asked there
        if (main) for (let k = await invoke<number>("recovery_count").catch(() => 0); k > 0; k--) void newWindow(["--restore"]);
        if (restore) return;
      }
      // (unless another window has the file by now: that one comes forward, one opened for it closes)
      if (file && (await docWindow(file, true, true))) return void (!main && getCurrentWindow().destroy());
      try {
        if (file) await openPath(file);
        if (args.includes("--run")) await simulate();
      } catch (e) {
        resyncDoc(); // (claimed, but maybe not opened: free again)
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
    // (the menu's; another window's switch reaches this one as a storage event)
    const toggle = () => setMode((m) => updateThemePreference(m === "dark" ? "light" : "dark").mode);
    const synced = (e: StorageEvent) => void (e.key === "theme" && setMode(initializeTheme().mode));
    window.addEventListener("nomospice:theme", toggle);
    window.addEventListener("storage", synced);
    return () => (window.removeEventListener("nomospice:theme", toggle), window.removeEventListener("storage", synced));
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
      const st = useS.getState();
      if (typing(e) || st.dialog || st.menu) return void (swallow(e, !!st.menu) && e.preventDefault());
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
        if (k === "z") return run(go(e.shiftKey ? "redo" : "undo"));
        if (k === "y") return run(go("redo"));
        if (k === "r") return run(go("rotate"));
        if (k === "e") return run(go("mirror"));
        if (k === "a") return run(go("select-all"));
        if (k === "d") return run(go("duplicate"));
        // copy, cut, paste of items, by key (WebKit sends no copy event without a text selection);
        // selected text anywhere (the log, the netlist): its own copy
        const text = !!sel?.toString();
        if ((k === "c" || k === "x") && !text) return void (copySelection(k === "x") && e.preventDefault());
        if (k === "v") return e.preventDefault(), pasteClipboard(); // (a selection does not matter: typing does, see above)
        if (k === "s") return run(go(e.shiftKey ? "save-as" : "save"));
        if (k === "o") return run(go("open"));
        if (k === "n") return run(newWindow);
        if (k === "k") return run(go("component"));
        if (k === "f" && !e.shiftKey && !e.altKey) return run(go("find"));
        if (k === ",") return run(go("libraries"));
        if (k === "w" && !MAC) return run(go("close")); // Ctrl+W closes the window (macOS: File → Close takes ⌘W)
        if (k === "l") return run(go("panel"));
        if (k === "=" || k === "+") return run(go("zoom-in")); // (+: where = needs Shift, a German keyboard)
        if (k === "-") return run(go("zoom-out"));
        if (k === "enter") return run(go("run"));
        return;
      }
      const map: Record<string, () => unknown> = {
        escape: () => (window.dispatchEvent(new Event("nomospice:cancel")), useS.setState({ sel: [] })),
        r: go("res"), c: go("cap"), l: go("ind"), d: go("diode"), v: go("voltage"), i: go("current"), g: go("ground"),
        w: go("wire"), f3: go("wire"), n: go("label"), f4: go("label"), s: go("directive"), t: go("comment"), p: go("component"), f2: go("component"),
        delete: deleteSelection, backspace: deleteSelection,
        f5: () => (useS.getState().sel.length ? deleteSelection() : useS.setState({ tool: { k: "delete" } })),
        f6: duplicate, f9: e.shiftKey ? redo : undo,
        // (F7/F8 as LTspice has them; on a Mac they are media keys unless fn is held: M too)
        // (F1; on a Mac a brightness key unless fn is held: ? too)
        f1: go("help"), "?": go("help"), f7: moveTool, m: moveTool, f8: go("select"), " ": go("fit"),
      };
      // (a symbol holds no parts, directives or analyses)
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
        <ToolbarButton label="New window" hotkey="⌘N" icon={<FilePlus />} onClick={() => newWindow()} />
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
        <ToolbarButton label="Edit simulation command" disabled={symbolDoc} icon={<SlidersHorizontal />} onClick={() => useS.setState({ dialog: { k: "sim" } })} />
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
        <ToolbarButton label="SPICE directive" hotkey="S" disabled={symbolDoc} icon={<Code />} onClick={() => newItem({ t: "text", x: 0, y: 0, text: "", directive: true })} />
        <ToolbarSeparator />
        <ToolbarButton label="Rotate" hotkey="⌃R" icon={<RotateCw />} disabled={symbolDoc} onClick={() => transform(rotate)} />
        <ToolbarButton label="Mirror" hotkey="⌃E" icon={<FlipHorizontal2 />} disabled={symbolDoc} onClick={() => transform(mirror)} />
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
