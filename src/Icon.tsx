import type { CSSProperties } from "react";

const paths = {
  board: "M3 4h18v16H3z M9 4v16 M15 4v16 M5.5 8h1 M11.5 8h1 M17.5 8h1",
  folder: "M3 7V5h6l2 2h10v13H3z",
  branch: "M6 6v12 M6 12h6a6 6 0 0 0 6-6 M8 4a2 2 0 1 1-4 0 2 2 0 0 1 4 0 M8 20a2 2 0 1 1-4 0 2 2 0 0 1 4 0 M20 4a2 2 0 1 1-4 0 2 2 0 0 1 4 0",
  plus: "M12 5v14 M5 12h14",
  arrow: "M5 12h14 M13 6l6 6-6 6",
  terminal: "m5 7 5 5-5 5 M13 17h6",
  maximize: "M8 3H3v5 M16 3h5v5 M3 16v5h5 M21 16v5h-5",
  restore: "M8 3v5H3 M16 3v5h5 M3 16h5v5 M21 16h-5v5",
  circle: "M20 12a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
  progress: "M12 4a8 8 0 1 1-8 8 M12 4v8l5 3",
  check: "m8 12 3 3 5-6 M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0",
  edit: "m15 4 5 5 M4 20l5-1L21 7l-5-5L4 14z",
  trash: "M3 6h18 M9 6V3h6v3 M5 6l1 15h12l1-15 M10 10v7 M14 10v7",
  archive: "M3 3h18v5H3z M5 8v13h14V8 M9 12h6",
  close: "m6 6 12 12 M6 18 18 6",
  hide: "m3 3 18 18 M10 5a12 12 0 0 1 11 7 17 17 0 0 1-3 4 M6 6a17 17 0 0 0-3 6c4 8 12 9 17 5 M9 9a4 4 0 0 0 6 6",
} as const;

export type IconName = keyof typeof paths;

export function Icon({ name, size = 16, style }: { name: IconName; size?: number; style?: CSSProperties }) {
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={style}><path d={paths[name]} /></svg>;
}
