// @vitest-environment jsdom
import { board, project, bindTask } from "./fixtures";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";

it("synchronizes Herdr workspaces on focus while retaining local tasks on failure", async () => {
  let offline = false;
  const original = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "list_projects") return [project];
    if (command === "sync_projects") {
      if (offline) throw new Error("herdr.unavailable");
      return { projects: [{ ...project, name: "Synced name" }], workspaces: [], operations: [] };
    }
    if (command === "workspace_operations") return [];
    return original(command, args);
  });
  render(<App />);
  await screen.findByRole("button", { name: /^Synced name/ });
  offline = true;
  fireEvent(window, new Event("focus"));
  await screen.findByText(/同步失败/);
  expect(screen.getByText("修复拖放派发")).toBeTruthy();
});

describe("workspace 导航同步", () => {
  const other = { ...project, id: "project-navigation-2", workspace_id: "w2", name: "第二工作空间" };
  const names = () => Array.from(document.querySelectorAll(".project-name")).map(el => el.textContent);

  it("Herdr 重排和关闭非当前 workspace 保持选择及当前终端，关闭当前项释放连接并回退首项", async () => {
    bindTask(); board.storedTask.status = "in_progress";
    let projects = [project, other];
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, args, options) => {
      if (command === "list_projects") return Promise.resolve([project, other]);
      if (command === "sync_projects") return Promise.resolve({ projects: [...projects], operations: [], workspaces: [] });
      if (command === "list_tasks" && (args as { projectId: string }).projectId === other.id) return Promise.resolve([]);
      return base(command, args, options);
    });
    render(<App />); await screen.findByText("已连接");
    const connection = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "terminal_open")![1] as { connectionId: string };
    projects = [other, project];
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(names()).toEqual([other.name, project.name]));
    expect(document.querySelector('.project-row[aria-current="page"]')?.textContent).toContain(project.name);
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_open")).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalledWith("terminal_close", { connectionId: connection.connectionId });
    projects = [project];
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(names()).toEqual([project.name]));
    expect(invoke).not.toHaveBeenCalledWith("terminal_close", { connectionId: connection.connectionId });
    projects = [other];
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(names()).toEqual([other.name]));
    await waitFor(() => expect(document.querySelector('.project-row[aria-current="page"]')?.textContent).toContain(other.name));
    expect(invoke).toHaveBeenCalledWith("terminal_close", { connectionId: connection.connectionId });
    projects = [];
    fireEvent(window, new Event("focus"));
    await screen.findByText("从一个项目，开始推进。");
    expect(names()).toEqual([]);
    expect(screen.queryByText(board.storedTask.name)).toBeNull();
  });

  it("慢速焦点刷新与离线错误不恢复旧缓存，显式本地变更仍可离线刷新", async () => {
    let cached = [project, other];
    let delayed = false;
    let reject!: (error: Error) => void;
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, args, options) => {
      if (command === "list_projects") return Promise.resolve(cached);
      if (command === "sync_projects") return delayed
        ? new Promise((_resolve, fail) => { reject = fail; })
        : Promise.resolve({ projects: [other], operations: [], workspaces: [] });
      if (command === "workspace_operations") return Promise.resolve([]);
      return base(command, args, options);
    });
    render(<App />);
    await waitFor(() => expect(names()).toEqual([other.name]));
    delayed = true;
    fireEvent(window, new Event("focus"));
    await act(async () => {});
    expect(names()).toEqual([other.name]);
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "list_projects")).toHaveLength(1);
    await act(async () => { reject(new Error("herdr.unavailable")); });
    await screen.findByText(/同步失败/);
    expect(names()).toEqual([other.name]);
    cached = [];
    fireEvent.click(screen.getByRole("button", { name: "同步工作空间" }));
    await waitFor(() => expect(names()).toEqual([]));
    await act(async () => { reject(new Error("herdr.unavailable")); });
    expect(names()).toEqual([]);
  });

  it("焦点请求合并后丢弃旧快照，不让已关闭项目重新出现", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    let requests = 0;
    let resolve!: (value: unknown) => void;
    vi.mocked(invoke).mockImplementation((command, args, options) => {
      if (command === "list_projects") return Promise.resolve([project]);
      if (command === "sync_projects") {
        requests++;
        return requests === 1 ? Promise.resolve({ projects: [project], operations: [], workspaces: [] })
          : new Promise(done => { resolve = done; });
      }
      return base(command, args, options);
    });
    render(<App />);
    await screen.findByRole("button", { name: /^测试项目/ });
    await act(async () => {});
    fireEvent(window, new Event("focus"));
    fireEvent(window, new Event("focus"));
    expect(requests).toBe(2);
    await act(async () => { resolve({ projects: [other], operations: [], workspaces: [] }); });
    expect(names()).toEqual([project.name]);
    expect(requests).toBe(3);
    await act(async () => { resolve({ projects: [], operations: [], workspaces: [] }); });
    expect(names()).toEqual([]);
    await screen.findByText("从一个项目，开始推进。");
  });
});
