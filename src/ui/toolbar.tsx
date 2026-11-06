// Adapted from OpenPCB (AGPL-3.0-or-later), https://github.com/OpenPCB-app/OpenPCB: larger tools (32 px, 20 px icons)
import * as React from "react";
import { cn, keys } from "@/lib/utils";
import { FOCUS_RING } from "./focus";

export type ToolbarProps = React.HTMLAttributes<HTMLDivElement>;

/** 46px docked editor toolbar; wraps into a second row in narrow windows. */
export const Toolbar = React.forwardRef<HTMLDivElement, ToolbarProps>(
  ({ className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        "flex min-h-[46px] shrink-0 flex-wrap items-center gap-0.5 border-b py-[7px] border-border bg-surface-panel px-1.5 text-text-secondary",
        className,
      )}
      {...props}
    />
  ),
);
Toolbar.displayName = "Toolbar";

export interface ToolbarButtonProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "title"> {
  icon?: React.ReactNode;
  /**
   * Tooltip text. Defaults to the accessible name; pass a longer string (e.g.
   * with a modifier hotkey) without changing `aria-label`.
   */
  title?: string;
  /**
   * Accessible name. With `hotkey` the button's `title` and `aria-label` become
   * exactly `${label} (${hotkey})` — e2e locators depend on these strings.
   */
  label: string;
  hotkey?: string;
  active?: boolean;
  /** Emit `aria-pressed` from `active` (for toggle-style tools). */
  pressable?: boolean;
  /** Optional visible text; makes the button auto-width instead of 24×24. */
  children?: React.ReactNode;
}

/** 32×32 icon tool (or auto-width text tool when `children` is given). */
export const ToolbarButton = React.forwardRef<
  HTMLButtonElement,
  ToolbarButtonProps
>(
  (
    { icon, label, hotkey, title, active = false, pressable = false, className, children, ...props },
    ref,
  ) => {
    const name = hotkey ? `${label} (${keys(hotkey)})` : label;
    const tooltip = title ?? name;
    return (
      <button
        ref={ref}
        type="button"
        className={cn(
          "inline-flex shrink-0 items-center justify-center rounded-control transition-colors",
          FOCUS_RING,
          "disabled:cursor-not-allowed disabled:opacity-35",
          "[&_svg]:h-[20px] [&_svg]:w-[20px] [&_svg]:shrink-0 [&_svg]:[stroke-width:1.6]",
          children !== undefined && children !== null
            ? "h-8 gap-1.5 px-2.5 text-sm"
            : "h-8 w-8",
          active
            ? "bg-surface-control text-text-strong"
            : "hover:bg-surface-hover hover:text-text-strong",
          className,
        )}
        {...props}
        aria-pressed={pressable ? active : undefined}
        title={tooltip}
        aria-label={name}
      >
        {icon}
        {children}
      </button>
    );
  },
);
ToolbarButton.displayName = "ToolbarButton";

/** 1×16 vertical rule between tool groups. */
export function ToolbarSeparator({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("mx-1.5 h-6 w-px shrink-0 bg-divider", className)}
    />
  );
}

/** Pushes the remaining toolbar items to the right edge. */
export function ToolbarSpacer({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cn("flex-1", className)} />;
}
