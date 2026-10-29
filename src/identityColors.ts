/** Local presentation preferences, independent of machine connectivity and task state. */
export type IdentityKind = "machine" | "workspace";
export const identityColorsStorageKey = "kanban.identityColors.v1";

// Persist actual CSS colors, so future palette edits cannot recolor existing identities.
const palette = [
  "#93c5fd", "#c4b5fd", "#f9a8d4", "#fdba74",
  "#67e8f9", "#bef264", "#fda4af", "#a5b4fc",
  "#5eead4", "#fde68a", "#d8b4fe", "#86efac",
];
type Colors = Record<IdentityKind, Map<string, string>>;
let colors: Colors | undefined;

function validColor(value: unknown): value is string {
  return typeof value === "string" && (/^#[\da-f]{6}$/i.test(value)
    || /^hsl\(\d{1,3}\.\d{3} 65% 75%\)$/.test(value));
}

function readColors(): Colors {
  const result: Colors = { machine: new Map(), workspace: new Map() };
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(identityColorsStorageKey) ?? "null");
    if (saved && typeof saved === "object") {
      for (const kind of ["machine", "workspace"] as const) {
        const entries = (saved as Record<string, unknown>)[kind];
        if (!entries || typeof entries !== "object" || Array.isArray(entries)) continue;
        for (const [id, color] of Object.entries(entries)) {
          if (validColor(color)) result[kind].set(id, color);
        }
      }
    }
  } catch { /* Missing, invalid or unavailable storage falls back to session preferences. */ }
  return result;
}

function candidateColor(kind: IdentityKind, index: number): string {
  if (index < palette.length) return palette[(index + (kind === "workspace" ? 4 : 0)) % palette.length];
  // More identities retain distinct values, though large sets can look similar.
  return `hsl(${((index * 137.508) % 360).toFixed(3)} 65% 75%)`;
}

/** Names, list order, hidden state and connection status never participate in the key. */
export function identityColor(kind: IdentityKind, id: string): string {
  colors ??= readColors();
  const existing = colors[kind].get(id);
  if (existing) return existing;
  const used = new Set(colors[kind].values());
  let index = 0;
  let color = candidateColor(kind, index);
  while (used.has(color)) color = candidateColor(kind, ++index);
  colors[kind].set(id, color);
  try {
    localStorage.setItem(identityColorsStorageKey, JSON.stringify({
      machine: Object.fromEntries(colors.machine),
      workspace: Object.fromEntries(colors.workspace),
    }));
  } catch { /* Keep the same colors in memory when preferences cannot be written. */ }
  return color;
}
