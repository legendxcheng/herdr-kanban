// @vitest-environment jsdom
import { bindTask, board, project, sendPrompt } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { KanbanBoard, TaskBoard } from "../KanbanBoard";

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);
function mockShell(open: (args: Record<string, unknown>) => Promise<unknown> = async args => args.connectionId) {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "workspace_terminal_open"
    ? open(args as Record<string, unknown>) : base(command, args, options));
}

it("opens an empty Workspace shell once and keeps it through refresh and repeated clicks", async () => {
  board.deleted = true; mockShell();
  render(<KanbanBoard project={project} />);
  const button = await screen.findByRole("button", { name: "打开终端" });
  fireEvent.click(button); await screen.findByText("Herdr 已打开");
  fireEvent.click(button); fireEvent(window, new Event("focus"));
  await act(async () => {});
  expect(calls("workspace_terminal_open")).toHaveLength(1);
  expect(calls("workspace_terminal_open")[0][1]).toMatchObject({ projectId: project.id });
  expect(calls("terminal_open")).toHaveLength(0);
  expect(sendPrompt).not.toHaveBeenCalled();
  expect(button.getAttribute("aria-pressed")).toBe("true");
});

it("switches between task and shell without dispatch and polling does not select the task", async () => {
  board.storedTask.status = "in_progress"; bindTask(); mockShell();
  render(<KanbanBoard project={project} />); await screen.findByText("Herdr 已打开");
  fireEvent.click(screen.getByRole("button", { name: "打开终端" }));
  await screen.findByText(`${project.name} · 命令行`);
  await screen.findByText("Herdr 已打开");
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect(calls("terminal_open")).toHaveLength(1);
  fireEvent.click(document.querySelector(".task-name")!);
  await waitFor(() => expect(calls("terminal_open")).toHaveLength(2));
  fireEvent.click(screen.getByRole("button", { name: "打开终端" }));
  await waitFor(() => expect(calls("workspace_terminal_open")).toHaveLength(2));
  expect(sendPrompt).not.toHaveBeenCalled();
});

it("routes remote Workspace by project ID and offers reconnect after a failure", async () => {
  board.deleted = true;
  let fail = true;
  mockShell(async () => { if (fail) throw new Error("machine.timeout"); });
  const remote = { ...project, id: "remote-project", machine_id: "remote" };
  render(<KanbanBoard project={remote} machineName="devpc" />);
  fireEvent.click(await screen.findByRole("button", { name: "打开终端" }));
  await screen.findByText("连接已断开");
  expect(calls("workspace_terminal_open")[0][1]).toMatchObject({ projectId: "remote-project" });
  expect(document.querySelector(".terminal-machine")?.textContent).toBe("devpc");
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "重新连接" }));
  await screen.findByText("Herdr 已打开");
  expect(calls("workspace_terminal_open")).toHaveLength(2);
  expect(calls("terminal_open")).toHaveLength(0);
});

it("ignores late shell completion after selecting another task", async () => {
  board.storedTask.status = "in_progress"; bindTask();
  let finish!: () => void;
  mockShell(() => new Promise<void>(resolve => { finish = resolve; }));
  render(<KanbanBoard project={project} />); await screen.findByText("Herdr 已打开");
  fireEvent.click(screen.getByRole("button", { name: "打开终端" })); await screen.findByText("连接中…");
  const shell = calls("workspace_terminal_open")[0][1] as { connectionId: string };
  fireEvent.click(document.querySelector(".task-name")!); await screen.findByText("Herdr 已打开");
  await act(async () => finish());
  expect(screen.queryByText(`${project.name} · 命令行`)).toBeNull();
  expect(invoke).toHaveBeenCalledWith("terminal_close", { connectionId: shell.connectionId });
  expect(calls("terminal_open")).toHaveLength(2);
});

it("does not expose OpenTerminal in All Tasks", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command,args,options) => command === "board_tasks" ? Promise.resolve([]) : base(command,args,options));
  render(<TaskBoard projects={[project]} />);
  await screen.findByRole("button", { name: /新建任务/ });
  expect(screen.queryByRole("button", { name: "打开终端" })).toBeNull();
});

it("protects an unsaved Todo draft before opening the shell", async () => {
  mockShell(); render(<KanbanBoard project={project} />);
  fireEvent.click(await screen.findByRole("button", { name: `查看并编辑：${board.storedTask.name}` }));
  const panel = screen.getByRole("complementary", { name: "编辑任务" });
  fireEvent.change(within(panel).getByRole("textbox", { name: "标题" }), { target: { value: "保留草稿" } });
  fireEvent.click(screen.getByRole("button", { name: "打开终端" }));
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect((within(panel).getByRole("textbox", { name: "标题" }) as HTMLInputElement).value).toBe("保留草稿");
  expect(calls("workspace_terminal_open")).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "打开终端" }));
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  await screen.findByText("Herdr 已打开");
  expect(board.storedTask.name).toBe("保留草稿");
  expect(calls("workspace_terminal_open")).toHaveLength(1);
});

it("keeps the shell connection across maximize and restore", async () => {
  board.deleted = true; mockShell();
  const props = { project, onToggleTerminalMaximized: vi.fn() };
  const view = render(<KanbanBoard {...props} />);
  fireEvent.click(await screen.findByRole("button", { name: "打开终端" }));
  await screen.findByText("Herdr 已打开");
  view.rerender(<KanbanBoard {...props} terminalMaximized />);
  await screen.findByRole("button", { name: "恢复终端" });
  view.rerender(<KanbanBoard {...props} terminalMaximized={false} />);
  expect(calls("workspace_terminal_open")).toHaveLength(1);
  expect(calls("terminal_close")).toHaveLength(0);
});
