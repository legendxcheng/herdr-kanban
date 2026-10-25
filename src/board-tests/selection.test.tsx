// @vitest-environment jsdom
import { board, project, sendPrompt, renderBoard, column, dropOn, expectNoStatusWrite, bindTask, secondTask } from "./fixtures";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { KanbanBoard } from "../KanbanBoard";
import type { Task, TaskStatus } from "../types";
import App from "../App";

describe("默认终端与任务选择", () => {
  it("启动自动选择第一张开发中任务，手动选择后刷新不抢回或重连", async () => {
    bindTask();
    board.extraTasks = [secondTask()];
    await renderBoard("in_progress");
    await screen.findByText("Herdr 已打开");
    expect(screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "查看终端：任务 B" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: "task-2" })));
    const opened = vi.mocked(invoke).mock.calls.filter(([name]) => name === "terminal_open").length;
    fireEvent.click(within(column("in_progress")).getAllByRole("button", { name: "编辑" })[0]);
    fireEvent.click(await screen.findByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "保存" })).toBeNull());
    expect(screen.getByRole("button", { name: "查看终端：任务 B" }).getAttribute("aria-pressed")).toBe("true");
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "terminal_open")).toHaveLength(opened);
    expect(sendPrompt).not.toHaveBeenCalled();
    expectNoStatusWrite();
  });

  it.each(["done", "todo", "delete"])("选中任务变为 %s 后释放旧连接并选择下一张", async (action) => {
    bindTask(); board.extraTasks = [secondTask()];
    await renderBoard("in_progress");
    await screen.findByText("Herdr 已打开");
    const firstOpen = vi.mocked(invoke).mock.calls.find(([name]) => name === "terminal_open")![1] as { connectionId: string };
    if (action === "delete") {
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
      fireEvent.click(within(column("in_progress")).getAllByRole("button", { name: "删除" })[0]);
      confirm.mockRestore();
    } else dropOn(column(action as TaskStatus));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: "task-2" })));
    expect(invoke).toHaveBeenCalledWith("terminal_close", { connectionId: firstOpen.connectionId });
    expect(screen.getByRole("button", { name: "查看终端：任务 B" }).getAttribute("aria-pressed")).toBe("true");
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("最后一张开发中任务完成后保留空终端", async () => {
    bindTask(); await renderBoard("in_progress"); await screen.findByText("Herdr 已打开");
    fireEvent.click(screen.getByRole("button", { name: "标记完成" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("暂无开发中的任务"));
    expect(screen.getByRole("complementary", { name: "Herdr 终端" })).toBeTruthy();
    expect(screen.queryByLabelText("终端输入")).toBeNull();
    expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "terminal_close")).toBe(true);
  });

  it("加载期间和没有开发中任务时保留空终端，不连接或派发", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    let finish!: (tasks: Task[]) => void;
    vi.mocked(invoke).mockImplementation((command, args, options) => command === "list_tasks"
      ? new Promise((resolve) => { finish = resolve; }) : base(command, args, options));
    render(<KanbanBoard project={project} />);
    expect(screen.getByText("加载中…")).toBeTruthy();
    expect(screen.getByRole("complementary", { name: "Herdr 终端" })).toBeTruthy();
    finish([board.storedTask]);
    await screen.findByText(board.storedTask.name, { selector: ".task-name" });
    expect(screen.getByRole("status").textContent).toBe("暂无开发中的任务");
    expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "terminal_open")).toBe(false);
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("首张任务绑定异常仍选中并展示修复入口，不跳到后续任务", async () => {
    bindTask(); board.extraTasks = [secondTask()]; board.storedTask.binding = { state: "repair", operation: null, target: null };
    await renderBoard("in_progress");
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("请先修复任务绑定"));
    expect(screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` }).getAttribute("aria-pressed")).toBe("true");
    expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "terminal_open")).toBe(false);
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("没有项目时仍显示空终端", async () => {
    render(<App />);
    await screen.findByText("从一个项目，开始推进。");
    expect(screen.getByRole("complementary", { name: "Herdr 终端" })).toBeTruthy();
    expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "terminal_open")).toBe(false);
  });

  it("切项目自动选择新项目任务并释放原连接", async () => {
    bindTask(); board.storedTask.status = "in_progress";
    const other = { ...project, id: "project-2", name: "第二项目" };
    const taskB = { ...secondTask(), project_id: other.id };
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, args, options) => {
      if (command === "list_projects") return Promise.resolve([project, other]);
      if (command === "list_tasks" && (args as { projectId: string }).projectId === other.id) return Promise.resolve([taskB]);
      return base(command, args, options);
    });
    render(<App />); await screen.findByText("Herdr 已打开");
    const first = vi.mocked(invoke).mock.calls.find(([name]) => name === "terminal_open")![1] as { connectionId: string };
    fireEvent.click(screen.getByRole("button", { name: /^第二项目/ }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: taskB.id })));
    expect(invoke).toHaveBeenCalledWith("terminal_close", { connectionId: first.connectionId });
    expect(screen.getByRole("button", { name: "查看终端：任务 B" }).getAttribute("aria-pressed")).toBe("true");
  });
});
