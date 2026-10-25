import { terminalAppearance } from "./terminalAppearance";

const storageKey = "kanban.terminal.fontSize";
const minFontSize = 10;
const maxFontSize = 32;
let currentFontSize: number | undefined;

export function readTerminalFontSize(): number {
  if (currentFontSize !== undefined) return currentFontSize;
  currentFontSize = terminalAppearance.fontSize!;
  try {
    const stored = Number(localStorage.getItem(storageKey));
    if (Number.isFinite(stored) && stored >= minFontSize && stored <= maxFontSize) {
      currentFontSize = stored;
    }
  } catch { /* Keep the session preference when storage is unavailable. */ }
  return currentFontSize;
}

export function changeTerminalFontSize(size: number, delta: number): number {
  currentFontSize = Math.min(maxFontSize, Math.max(minFontSize, size + delta));
  try { localStorage.setItem(storageKey, String(currentFontSize)); }
  catch { /* Storage failure must not prevent terminal use. */ }
  return currentFontSize;
}

export function terminalFontSizeDelta(event: KeyboardEvent): number {
  if (!event.ctrlKey || event.altKey || event.metaKey || event.isComposing) return 0;
  if (event.key === "+" || event.key === "=" || event.code === "NumpadAdd") return 1;
  if (event.key === "-" || event.code === "NumpadSubtract") return -1;
  return 0;
}
