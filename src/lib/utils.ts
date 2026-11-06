// Copied from OpenPCB (AGPL-3.0-or-later), https://github.com/OpenPCB-app/OpenPCB
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

// nomospice: shortcut labels are written with macOS glyphs; Windows/Linux show Ctrl/Shift/…
const MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent);
export const keys = (s: string) =>
  MAC ? s : s.replace(/⇧⌘/g, "Ctrl+Shift+").replace(/[⌘⌃]/g, "Ctrl+").replace(/⇧/g, "Shift+").replace(/↵/g, "Enter").replace(/⌫/g, "Del");
