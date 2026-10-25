// @vitest-environment jsdom
import { bindTask, board, project } from "./fixtures";
import { act, createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";

const second = { ...project, id: "project-2", workspace_id: "w2", name: "Workspace B" };
const third = { ...project, id: "project-3", workspace_id: "w3", name: "Workspace C" };
const names = () => Array.from(document.querySelectorAll(".project-name"), el => el.textContent);
function setup() {
  const state = { projects: [project, second, third] };
  const base = vi.mocked(invoke).getMockImplementation()!;
  const snapshot = () => ({ session: "default", projects: [...state.projects], workspaces: [], operations: [] });
  vi.mocked(invoke).mockImplementation(async (command, args, options) => {
    const input = args as { projectId: string; beforeProjectId: string | null };
    if (command === "list_projects") return [...state.projects];
    if (command === "sync_projects") return snapshot();
    if (command === "move_project") {
      const moved = state.projects.find(p => p.id === input.projectId)!;
      state.projects = state.projects.filter(p => p.id !== moved.id);
      const index = input.beforeProjectId === null ? state.projects.length : state.projects.findIndex(p => p.id === input.beforeProjectId);
      state.projects.splice(index, 0, moved);
      return snapshot();
    }
    if (command === "list_tasks" && input.projectId !== project.id) return [];
    return base(command, args, options);
  });
  return { state, snapshot };
}
function row(id: string) { return document.querySelector(`[data-project-id="${id}"]`)!; }
function drag(source: string, target: string, after = false) {
  const transfer = { setData: vi.fn(), effectAllowed: "move", dropEffect: "move" };
  vi.spyOn(row(target), "getBoundingClientRect").mockReturnValue({ top: 0, height: 50 } as DOMRect);
  fireEvent.dragStart(row(source), { dataTransfer: transfer });
  for (const type of ["dragOver", "drop"] as const) {
    const event = createEvent[type](row(target), { dataTransfer: transfer });
    Object.defineProperty(event, "clientY", { value: after ? 40 : 10 });
    fireEvent(row(target), event);
  }
  fireEvent.dragEnd(row(source), { dataTransfer: transfer });
}
it("拖动可上移和移至末尾，保持项目选择、任务及终端连接，外部重排继续同步", async () => {
  const { state } = setup(); bindTask(); board.storedTask.status = "in_progress";
  render(<App />); await screen.findByText("已连接");
  drag(third.id, project.id);
  await waitFor(() => expect(names()).toEqual([third.name, project.name, second.name]));
  expect(invoke).toHaveBeenCalledWith("move_project", { projectId: third.id, beforeProjectId: project.id });
  expect(document.querySelector('.project-row[aria-current="page"]')?.textContent).toContain(project.name);
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_open")).toHaveLength(1);
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_close")).toHaveLength(0);
  await waitFor(() => expect(row(third.id).getAttribute("draggable")).toBe("true"));
  drag(third.id, second.id, true);
  await waitFor(() => expect(names()).toEqual([project.name, second.name, third.name]));
  expect(invoke).toHaveBeenCalledWith("move_project", { projectId: third.id, beforeProjectId: null });
  state.projects = [second, third, project];
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(names()).toEqual([second.name, third.name, project.name]));
});
it("相同位置、外来拖拽、未关联项目和其他 session 不发送重排", async () => {
  const { state } = setup();
  state.projects = [project, second, { ...third, workspace_id: null, sync_status: "unlinked" }, { ...third, id: "other-session", herdr_session: "other" }];
  render(<App />); await waitFor(() => expect(row(project.id).getAttribute("draggable")).toBe("true"));
  drag(project.id, second.id);
  fireEvent.drop(row(second.id));
  expect(row(third.id).getAttribute("draggable")).toBe("false");
  expect(row("other-session").getAttribute("draggable")).toBe("false");
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "move_project")).toHaveLength(0);
});
it("请求超时后按实际顺序恢复，不重新发送移动", async () => {
  setup();
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => {
    const result = await base(cmd, args, opts);
    if (cmd === "move_project") throw new Error("herdr.timeout");
    return result;
  });
  render(<App />); await waitFor(() => expect(row(third.id).getAttribute("draggable")).toBe("true"));
  drag(third.id, project.id);
  await screen.findByText(/顺序调整未确认/);
  await waitFor(() => expect(names()).toEqual([third.name, project.name, second.name]));
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "move_project")).toHaveLength(1);
});
it("迟到的刷新不覆盖新顺序，保存中阻止重复拖动，拖动后的点击不切换项目", async () => {
  const { snapshot } = setup();
  const base = vi.mocked(invoke).getMockImplementation()!;
  let reads = 0;
  let finishOld!: (value: unknown) => void;
  let finishMove!: () => void;
  vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => {
    if (cmd === "sync_projects" && ++reads === 2) return new Promise(resolve => { finishOld = resolve; });
    if (cmd === "move_project") await new Promise<void>(resolve => { finishMove = resolve; });
    return base(cmd, args, opts);
  });
  render(<App />); await waitFor(() => expect(row(third.id).getAttribute("draggable")).toBe("true"));
  const old = snapshot();
  fireEvent(window, new Event("focus")); await act(async () => {});
  drag(third.id, project.id);
  await screen.findByText("正在同步工作空间顺序…");
  expect(row(third.id).getAttribute("draggable")).toBe("false");
  fireEvent.click(row(third.id).querySelector(".project-row")!);
  expect(document.querySelector('.project-row[aria-current="page"]')?.textContent).toContain(project.name);
  await act(async () => { finishMove(); });
  await waitFor(() => expect(names()).toEqual([third.name, project.name, second.name]));
  await act(async () => { finishOld(old); });
  await waitFor(() => expect(reads).toBe(3));
  expect(names()).toEqual([third.name, project.name, second.name]);
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "move_project")).toHaveLength(1);
});
