// @vitest-environment jsdom
import { bindTask, board, project, sendPrompt } from "./fixtures";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";

function withProject() {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "list_projects"
    ? Promise.resolve([project]) : base(command, args, options));
}

it("最大化和恢复复用当前连接及输入，不触发后台看板快捷键", async () => {
  bindTask(); board.storedTask.status = "in_progress"; withProject();
  render(<App />); await screen.findByText("已连接");
  const input = screen.getByLabelText("终端输入");
  const connection = vi.mocked(invoke).mock.calls.find(([command]) => command === "terminal_open")![1] as { connectionId: string };
  for (let i = 0; i < 3; i++) {
    fireEvent.click(screen.getByRole("button", { name: "最大化终端" }));
    fireEvent.keyDown(document, { key: "n" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.querySelector(".sidebar")!.hasAttribute("inert")).toBe(true);
    expect(document.querySelector(".board-main")!.hasAttribute("inert")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "恢复终端" }));
    expect(document.querySelector(".sidebar")!.hasAttribute("inert")).toBe(false);
    expect(document.querySelector(".board-main")!.hasAttribute("inert")).toBe(false);
    expect(screen.getByLabelText("终端输入")).toBe(input);
  }
  fireEvent.input(input, { target: { value: "still connected" } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_input", { connectionId: connection.connectionId, data: "still connected" }));
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_open")).toHaveLength(1);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_close")).toHaveLength(0);
  expect(sendPrompt).not.toHaveBeenCalled();
});

it("连接尚未完成时可最大化和恢复，迟到就绪仍使用原连接", async () => {
  bindTask(); board.storedTask.status = "in_progress"; withProject();
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: () => void;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "terminal_open"
    ? new Promise<void>(resolve => { finish = resolve; }) : base(command, args, options));
  render(<App />); await screen.findByText("连接中…");
  const input = screen.getByLabelText("终端输入");
  fireEvent.click(screen.getByRole("button", { name: "最大化终端" }));
  finish(); await screen.findByText("已连接");
  fireEvent.click(screen.getByRole("button", { name: "恢复终端" }));
  expect(screen.getByLabelText("终端输入")).toBe(input);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_open")).toHaveLength(1);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_close")).toHaveLength(0);
});

it.each(["empty", "disconnected"])("%s 状态始终可恢复布局，切换本身不重试连接", async mode => {
  if (mode === "disconnected") { bindTask(); board.storedTask.status = "in_progress"; withProject(); }
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "terminal_open"
    ? Promise.reject("terminal.closed") : base(command, args, options));
  render(<App />);
  if (mode === "disconnected") await screen.findByText("连接已断开");
  const before = vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_open").length;
  fireEvent.click(screen.getByRole("button", { name: "最大化终端" }));
  fireEvent.click(screen.getByRole("button", { name: "恢复终端" }));
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_open")).toHaveLength(before);
  expect(screen.getByRole("button", { name: "最大化终端" })).toBeTruthy();
});
