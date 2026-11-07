// Editing actions shared by the toolbar, hotkeys and context menus.
import type { Item, Shape } from "./types";
import { commit, loadSymbols, useS } from "./store";
import { isShape, rotateInPlace, shifted, snap } from "./geom";

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
  const copy: Item = it.t === "sym" ? { ...it, attrs: Object.fromEntries(Object.entries(it.attrs).filter(([k]) => k !== "InstName")) } : { ...it };
  useS.setState({ tool: { k: "place", item: copy }, sel: [] });
}

/** Clipboard text of copied items: this prefix, then their JSON (also between two windows). */
const CLIP = "nomospice-items:";

/** The selection as clipboard text, or null: nothing to copy. */
export function copyText(): string | null {
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
  if (!num(it.x) || !num(it.y)) return null;
  // (a label as read from a file may end in spaces: its name is the rest)
  if (it.t === "flag" && typeof it.name === "string" && word(it.name.trim())) return { t: "flag", x: it.x as number, y: it.y as number, name: it.name.trim(), ...extras(it.extra, "IOPIN") };
  // (a text keeps its tabs and line breaks; its alignment is one of the .asc's words)
  if (it.t === "text" && typeof it.text === "string" && !CONTROL.test(it.text)) {
    // ("Left 2": the alignment and the font size)
    const align = typeof it.align === "string" && /^V?(Left|Right|Center|Top|Bottom) \d+$/.test(it.align) ? { align: it.align } : {};
    return { t: "text", x: it.x as number, y: it.y as number, text: it.text as string, directive: !!it.directive, ...align };
  }
  const attrs = it.attrs as Record<string, unknown>;
  // (an attribute's key without spaces, or empty: a bare SYMATTR line of a file)
  if (it.t === "sym" && word(it.sym) && typeof it.rot === "string" && /^[RM](0|90|180|270)$/.test(it.rot) && attrs && typeof attrs === "object" && Object.keys(attrs).every((k) => str(k) && !/\s/.test(k)) && Object.values(attrs).every(str)) {
    return { t: "sym", sym: it.sym as string, x: it.x as number, y: it.y as number, rot: it.rot as string, attrs: { ...(attrs as Record<string, string>) }, ...extras(it.extra, "WINDOW") };
  }
  return null;
}

/** The items copied last, and whether the system clipboard holds them too (writing it can fail). */
let clip = "", written = false;

/** Copy (or cut) the selection: to the system clipboard and here. False: nothing selected. */
export function copySelection(cut = false) {
  const t = copyText();
  if (!t) return false;
  clip = t;
  written = false;
  void navigator.clipboard?.writeText(t).then(() => (written = clip === t), () => {});
  if (cut) deleteSelection();
  return true;
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
export function pasteText(text: string): boolean {
  if (!text.startsWith(CLIP)) return false;
  let raw: unknown;
  try {
    raw = JSON.parse(text.slice(CLIP.length));
  } catch {
    return false;
  }
  const its = (Array.isArray(raw) ? raw : []).map(shape).filter((x): x is Shape => !!x);
  if (!its.length) return false;
  const xs = its.flatMap((it) => (it.t === "wire" ? [it.x1, it.x2] : [it.x])), ys = its.flatMap((it) => (it.t === "wire" ? [it.y1, it.y2] : [it.y]));
  const [x0, y0] = [snap(xs.reduce((a, b) => Math.min(a, b))), snap(ys.reduce((a, b) => Math.min(a, b)))];
  // (a group wider than any schematic: placed anywhere it would leave the core's coordinates)
  if (xs.some((x) => x - x0 > 2 ** 24) || ys.some((y) => y - y0 > 2 ** 24)) return false;
  useS.setState({ tool: { k: "paste", items: its.map((it) => shifted(it, -x0, -y0)) }, sel: [] });
  void loadSymbols(its).catch(() => {}); // (parts from another schematic: their symbols, if this one has none)
  return true;
}

export const placeFlag = (name: string) => useS.setState({ tool: { k: "place", item: { t: "flag", x: 0, y: 0, name } }, sel: [] });

/** Ask for a label/comment/directive's text, then place it (at `at`, or with the mouse). */
export const newItem = (item: Item, at?: [number, number]) => useS.setState({ dialog: { k: "new", item, at } });
