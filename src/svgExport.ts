import { invoke } from "@tauri-apps/api/core";
import { message, save } from "@tauri-apps/plugin-dialog";
import { dirOf, fileName, useS } from "./store";

/** Run `fn` with the light theme's CSS variables: exports are for slides and paper. The class is
 *  restored in the same task, so nothing is painted in between. */
export function withLightTheme<T>(fn: () => T): T {
  const root = document.documentElement;
  const was = ["dark", "light"].filter((c) => root.classList.contains(c));
  root.classList.remove("dark");
  root.classList.add("light");
  try {
    return fn();
  } finally {
    root.classList.remove("light");
    root.classList.add(...was);
  }
}

const PROPS = ["color", "fill", "fill-opacity", "stroke", "stroke-width", "stroke-dasharray", "stroke-linecap", "stroke-linejoin",
  "opacity", "font-family", "font-size", "font-weight", "font-style", "text-anchor", "dominant-baseline"];

/** Computed styles as plain attributes: the file must look the same without the app's CSS
 *  (Inkscape, PowerPoint, LaTeX via svg → pdf). An attribute is written only where it differs from
 *  the parent's, and `currentColor` / `var(--…)` are always resolved. */
export function inlineStyles(svg: SVGSVGElement) {
  const els = [svg, ...svg.querySelectorAll<SVGElement>("*")].filter((el) => !el.closest("defs"));
  // read everything first: dropping a parent's class would change what its children compute
  const vals = new Map(els.map((el) => {
    const cs = getComputedStyle(el);
    return [el, Object.fromEntries(PROPS.map((p) => [p, cs.getPropertyValue(p)]))] as const;
  }));
  for (const el of els) {
    const mine = vals.get(el)!, parent = vals.get(el.parentElement as unknown as SVGElement);
    for (const p of PROPS) {
      const v = mine[p], own = el.getAttribute(p);
      if (!v || v === "normal" || v === "auto") continue;
      const unresolved = !!own && (own === "currentColor" || own.includes("var("));
      if (p === "opacity" ? v === "1" && !unresolved : parent?.[p] === v && !unresolved) continue;
      el.setAttribute(p, v);
    }
    el.removeAttribute("class");
    el.removeAttribute("style"); // also drops vector-effect: a scaled export should scale its strokes
  }
}

/** Ask where to save a `.ext` file beside the schematic (named after it, plus `suffix`), then let
 *  the core write it with `command` (its error shown); what the command returns, or undefined. */
async function saveBeside<T>(ext: string, filter: string, suffix: string, command: string, args: Record<string, unknown>): Promise<T | undefined> {
  const { path } = useS.getState();
  const base = fileName(path).replace(/\.asc$/i, ""), dir = path ? dirOf(path) : undefined;
  const name = `${base}${suffix}.${ext}`;
  const target = await save({ defaultPath: dir ? `${dir}/${name}` : name, filters: [{ name: filter, extensions: [ext] }] });
  if (!target) return;
  const file = target.toLowerCase().endsWith(`.${ext}`) ? target : `${target}.${ext}`;
  return invoke<T>(command, { path: file, ...args }).catch((e) => void message(String(e), { title: "Export failed", kind: "error" }));
}

/** An SVG (the schematic, the plots) beside the schematic. */
export const saveSvg = (svg: string, suffix: string) => saveBeside("svg", "SVG image", suffix, "export_svg", { svg });

/** The shown run's results as a .raw file (ngspice's binary raw format). */
export async function saveRaw() {
  const run = useS.getState().sim?.run;
  if (!run) return;
  await saveBeside("raw", "Simulation results (raw)", "", "export_raw", { run: run.id });
}

export const xmlEscape = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
