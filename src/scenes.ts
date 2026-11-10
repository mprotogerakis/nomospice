// Documentation screenshots, dev builds only (scripts/docs-shots.sh). A scene, docs/scenes/<name>.js,
// drives the app as a user would (keys, clicks on the schematic, dialogs) through `api`; each
// shot(id) is a picture of the window, docs/img/<name>/<id>.png. The tutorial pages show them in
// order, and the text after each is what scripts/docs-video.py says over it.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { addTrace, idle, loadDoc, setPlot, simulate, useS } from "./store";
import { command } from "./App";
import type { Item } from "./types";

type Pt = [number, number];
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const frames = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));

/** The schematic's world group: schematic points to window points. */
const world = () => document.querySelector<SVGGElement>("svg g[data-world]")!;
function client([x, y]: Pt) {
  return new DOMPoint(x, y).matrixTransform(world().getScreenCTM()!);
}
function pointer(type: string, at: Pt, down: boolean, mods: KeyMods = {}) {
  const p = client(at);
  world().ownerSVGElement!.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, clientX: p.x, clientY: p.y, button: 0, buttons: down ? 1 : 0, pointerId: 1, isPrimary: true, ...mods }));
}
type KeyMods = { metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean; altKey?: boolean };

/** Set a field's value as typing would (React sees the input event). */
function typeInto(el: HTMLInputElement | HTMLTextAreaElement, text: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, text);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

let scene = "", work = "";

export const api = {
  useS, simulate, addTrace, setPlot, command, sleep, invoke,
  /** The store settled, no run going, and drawn. */
  async settle(ms = 200) {
    await idle();
    await sleep(ms);
    await frames();
  },
  /** A picture of the window: docs/img/<scene>/<id>.png. */
  async shot(id: string) {
    await api.settle(400);
    await invoke("scene_shot", { name: `${scene}/${id}` });
  },
  /** A key as pressed on the schematic (the page's hotkeys: "r", "Escape", "Enter" with metaKey …). */
  async key(key: string, mods: KeyMods = {}) {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...mods }));
    await api.settle();
  },
  /** The mouse over schematic point `at` (a part being placed follows it). */
  async move(at: Pt) {
    pointer("pointermove", at, false);
    await api.settle(50);
  },
  /** A click at schematic point `at`: places, draws a wire's corner, selects or probes. */
  async click(at: Pt, mods: KeyMods = {}) {
    pointer("pointermove", at, false, mods);
    pointer("pointerdown", at, true, mods);
    pointer("pointerup", at, false, mods);
    await api.settle();
  },
  async doubleClick(at: Pt) {
    const p = client(at);
    world().ownerSVGElement!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, clientX: p.x, clientY: p.y }));
    await api.settle();
  },
  async rightClick(at: Pt) {
    const p = client(at);
    world().ownerSVGElement!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: p.x, clientY: p.y, button: 2 }));
    await api.settle();
  },
  /** Types into the field matching `selector` (in the open dialog, if one is). */
  async type(selector: string, text: string) {
    const scope = document.querySelector("dialog[open]") ?? document;
    const el = scope.querySelector<HTMLInputElement>(selector);
    if (!el) throw new Error(`no field ${selector}`);
    el.focus();
    typeInto(el, text);
    await api.settle();
  },
  /** Fills the open dialog's field labelled `label` (a text field, a choice, a box to tick). */
  async fill(label: string, value: string | boolean) {
    const scope = document.querySelector("dialog[open]") ?? document;
    const field = [...scope.querySelectorAll("label")].find((l) => l.querySelector(":scope > span")?.textContent?.trim() === label)?.querySelector<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>("input,select,textarea");
    if (!field) throw new Error(`no field ${label}`);
    field.focus();
    if (field instanceof HTMLSelectElement) {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(field, value);
      field.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (field instanceof HTMLInputElement && field.type === "checkbox") {
      if (field.checked !== value) field.click();
    } else typeInto(field, String(value));
    await api.settle();
  },
  /** The open dialog's OK (whatever it is called). */
  async ok() {
    document.querySelector<HTMLButtonElement>("dialog[open] button[type=submit]")!.click();
    await api.settle();
  },
  /** Clicks the button (or menu item, tab) whose text is `text`. */
  async press(text: string) {
    const scope = document.querySelector("dialog[open]") ?? document;
    // (exactly that text, else one starting with it: a menu item's hint follows, "Libraries…")
    const all = [...scope.querySelectorAll<HTMLElement>("button,[role=menuitem],[role=tab]")];
    const b = all.find((x) => x.textContent?.trim() === text) ?? all.find((x) => x.textContent?.trim().startsWith(text));
    if (!b) throw new Error(`no button ${text}`);
    b.click();
    await api.settle();
  },
  /** Clicks the element matching `selector` (the plot's canvas, a trace row …) at its fraction (fx, fy). */
  async clickAt(selector: string, fx = 0.5, fy = 0.5) {
    const el = document.querySelector<HTMLElement>(selector);
    if (!el) throw new Error(`no ${selector}`);
    const r = el.getBoundingClientRect(), x = r.left + fx * r.width, y = r.top + fy * r.height;
    for (const type of ["mousedown", "mouseup", "click"]) el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, detail: 1 }));
    await api.settle();
  },
  /** A right-click on the element matching `selector` (the plot …) at its fraction (fx, fy): its menu. */
  async menuAt(selector: string, fx = 0.5, fy = 0.5) {
    const el = document.querySelector<HTMLElement>(selector);
    if (!el) throw new Error(`no ${selector}`);
    const r = el.getBoundingClientRect();
    el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + fx * r.width, clientY: r.top + fy * r.height, button: 2 }));
    await api.settle();
  },
  /** A schematic as a document: items, at `name` in the scene's work folder (written there). */
  async load(items: Item[], name?: string) {
    const path = name ? `${work}/${name}` : null;
    if (path) await invoke("save_asc", { path, items });
    await loadDoc(items, path);
    await api.settle(400);
  },
  /** The schematic `name`.asc of the app's examples. */
  async example(name: string) {
    const ex = await invoke<[string, Item[]][]>("examples");
    await api.load(ex.find(([n]) => n === name)![1], `${name}.asc`);
  },
  /** Light or dark, as the toolbar's switch. */
  async theme(mode: "light" | "dark") {
    if (!document.documentElement.classList.contains(mode)) window.dispatchEvent(new Event("nomospice:theme"));
    await api.settle();
  },
};

/** Plays the scene this dev build was started for, if any, then ends the app. */
export async function playScene() {
  const s = await invoke<[string, string] | null>("scene");
  if (!s) return;
  [scene, work] = s;
  // (synthetic pointers are no real ones: nothing to capture)
  Element.prototype.setPointerCapture = () => {};
  Element.prototype.releasePointerCapture = () => {};
  Element.prototype.hasPointerCapture = () => false;
  await getCurrentWindow().setFocus().catch(() => {});
  try {
    await sleep(1500); // (the window's first layout, the examples loaded)
    const mod = await import(/* @vite-ignore */ `/docs/scenes/${scene}.js`);
    await mod.default(api);
  } catch (e) {
    await invoke("scene_log", { message: String(e instanceof Error ? e.stack ?? e.message : e) }).catch(() => {});
    await invoke("scene_shot", { name: `${scene}/ERROR` }).catch(() => {});
  }
  await invoke("scene_done");
}
