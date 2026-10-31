// @vitest-environment jsdom
import { bindTask, board, flowEvents, project, secondTask, sendPrompt } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";
import { KanbanBoard, TaskBoard } from "../KanbanBoard";
import { readWorkspaceTaskSelection, rememberWorkspaceTaskSelection } from "../workspaceTaskSelection";
import type { Task } from "../types";

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);
const lastCall = (command: string) => calls(command).slice(-1)[0];
const choose = (name: string) => fireEvent.click(screen.getByRole("button", { name: `查看终端：${name}` }));
const refresh = async () => act(async () => {
  for (const callback of flowEvents) callback({ payload: { project_id: project.id, task_id: "task-1", operation_id: "refresh" } });
});
function twoRunningTasks() {
  bindTask(); board.storedTask.status = "in_progress"; board.extraTasks = [secondTask()];
}

it("restores each Workspace's last choice on A/B/A switches and App remount", async () => {
  twoRunningTasks();
  const other = { ...project, id: "project-2", name: "第二项目" };
  const otherTasks = [
    { ...secondTask(), id: "other-1", project_id: other.id, name: "项目二第一项" },
    { ...secondTask(), id: "other-2", project_id: other.id, name: "项目二第二项" },
  ];
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "list_projects") return Promise.resolve([project, other]);
    if (command === "list_tasks" && (args as { projectId: string }).projectId === other.id) return Promise.resolve(otherTasks);
    return base(command, args, options);
  });
  const app = render(<App />); await screen.findByText("Herdr 已打开");
  choose("任务 B");
  await waitFor(() => expect(readWorkspaceTaskSelection(project.id)).toBe("task-2"));
  fireEvent.click(screen.getByRole("button", { name: /^第二项目/ }));
  await screen.findByText("项目二第二项", { selector: ".task-name" }); choose("项目二第二项");
  await waitFor(() => expect(readWorkspaceTaskSelection(other.id)).toBe("other-2"));
  const beforeReturn = calls("terminal_open").length;
  fireEvent.click(screen.getByRole("button", { name: /^测试项目/ }));
  await waitFor(() => expect(calls("terminal_open")).toHaveLength(beforeReturn + 1));
  expect(lastCall("terminal_open")[1]).toMatchObject({ taskId: "task-2" });
  fireEvent.click(screen.getByRole("button", { name: /^第二项目/ }));
  await waitFor(() => expect(lastCall("terminal_open")[1]).toMatchObject({ taskId: "other-2" }));
  app.unmount();
  const beforeReload = calls("terminal_open").length;
  render(<App />);
  await waitFor(() => expect(calls("terminal_open")).toHaveLength(beforeReload + 1));
  expect(lastCall("terminal_open")[1]).toMatchObject({ taskId: "task-2" });
  expect(sendPrompt).not.toHaveBeenCalled();
  expect(calls("start_task")).toHaveLength(0);
});

it("restores Todo editing without connecting the first running task, then remembers explicit close", async () => {
  bindTask(); board.extraTasks = [secondTask()];
  rememberWorkspaceTaskSelection(project.id, board.storedTask.id);
  const app = render(<KanbanBoard project={project} />);
  const panel = await screen.findByRole("complementary", { name: "编辑任务" });
  expect((within(panel).getByRole("textbox", { name: "标题" }) as HTMLInputElement).value).toBe(board.storedTask.name);
  await refresh();
  expect(calls("terminal_open")).toHaveLength(0);
  expect(readWorkspaceTaskSelection(project.id)).toBe(board.storedTask.id);
  fireEvent.click(within(panel).getAllByRole("button", { name: "关闭" })[0]);
  await screen.findByText("Herdr 已打开");
  expect(readWorkspaceTaskSelection(project.id)).toBe("task-2");
  app.unmount(); render(<KanbanBoard project={project} />);
  await screen.findByText("Herdr 已打开");
  expect(screen.queryByRole("complementary", { name: "编辑任务" })).toBeNull();
});

it.each(["deleted", "done", "archived", "wrong-project"])("falls back when the remembered task is %s", async reason => {
  twoRunningTasks(); rememberWorkspaceTaskSelection(project.id, "task-2");
  if (reason === "deleted") board.extraTasks = [];
  if (reason === "done") board.extraTasks[0].status = "done";
  if (reason === "archived") board.extraTasks[0].archived_at = "2026-10-31";
  if (reason === "wrong-project") board.extraTasks[0].project_id = "other-project";
  render(<KanbanBoard project={project} />); await screen.findByText("Herdr 已打开");
  expect(calls("terminal_open")).toHaveLength(1);
  expect(calls("terminal_open")[0][1]).toMatchObject({ taskId: board.storedTask.id });
  expect(readWorkspaceTaskSelection(project.id)).toBe(board.storedTask.id);
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("clears an invalid choice after a successful empty read", async () => {
  board.deleted = true; rememberWorkspaceTaskSelection(project.id, "gone");
  render(<KanbanBoard project={project} />);
  await screen.findByRole("button", { name: /新建任务/ });
  await waitFor(() => expect(readWorkspaceTaskSelection(project.id)).toBeNull());
  expect(calls("terminal_open")).toHaveLength(0);
});

it.each(["repair", "preparing"])("restores the selected task even when its terminal is %s", async state => {
  twoRunningTasks(); rememberWorkspaceTaskSelection(project.id, "task-2");
  if (state === "repair") board.extraTasks[0].binding = { state: "repair", operation: null, target: null };
  else {
    board.extraTasks[0].status = "todo";
    board.extraTasks[0].flow = { session_id: null, operation_id: "starting", kind: "start", state: "pending",
      error: "", pane_id: null, stage: "queued", requested_status: "in_progress" };
  }
  render(<KanbanBoard project={project} />);
  await screen.findByText("任务 B", { selector: ".task-name" });
  expect(screen.getByRole("button", { name: "查看终端：任务 B" }).getAttribute("aria-pressed")).toBe("true");
  expect(calls("terminal_open")).toHaveLength(0);
  expect(calls("start_task")).toHaveLength(0);
  expect(screen.queryByRole("complementary", { name: "编辑任务" })).toBeNull();
  expect(readWorkspaceTaskSelection(project.id)).toBe("task-2");
});

it("preserves a remembered choice through a failed initial read and restores after a slow retry", async () => {
  twoRunningTasks(); rememberWorkspaceTaskSelection(project.id, "task-2");
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: (tasks: Task[]) => void;
  let failed = false;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command !== "list_tasks") return base(command, args, options);
    if (!failed) { failed = true; return Promise.reject(new Error("offline")); }
    return new Promise(resolve => { finish = resolve; });
  });
  render(<KanbanBoard project={project} />);
  await screen.findByRole("button", { name: /新建任务/ });
  expect(readWorkspaceTaskSelection(project.id)).toBe("task-2");
  await refresh();
  expect(calls("terminal_open")).toHaveLength(0);
  expect(readWorkspaceTaskSelection(project.id)).toBe("task-2");
  await act(async () => { finish([board.storedTask, ...board.extraTasks]); });
  await screen.findByText("Herdr 已打开");
  expect(calls("terminal_open")).toHaveLength(1);
  expect(calls("terminal_open")[0][1]).toMatchObject({ taskId: "task-2" });
});

it("ignores a late old Workspace read without changing either saved choice", async () => {
  twoRunningTasks(); rememberWorkspaceTaskSelection(project.id, "task-2");
  const other = { ...project, id: "other", name: project.name, machine_id: "remote" };
  rememberWorkspaceTaskSelection(other.id, "other-task");
  const otherTask = { ...secondTask(), id: "other-task", project_id: other.id };
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: (tasks: Task[]) => void;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "list_tasks"
    ? (args as { projectId: string }).projectId === project.id
      ? new Promise(resolve => { finish = resolve; }) : Promise.resolve([otherTask])
    : base(command, args, options));
  const view = render(<KanbanBoard key={project.id} project={project} />);
  view.rerender(<KanbanBoard key={other.id} project={other} />);
  await screen.findByText("Herdr 已打开");
  await act(async () => { finish([board.storedTask, ...board.extraTasks]); });
  expect(calls("terminal_open")).toHaveLength(1);
  expect(calls("terminal_open")[0][1]).toMatchObject({ taskId: "other-task" });
  expect(readWorkspaceTaskSelection(project.id)).toBe("task-2");
  expect(readWorkspaceTaskSelection(other.id)).toBe("other-task");
});

it("ordinary shell and All Tasks do not replace the Workspace's remembered Todo", async () => {
  bindTask(); board.extraTasks = [secondTask()];
  rememberWorkspaceTaskSelection(project.id, board.storedTask.id);
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "workspace_terminal_open") return Promise.resolve((args as { connectionId: string }).connectionId);
    if (command === "board_tasks") return Promise.resolve([board.storedTask, ...board.extraTasks]);
    return base(command, args, options);
  });
  const view = render(<KanbanBoard project={project} />);
  await screen.findByRole("complementary", { name: "编辑任务" });
  fireEvent.click(screen.getByRole("button", { name: "打开终端" }));
  await screen.findByText("Herdr 已打开"); await refresh();
  expect(readWorkspaceTaskSelection(project.id)).toBe(board.storedTask.id);
  view.unmount();
  const summary = render(<TaskBoard projects={[project]} />);
  await screen.findByText("任务 B", { selector: ".task-name" }); choose("任务 B");
  await screen.findByText("Herdr 已打开");
  expect(readWorkspaceTaskSelection(project.id)).toBe(board.storedTask.id);
  summary.unmount(); render(<KanbanBoard project={project} />);
  await screen.findByRole("complementary", { name: "编辑任务" });
  expect(sendPrompt).not.toHaveBeenCalled();
});
