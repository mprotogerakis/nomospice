// Editing actions shared by the toolbar, hotkeys and context menus.
import type { Item, Shape } from "./types";
import { commit, forDoc, isSymbolDoc, loadSymbols, useS } from "./store";
import { box, isShape, rotateInPlace, shifted, snap } from "./geom";

export function deleteSelection() {
  const { items, sel } = useS.getState();
  if (!sel.length) return;
  const s = new Set(sel);
  commit(items.filter((_, i) => !s.has(i)), []);
}

/** Rotate/mirror the part being placed, else the selected parts. */
export function transform(f: (r: string) => string) {
  const { tool, items, sel, lib } = useS.getState();
  if (tool.k === "place" && tool.item.t === "sym") return useS.setState({ tool: { k: "place", item: { ...tool.item, rot: f(tool.item.rot) } } });
  const s = new Set(sel);
  if (items.some((it, i) => s.has(i) && it.t === "sym")) commit(items.map((it, i) => (s.has(i) && it.t === "sym" ? rotateInPlace(it, lib, f) : it)));
}

export function duplicate() {
  const { items, sel } = useS.getState();
  const it = sel.length === 1 ? items[sel[0]] : null;
  if (!it || it.t === "wire" || it.t === "raw") return;
  const copy = it.t === "sym" ? { ...it, attrs: Object.fromEntries(Object.entries(it.attrs).filter(([k]) => k !== "InstName")) } : forDoc({ ...it }, true);
  if (!copy) return;
  useS.setState({ tool: { k: "place", item: copy }, sel: [] });
}

/** Clipboard text of copied items: this prefix, then their JSON (also between two windows). */
const CLIP = "nomospice-items:";

/** The selection as clipboard text, or null: nothing to copy. */
function copyText(): string | null {
  const { items, sel } = useS.getState();
  const its = [...sel].sort((a, b) => a - b).map((i) => items[i]).filter((it): it is Shape => !!it && isShape(it)); // (document order)
  return its.length ? CLIP + JSON.stringify(its) : null;
}

// (what the core takes: i32 coordinates; no line breaks or control characters a saved .asc line
// would carry into lines of its own, no spaces in a symbol name or an attribute's key)
const num = (v: unknown) => typeof v === "number" && Number.isInteger(v) && Math.abs(v) < 2 ** 30;
// (control characters but the tab; a text's own line breaks too)
const CONTROL = /[\u0000-\u0008\u000b-\u001f\u007f]/;
const str = (v: unknown) => typeof v === "string" && !CONTROL.test(v) && !v.includes("\n");
const word = (v: unknown) => str(v) && !/\s/.test(v as string) && (v as string).length > 0;
// (a symbol's WINDOW lines, a label's IOPIN: nothing else rides along to be read as items later)
const extras = (v: unknown, head: string) => (Array.isArray(v) && v.every((l) => str(l) && new RegExp(`^\\s*${head}\\s`).test(l as string)) ? { extra: v as string[] } : {});
/** A pasted item is only taken as one of ours (the clipboard holds whatever was copied anywhere). */
function shape(v: unknown): Shape | null {
  const it = v as Record<string, unknown>;
  if (!it || typeof it !== "object") return null;
  if (it.t === "wire" && [it.x1, it.y1, it.x2, it.y2].every(num) && (it.x1 !== it.x2 || it.y1 !== it.y2)) return { t: "wire", x1: it.x1 as number, y1: it.y1 as number, x2: it.x2 as number, y2: it.y2 as number };
  // (a drawing: its kind, all its points; its pen as LTspice knows it, Normal or Wide; a style 0 … 4)
  if (it.t === "draw" && typeof it.kind === "string" && /^(LINE|RECTANGLE|CIRCLE|ARC)$/i.test(it.kind) && Array.isArray(it.p) && it.p.length === (/^arc$/i.test(it.kind) ? 8 : 4) && it.p.every(num) && (it.style == null || num(it.style)))
    return { t: "draw", kind: it.kind.toUpperCase(), width: /^wide$/i.test(String(it.width)) ? "Wide" : "Normal", p: it.p as number[], ...(typeof it.style === "number" && it.style >= 0 && it.style <= 4 ? { style: it.style } : {}) }; // (another style: none, not a 0 that was not there)
  if (!num(it.x) || !num(it.y)) return null;
  // (a label as read from a file may end in spaces: its name is the rest)
  // (a symbol's pin, its PIN line with it, may have no name or one of words; one line all the same)
  const pin = Array.isArray(it.extra) && it.extra.some((l) => str(l) && /^\s*PIN\s/.test(l as string));
  if (it.t === "flag" && typeof it.name === "string" && (word(it.name.trim()) || (pin && str(it.name)))) return { t: "flag", x: it.x as number, y: it.y as number, name: it.name.trim(), ...extras(it.extra, "(IOPIN|PIN|PINATTR)") };
  // (a text keeps its tabs and line breaks; its alignment is one of the .asc's words)
  if (it.t === "text" && typeof it.text === "string" && !CONTROL.test(it.text)) {
    // ("Left 2": the alignment and the font size)
    const align = typeof it.align === "string" && /^V?(Left|Right|Center|Top|Bottom) \d+$/.test(it.align) ? { align: it.align } : {};
    return { t: "text", x: it.x as number, y: it.y as number, text: it.text as string, directive: !!it.directive, ...align };
  }
  const attrs = it.attrs as Record<string, unknown>;
  // (an attribute's key: spaces become _, as a SYMATTR line can only hold one word there; empty: a
  // bare SYMATTR line of a file)
  if (it.t === "sym" && word(it.sym) && typeof it.rot === "string" && /^[RM](0|90|180|270)$/.test(it.rot) && attrs && typeof attrs === "object" && !Array.isArray(attrs) && Object.keys(attrs).every(str) && Object.values(attrs).every(str)) {
    const keyed = Object.fromEntries(Object.entries(attrs as Record<string, string>).map(([k, v]) => [k.trim().replace(/\s+/g, "_"), v]));
    return { t: "sym", sym: it.sym as string, x: it.x as number, y: it.y as number, rot: it.rot as string, attrs: keyed, ...extras(it.extra, "WINDOW") };
  }
  return null;
}

/** The items copied last, and whether the system clipboard holds them too (writing it can fail). */
let clip = "", written = false;

/** Copy (or cut) the selection: to the system clipboard (`to`: a copy event's) and here. False:
 *  nothing selected. */
export function copySelection(cut = false, to?: DataTransfer) {
  const t = copyText();
  if (!t) return false;
  clip = t;
  written = !!to;
  if (to) to.setData("text/plain", t);
  else void navigator.clipboard?.writeText(t).then(() => (written = clip === t), () => {});
  if (cut) deleteSelection();
  return true;
}

/** ⌘-keys of the menu bar (see build_menu) the page leaves to it: Close, Quit, model libraries. */
export const MENU_OWN_KEYS = "wq,";
/** ⌘-keys of the menu bar the page runs itself, swallowed where it does not (typing, a dialog):
 *  else the menu bar would run them all the same. */
export const PAGE_KEYS = "nosdfkl=+-";

/** Whether a key the page leaves alone (typing, a dialog or `contextMenu` open) must be swallowed,
 *  else the menu bar's keys (macOS) would run it all the same: in a context menu all but its own
 *  keys and the menu's (Close, Quit, model libraries); elsewhere the page's ⌘-keys and, outside a
 *  text field, letters and ⌃-keys (in one they are its own; a select jumps to what is typed). */
export function swallow(e: KeyboardEvent, contextMenu: boolean) {
  const key = e.key.toLowerCase();
  const letter = key.length === 1 && key !== " " && !e.metaKey && !(e.target instanceof Element && e.target.closest(`${TEXT_FIELD},select`));
  return letter || (e.metaKey && (contextMenu ? !MENU_OWN_KEYS.includes(key) : key === "enter" || (key.length === 1 && PAGE_KEYS.includes(key))));
}

/** What takes typing (a checkbox or button does not). */
export const TEXT_FIELD = "input:not([type=checkbox]):not([type=radio]):not([type=button]),textarea";
/** The focused text field, if one has the focus (its own Undo, Select All, Copy … then). */
export const textField = () => document.activeElement?.closest<HTMLInputElement>(TEXT_FIELD) ?? null;

/** Edit → Cut, Copy, Paste clicked in the menu bar (keys go by keydown): the parts' too. WebKit
 *  offers Copy and Cut only for selected text, unless beforecopy/beforecut say there is something. */
export function installClipboardEvents() {
  const parts = () => !textField() && !useS.getState().dialog && !useS.getState().menu && !window.getSelection()?.toString();
  const offer = (e: Event) => void (parts() && copyText() && e.preventDefault());
  document.addEventListener("beforecopy", offer);
  document.addEventListener("beforecut", offer);
  document.addEventListener("beforepaste", (e) => void (parts() && e.preventDefault()));
  const take = (cut: boolean) => (e: ClipboardEvent) => void (parts() && e.clipboardData && copySelection(cut, e.clipboardData) && e.preventDefault());
  document.addEventListener("copy", take(false));
  document.addEventListener("cut", take(true));
  document.addEventListener("paste", (e) => {
    if (!parts() || !e.clipboardData) return;
    e.preventDefault();
    pasteText(e.clipboardData.getData("text/plain"));
  });
}

/** Paste the system clipboard's items (also from another window). The last copied here instead
 *  where the system clipboard cannot be read here at all, or did not take them; other text there,
 *  or reading declined: nothing. Not if the tool changed while reading (Esc, another tool). */
export function pasteClipboard() {
  const tool = useS.getState().tool;
  const still = () => useS.getState().tool === tool;
  if (clip && !written) return void pasteText(clip);
  if (typeof navigator.clipboard?.readText !== "function") return void (clip && pasteText(clip));
  void navigator.clipboard.readText().then((t) => still() && pasteText(t), () => {});
}

/** Paste clipboard text: the items follow the cursor (held at their top-left grid point) until a
 *  click places them. False: not items of ours. */
function pasteText(text: string): boolean {
  if (!text.startsWith(CLIP)) return false;
  let raw: unknown;
  try {
    raw = JSON.parse(text.slice(CLIP.length));
  } catch {
    return false;
  }
  // (a pin whose number a pin here has is a copy: it gets one of its own; a cut one keeps its own)
  const order = (e?: string[]) => e?.find((l) => l.trimStart().startsWith("PINATTR SpiceOrder "))?.trim();
  const orders = new Set(useS.getState().items.flatMap((x) => (x.t === "flag" && order(x.extra) ? [order(x.extra)] : [])));
  const its = (Array.isArray(raw) ? raw : []).map(shape).filter((x): x is Shape => !!x)
    .map((x) => forDoc(x, x.t === "flag" && orders.has(order(x.extra)))).filter((x): x is Shape => !!x && x.t !== "raw");
  if (!its.length) return false;
  // (anchored by what is drawn: not an arc's rays, which may reach far out)
  const bs = its.map((it) => (it.t === "wire" ? [Math.min(it.x1, it.x2), Math.min(it.y1, it.y2)] : it.t === "draw" ? box(it, {})! : [it.x, it.y]));
  // (reduce, not Math.min(...): a whole big schematic would overflow the call's arguments)
  const [x0, y0] = [snap(bs.reduce((m, b) => Math.min(m, b[0]), Infinity)), snap(bs.reduce((m, b) => Math.min(m, b[1]), Infinity))];
  // (a group wider than any schematic: placed anywhere it would leave the core's coordinates;
  // every coordinate, an arc's rays too)
  const coords = its.flatMap((it) => (it.t === "wire" ? [[it.x1, it.y1], [it.x2, it.y2]] : it.t === "draw" ? it.p.flatMap((v, k) => (k % 2 ? [] : [[v, it.p[k + 1]]])) : [[it.x, it.y]]));
  if (coords.some(([x, y]) => Math.abs(x - x0) > 2 ** 24 || Math.abs(y - y0) > 2 ** 24)) return false;
  useS.setState({ tool: { k: "paste", items: its.map((it) => shifted(it, -x0, -y0)) }, sel: [] });
  void loadSymbols(its).catch(() => {}); // (parts from another schematic: their symbols, if this one has none)
  return true;
}

/** Place a label (ground: `0`); a symbol has no ground. */
export const placeFlag = (name: string) => !isSymbolDoc(useS.getState().path) && useS.setState({ tool: { k: "place", item: { t: "flag", x: 0, y: 0, name } }, sel: [] });

/** Ask for a label/comment/directive's text, then place it (at `at`, or with the mouse). */
export const newItem = (item: Item, at?: [number, number]) => useS.setState({ dialog: { k: "new", item, at } });
