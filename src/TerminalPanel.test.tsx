// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TerminalPanel } from "./TerminalPanel";
import type { Task } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class { onmessage = (_packet: unknown) => {}; } }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
// Only the external terminal renderer and native IPC are substituted.
vi.mock("@xterm/xterm", () => ({ Terminal: class {
  cols = 80; rows = 24; options = {}; textarea = document.createElement("textarea");
  open(host: HTMLElement) {
    this.textarea.setAttribute("aria-label", "终端输入");
    const grid = document.createElement("div"); grid.className = "xterm-screen";
    grid.getBoundingClientRect = () => ({ x: 20, y: 50, left: 20, top: 50, right: 820, bottom: 530, width: 800, height: 480, toJSON() {} });
    host.append(grid, this.textarea);
  }
  loadAddon() {} attachCustomKeyEventHandler() {} focus() { this.textarea.focus(); }
  write(_data: unknown, done?: () => void) { done?.(); }
  onData(callback: (value: string) => void) { this.textarea.oninput = () => callback(this.textarea.value); return { dispose() {} }; }
  onResize() { return { dispose() {} }; } dispose() { this.textarea.remove(); }
} }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
const task = { id: "a", name: "任务 A", binding: { state: "valid", target: { pane_id: "p1" } } } as Task;
beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "terminal_open") {
      const input = args as { connectionId: string; output: { onmessage: (packet: unknown) => void } };
      input.output.onmessage({ kind: "data", data: [], cols: 80, rows: 24 });
      return input.connectionId;
    }
  });
});
afterEach(cleanup);
it("scrolls six remote lines per wheel notch without moving the surrounding page", async () => {
  const view = render(<TerminalPanel task={task} />);
  await screen.findByText("已连接");
  const grid = view.container.querySelector(".xterm-screen")!;
  const event = new WheelEvent("wheel", { bubbles: true, cancelable: true, clientX: 145, clientY: 115, deltaY: -3, deltaMode: 1 });
  fireEvent(grid, event);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_scroll", {
    connectionId: expect.any(String), event: { direction: "up", lines: 6, column: 12, row: 3, modifiers: 0, cols: 80, rows: 24 },
  }));
  expect(event.defaultPrevented).toBe(true);
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "terminal_input")).toBe(false);
});
it("accumulates small touchpad deltas and reverses without retaining the previous direction", async () => {
  const view = render(<TerminalPanel task={task} />);
  await screen.findByText("已连接");
  const grid = view.container.querySelector(".xterm-screen")!;
  for (let i = 0; i < 4; i++) {
    fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: -5 });
    fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: 0, deltaX: 10 });
  }
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_scroll", expect.objectContaining({ event: expect.objectContaining({ lines: 1, direction: "up" }) })));
  fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: -10 });
  fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: 20 });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_scroll", expect.objectContaining({ event: expect.objectContaining({ lines: 1, direction: "down" }) })));
});
it("does not forward clicks and discards pending scroll on task switch", async () => {
  let finish!: () => void;
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "terminal_scroll"
    ? new Promise<void>(resolve => { finish = resolve; }) : base(command, args, options));
  const view = render(<TerminalPanel task={task} />);
  await screen.findByText("已连接");
  const grid = view.container.querySelector(".xterm-screen")!;
  fireEvent.mouseDown(grid, { clientX: 145, clientY: 115 });
  fireEvent.mouseUp(grid, { clientX: 145, clientY: 115 });
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_scroll")).toBe(false);
  fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: -120 });
  await waitFor(() => expect(finish).toBeTypeOf("function"));
  fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: 120 });
  view.rerender(<TerminalPanel task={{ ...task, id: "b", name: "任务 B" }} />);
  await screen.findByText("已连接"); finish();
  await new Promise(resolve => setTimeout(resolve, 30));
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_scroll")).toHaveLength(1);
  fireEvent.input(screen.getByLabelText("终端输入"), { target: { value: "only B" } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_input", expect.objectContaining({ data: "only B" })));
});
it("waits for matching rendered geometry and ignores wheel events outside the grid", async () => {
  let frame!: (packet: unknown) => void;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "terminal_open") {
      const input = args as { connectionId: string; output: { onmessage: (packet: unknown) => void } };
      frame = input.output.onmessage; return input.connectionId;
    }
  });
  const view = render(<TerminalPanel task={task} />);
  await screen.findByText("已连接");
  const grid = view.container.querySelector(".xterm-screen")!;
  fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: -120 });
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_scroll")).toBe(false);
  frame({ kind: "data", data: [], cols: 80, rows: 24 });
  fireEvent.wheel(grid, { clientX: 10, clientY: 115, deltaY: -120 });
  fireEvent.wheel(grid, { clientX: 145, clientY: 115, deltaY: -120 });
  await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_scroll")).toHaveLength(1));
});
