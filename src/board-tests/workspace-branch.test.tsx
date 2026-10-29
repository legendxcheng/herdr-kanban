// @vitest-environment jsdom
import { bindTask, board, project } from "./fixtures";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";
import type { ProjectGitStatus } from "../types";

function setup() {
  const state = { projects: [{ ...project }], head: "feature/sidebar-current-branch", offline: false };
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => {
    if (cmd === "list_projects") return state.projects.map(p => ({ ...p }));
    if (cmd === "sync_projects") {
      if (state.offline) throw new Error("herdr.unavailable");
      return { session: "default", projects: state.projects.map(p => ({ ...p })), operations: [], workspaces: [] };
    }
    if (cmd === "workspace_operations") return [];
    if (cmd === "project_git_statuses") return state.projects.map(p => ({ project_id: p.id, root_path: p.root_path, state: "branch", head: state.head }));
    return base(cmd, args, opts);
  });
  return state;
}
const label = () => document.querySelector(".project-ws")!;

it("侧栏显示分支和完整悬浮提示，分支切换保持项目、卡片及终端连接", async () => {
  const state = setup(); bindTask(); board.storedTask.status = "in_progress";
  render(<App />);
  await waitFor(() => expect(label().textContent).toBe(state.head));
  expect(label().getAttribute("title")).toBe(`${state.head}\n${project.root_path}`);
  expect(document.querySelector(".project-name")?.textContent).toBe(project.name);
  expect(label().textContent).not.toContain(project.root_path);
  await screen.findByText("Herdr 已打开");
  state.head = "feature/切换后的分支";
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(label().textContent).toBe(state.head));
  expect(document.querySelector('.project-row[aria-current="page"]')).toBeTruthy();
  expect(screen.getByText(board.storedTask.name, { selector: ".task-name" })).toBeTruthy();
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_open")).toHaveLength(1);
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_close")).toHaveLength(0);
});

it("Herdr 离线时焦点和手动刷新仍读取本地分支", async () => {
  const state = setup(); render(<App />);
  await waitFor(() => expect(label().textContent).toBe(state.head));
  state.offline = true; state.head = "feature/offline-focus";
  fireEvent(window, new Event("focus"));
  await screen.findByText(/同步失败/);
  await waitFor(() => expect(label().textContent).toBe(state.head));
  state.head = "feature/manual-refresh";
  fireEvent.click(screen.getByRole("button", { name: "重试连接" }));
  await waitFor(() => expect(label().textContent).toBe(state.head));
});

it("前台每 15 秒读取分支，后台定时器不发起查询", async () => {
  vi.useFakeTimers();
  const state = setup();
  await act(async () => { render(<App />); });
  expect(label().textContent).toBe(state.head);
  state.offline = true; state.head = "feature/timer-refresh";
  await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
  expect(label().textContent).toBe(state.head);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  state.head = "feature/background";
  await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
  expect(label().textContent).toBe("feature/timer-refresh");
});

it("Herdr 查询尚未返回时，Git 定时刷新仍继续", async () => {
  vi.useFakeTimers();
  const state = setup();
  const base = vi.mocked(invoke).getMockImplementation()!;
  let waiting = false;
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => {
    if (cmd === "sync_projects" && waiting) return new Promise(() => {});
    return base(cmd, args, opts);
  });
  await act(async () => { render(<App />); });
  waiting = true;
  await act(async () => { fireEvent(window, new Event("focus")); });
  state.head = "feature/herdr-still-waiting";
  await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
  expect(label().textContent).toBe(state.head);
});

it("游离 HEAD、非 Git、查询失败及恢复使用明确文案，保留导航", async () => {
  setup();
  const base = vi.mocked(invoke).getMockImplementation()!;
  let status: ProjectGitStatus = { project_id: project.id, root_path: project.root_path, state: "detached", head: "abc1234" };
  let failed = false;
  vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => {
    if (cmd === "project_git_statuses") {
      if (failed) throw new Error("unavailable");
      return [{ ...status }];
    }
    return base(cmd, args, opts);
  });
  render(<App />);
  await waitFor(() => expect(label().textContent).toBe("游离 HEAD · abc1234"));
  status = { ...status, state: "not_repository", head: null };
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(label().textContent).toBe("非 Git 仓库"));
  failed = true; fireEvent(window, new Event("focus"));
  await waitFor(() => expect(label().textContent).toBe("分支不可用"));
  failed = false; status = { ...status, state: "branch", head: "main" };
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(label().textContent).toBe("main"));
  expect(document.querySelectorAll(".project-name")).toHaveLength(1);
});

it("慢查询期间路径变化与连续焦点只执行最新排队查询，迟到分支不覆盖新路径", async () => {
  const state = setup();
  const base = vi.mocked(invoke).getMockImplementation()!;
  const requests: { root: string; resolve: (value: ProjectGitStatus[]) => void }[] = [];
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => {
    if (cmd === "project_git_statuses") return new Promise<ProjectGitStatus[]>(resolve => requests.push({ root: state.projects[0].root_path, resolve }));
    return base(cmd, args, opts);
  });
  render(<App />);
  await waitFor(() => expect(requests).toHaveLength(1));
  state.projects = [{ ...project, root_path: "E:/new-worktree" }];
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(label().getAttribute("title")).toContain("E:/new-worktree"));
  fireEvent(window, new Event("focus")); fireEvent(window, new Event("focus"));
  await act(async () => {});
  expect(requests).toHaveLength(1);
  await act(async () => { requests[0].resolve([{ project_id: project.id, root_path: requests[0].root, state: "branch", head: "old-branch" }]); });
  await waitFor(() => expect(requests).toHaveLength(2));
  expect(label().textContent).not.toContain("old-branch");
  await act(async () => { requests[1].resolve([{ project_id: project.id, root_path: requests[1].root, state: "branch", head: "new-worktree" }]); });
  await waitFor(() => expect(label().textContent).toBe("new-worktree"));
});
