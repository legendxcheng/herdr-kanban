import type { ITerminalOptions } from "@xterm/xterm";

// Snapshot of ~/.config/wezterm/wezterm.lua on 2026-10-24.
// Palette exported from the installed WezTerm's Catppuccin Mocha scheme.
// This is a portable preset, not a runtime Lua configuration loader.
export const terminalAppearance: ITerminalOptions = {
  fontFamily: '"Cascadia Code", "Noto Sans SC", "Microsoft YaHei", monospace',
  // WezTerm uses points; xterm uses CSS pixels (96 px = 72 pt).
  fontSize: 13 * 96 / 72,
  fontWeight: "normal",
  lineHeight: 1.05,
  letterSpacing: 0,
  cursorStyle: "bar",
  cursorBlink: false,
  theme: {
    background: "#1e1e2e",
    foreground: "#cdd6f4",
    cursor: "#f5e0dc",
    cursorAccent: "#11111b",
    selectionBackground: "#585b70",
    selectionForeground: "#cdd6f4",
    black: "#45475a",
    red: "#f38ba8",
    green: "#a6e3a1",
    yellow: "#f9e2af",
    blue: "#89b4fa",
    magenta: "#f5c2e7",
    cyan: "#94e2d5",
    white: "#bac2de",
    brightBlack: "#585b70",
    brightRed: "#f38ba8",
    brightGreen: "#a6e3a1",
    brightYellow: "#f9e2af",
    brightBlue: "#89b4fa",
    brightMagenta: "#f5c2e7",
    brightCyan: "#94e2d5",
    brightWhite: "#a6adc8",
  },
};
