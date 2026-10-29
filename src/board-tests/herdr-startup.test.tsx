// @vitest-environment jsdom
import { project, bindTask, board } from "./fixtures";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";

it("waits for local Herdr before syncing or opening the cached task terminal", async () => {
  bindTask();
  board.storedTask.status = "in_progress";
  let ready!: () => void;
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "list_projects") return Promise.resolve([project]);
    if (command === "ensure_local_herdr") return new Promise<void>(resolve => { ready = resolve; });
    if (command === "sync_projects") return Promise.resolve({ projects: [project], operations: [], session: "default" });
    return base(command, args, options);
  });
  render(<App />);
  await waitFor(() => expect(ready).toBeTypeOf("function"));
  await screen.findByRole("button", { name: "正在检查并启动 Herdr…" });
  expect(invoke).not.toHaveBeenCalledWith("sync_projects");
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_open")).toBe(false);
  fireEvent(window, new Event("focus"));
  await act(async () => ready());
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_projects"));
  await screen.findByText("Herdr 已打开");
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "ensure_local_herdr")).toHaveLength(1);
});

it.each([
  ["herdr.startup_timeout", /等待 Herdr 启动超时/],
  ["herdr.invalid_response", /Herdr 返回了无效数据/],
])("keeps cached cards on %s, avoids automatic relaunch, and clears the error after retry", async (error, message) => {
  let failing = true;
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (command, args, options) => {
    if (command === "list_projects") return [project];
    if (command === "ensure_local_herdr") { if (failing) throw error; return; }
    if (command === "sync_projects") return { projects: [project], operations: [], session: "default" };
    return base(command, args, options);
  });
  render(<App />);
  await screen.findByText(message);
  await screen.findByText("修复拖放派发");
  await act(async () => { fireEvent(window, new Event("focus")); });
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "ensure_local_herdr")).toHaveLength(1);
  failing = false;
  fireEvent.click(screen.getByRole("button", { name: "重试连接" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("ensure_local_herdr", { retry: true }));
  await waitFor(() => expect(screen.queryByText(message)).toBeNull());
  expect(invoke).toHaveBeenCalledWith("sync_projects");
});
