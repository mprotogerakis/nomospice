// Editing actions shared by the toolbar, hotkeys and context menus.
import type { Item } from "./types";
import { commit, useS } from "./store";
import { rotateInPlace } from "./geom";

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

export const placeFlag = (name: string) => useS.setState({ tool: { k: "place", item: { t: "flag", x: 0, y: 0, name } }, sel: [] });

/** Ask for a label/comment/directive's text, then place it (at `at`, or with the mouse). */
export const newItem = (item: Item, at?: [number, number]) => useS.setState({ dialog: { k: "new", item, at } });
