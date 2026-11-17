// The component library's tree: nomospice's own symbols by what they are, the schematic folder's
// and blocks, the model libraries by kind and source, the parts used last. Pure functions: the
// dialog (Dialogs.tsx) draws them, scripts/spice-check.ts checks them.
import type { LibEntry, SymbolDef } from "./types";

/** nomospice's own symbols by what they are, in the tree's order */
const OWN: [string, string[]][] = [
  ["Passive", ["res", "cap", "polcap", "ind"]],
  ["Sources", ["voltage", "current", "bv", "bi", "e", "g"]],
  ["Diodes", ["diode", "schottky", "zener", "led"]],
  ["Transistors", ["npn", "pnp", "nmos", "pmos", "njf", "pjf"]],
  ["Op-amps", ["opamp", "opamp2"]],
  ["Logic", ["and2", "nand2", "or2", "nor2", "xor2", "not", "dff"]],
  ["Switches and lines", ["sw", "csw", "tline", "ltline", "jumper"]],
];
/** The groups of the symbols, in order: nomospice's, the folder's, blocks, others. */
export const PART_GROUPS = [...OWN.map(([g]) => g), "This folder", "Blocks", "Other symbols"];

/** The group a symbol is listed in. */
export function partGroup(d: SymbolDef): string {
  if (d.sheet) return "Blocks";
  if (d.file) return "This folder";
  return OWN.find(([, names]) => names.includes(d.name.toLowerCase()))?.[0] ?? "Other symbols";
}

/** The library's model kinds as the tree groups them: label, kinds (`|`-joined, as library_list takes them). */
export const MODEL_GROUPS: [string, string][] = [
  ["Diode models", "D"], ["BJT models", "NPN|PNP|LPNP"], ["MOSFET models", "NMOS|PMOS|VDMOS"], ["JFET models", "NJF|PJF"], ["Subcircuits", "SUBCKT"],
];
/** The group of a model kind (`NPN`: BJT models); another kind (a switch's SW, a core's): `Other models`. */
export function modelGroup(kind: string): string {
  return MODEL_GROUPS.find(([, ks]) => ks.split("|").includes(kind.toUpperCase()))?.[0] ?? "Other models";
}

/** A model group of a source: its label, the kinds in it (for library_list) and how many. */
export interface SourceGroup { group: string; source: string; kinds: string; count: number }

/** library_groups' (kind, source, count) as the tree's model groups: by group, then source. */
export function sourceGroups(counts: [string, string, number][]): SourceGroup[] {
  const by = new Map<string, SourceGroup>();
  for (const [kind, source, count] of counts) {
    const group = modelGroup(kind), key = `${group}\u0000${source}`;
    const g = by.get(key);
    if (g) (g.count += count), g.kinds.split("|").includes(kind) || (g.kinds += `|${kind}`);
    else by.set(key, { group, source, kinds: kind, count });
  }
  const order = [...MODEL_GROUPS.map(([g]) => g), "Other models"];
  return [...by.values()].sort((a, b) => order.indexOf(a.group) - order.indexOf(b.group) || a.source.localeCompare(b.source));
}

/** The parts used last, newest first, at most `max`; `e` put in front (once). */
export function remember(recent: LibEntry[], e: LibEntry, max = 10): LibEntry[] {
  return [e, ...recent.filter((r) => !(r.name === e.name && r.kind === e.kind))].slice(0, max);
}

/** Where a symbol's pin name is written: away from the symbol's middle (left pins left of the pin …). */
export function pinSide(d: SymbolDef, p: { x: number; y: number }): "left" | "right" | "top" | "bottom" {
  const xs = d.pins.map((q) => q.x), ys = d.pins.map((q) => q.y);
  const cx = (Math.min(...xs) + Math.max(...xs)) / 2, cy = (Math.min(...ys) + Math.max(...ys)) / 2;
  const dx = p.x - cx, dy = p.y - cy;
  return Math.abs(dx) >= Math.abs(dy) ? (dx < 0 ? "left" : "right") : dy < 0 ? "top" : "bottom";
}
