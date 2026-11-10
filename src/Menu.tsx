import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useS } from "./store";
import { cn, keys } from "@/lib/utils";

export type MenuItem = { label: string; hint?: string; run: () => unknown; disabled?: boolean; danger?: boolean } | "-";

export const openMenu = (x: number, y: number, items: MenuItem[]) => useS.setState({ menu: { x, y, items } });
const closeMenu = () => useS.setState({ menu: null });

/** Context menu: at the pointer, kept on screen; arrows/Enter/Esc; closes on outside click. */
export function ContextMenu() {
  const menu = useS((s) => s.menu);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: 0, y: 0 });
  useLayoutEffect(() => {
    if (!menu || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    setPos({ x: Math.min(menu.x, innerWidth - r.width - 4), y: Math.min(menu.y, innerHeight - r.height - 4) });
    ref.current.querySelector<HTMLElement>("[role=menuitem]:not(:disabled)")?.focus();
  }, [menu]);
  useEffect(() => {
    if (!menu) return;
    const away = (e: Event) => !ref.current?.contains(e.target as Node) && closeMenu();
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("blur", closeMenu);
    window.addEventListener("resize", closeMenu);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("blur", closeMenu);
      window.removeEventListener("resize", closeMenu);
    };
  }, [menu]);
  if (!menu) return null;
  const onKey = (e: React.KeyboardEvent) => {
    const all = [...ref.current!.querySelectorAll<HTMLElement>("[role=menuitem]:not(:disabled)")];
    const k = all.indexOf(document.activeElement as HTMLElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      all[(k + (e.key === "ArrowDown" ? 1 : all.length - 1)) % all.length]?.focus();
    } else if (e.key === "Escape") closeMenu();
    // the app's hotkeys stay out while the menu is open, and the menu bar's keys (macOS) too, but
    // the menu's own (Enter, Space, Tab) and Close, Quit, model libraries
    else if ((e.key.length === 1 && e.key !== " " && !e.metaKey && !e.ctrlKey) || (e.metaKey && !"wq,".includes(e.key.toLowerCase()))) e.preventDefault();
    e.stopPropagation();
  };
  return (
    <div
      ref={ref} role="menu" onKeyDown={onKey} onContextMenu={(e) => e.preventDefault()}
      style={{ left: pos.x, top: pos.y }}
      className="fixed z-[60] min-w-[220px] rounded-float border border-menu-border bg-menu-bg py-1 text-xs shadow-float"
    >
      {menu.items.map((it, i) =>
        it === "-" ? <div key={i} className="my-1 h-px bg-divider" /> : (
          <button
            key={i} role="menuitem" type="button" disabled={it.disabled}
            onClick={() => (closeMenu(), it.run())}
            className={cn(
              "flex h-[24px] w-full items-center gap-6 px-3 text-left outline-none focus:bg-menu-highlight disabled:opacity-40",
              it.danger ? "text-status-danger" : "text-text",
            )}
          >
            <span className="flex-1">{it.label}</span>
            {it.hint && <span className="font-mono text-2xs text-text-tertiary">{keys(it.hint)}</span>}
          </button>
        ),
      )}
    </div>
  );
}
