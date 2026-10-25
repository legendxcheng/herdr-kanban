// @vitest-environment jsdom
import { board, column, flowEvents, project, renderBoard, sendPrompt } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { KanbanBoard } from "../KanbanBoard";
import type { BatchTaskResult } from "../api";

afterEach(() => { document.documentElement.lang = "zh-CN"; });

function removeStored(ids: string[]) {
  if (ids.includes(board.storedTask.id)) board.deleted = true;
  board.extraTasks = board.extraTasks.filter(task => !ids.includes(task.id));
}

describe("已完成区域批量操作", () => {
  it("入口只在完成列，空列禁用且不提交", async () => {
    await renderBoard();
    for (const name of ["一键归档", "一键删除"]) {
      const button = within(column("done")).getByRole("button", { name });
      expect(button.matches(":disabled")).toBe(true);
      fireEvent.click(button);
      expect(within(column("todo")).queryByRole("button", { name })).toBeNull();
      expect(within(column("in_progress")).queryByRole("button", { name })).toBeNull();
    }
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd.endsWith("completed_tasks"))).toBe(false);
  });

  it("一键归档只提交当前完成列的快照，不确认、不派发或连接终端", async () => {
    board.extraTasks = [{ ...board.storedTask, id: "done-2", name: "完成 B", status: "done" },
      { ...board.storedTask, id: "todo", name: "未开发", status: "todo" }];
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (cmd, args, options) => {
      if (cmd !== "archive_completed_tasks") return base(cmd, args, options);
      const ids = (args as { taskIds: string[] }).taskIds;
      removeStored(ids);
      return { succeeded: ids, skipped: [], failed: [] };
    });
    const confirm = vi.spyOn(window, "confirm");
    await renderBoard("done");
    fireEvent.click(screen.getByRole("button", { name: "一键归档" }));
    expect(await screen.findByText("归档完成：成功 2，跳过 0，失败 0")).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith("archive_completed_tasks", { projectId: project.id, taskIds: ["task-1", "done-2"] });
    expect(screen.getByText("0 / 1 已完成")).toBeTruthy();
    expect(screen.queryByText("完成 B")).toBeNull();
    expect(confirm).not.toHaveBeenCalled();
    expect(sendPrompt).not.toHaveBeenCalled();
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_open")).toBe(false);
  });

  it("删除只确认一次，取消不提交，在途防重复且后来完成的任务不加入快照", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    let finish!: (result: BatchTaskResult) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "delete_completed_tasks"
      ? new Promise(resolve => { finish = resolve; }) : base(cmd, args, options));
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await renderBoard("done");
    const button = screen.getByRole("button", { name: "一键删除" });
    fireEvent.click(button);
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "delete_completed_tasks")).toBe(false);
    expect(confirm).toHaveBeenLastCalledWith(expect.stringContaining("1 个已完成任务"));
    confirm.mockReturnValue(true);
    fireEvent.click(button); fireEvent.click(button);
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "delete_completed_tasks")).toHaveLength(1);
    expect(button.matches(":disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "一键归档" }).matches(":disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "删除" }).matches(":disabled")).toBe(true);
    expect(screen.getByText(board.storedTask.name).closest(".card-wrapper")!.getAttribute("draggable")).toBe("false");
    expect(screen.getByRole("button", { name: /新建任务/ }).matches(":disabled")).toBe(false);
    board.extraTasks.push({ ...board.storedTask, id: "later", name: "后来完成", status: "done" });
    await act(async () => { flowEvents.forEach(callback => callback({ payload: { project_id: project.id, task_id: "later", operation_id: "later" } })); });
    expect(await screen.findByText("后来完成")).toBeTruthy();
    removeStored(["task-1"]);
    await act(async () => finish({ succeeded: ["task-1"], skipped: [], failed: [] }));
    expect(await screen.findByText("删除完成：成功 1，跳过 0，失败 0")).toBeTruthy();
    expect(screen.getByText("后来完成")).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith("delete_completed_tasks", { projectId: project.id, taskIds: ["task-1"] });
  });

  it("部分成功立即移除，刷新失败和迟到的旧列表都不恢复卡片，保留异常原因", async () => {
    const skipped = { ...board.storedTask, id: "skip", name: "待清理", status: "done" as const };
    const failed = { ...board.storedTask, id: "fail", name: "写入失败", status: "done" as const };
    board.extraTasks = [skipped, failed];
    const base = vi.mocked(invoke).getMockImplementation()!;
    let committed = false;
    let failRefresh = true;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => {
      if (cmd === "archive_completed_tasks") {
        committed = true;
        return Promise.resolve({ succeeded: ["task-1"], skipped: [{ id: "skip", error: "archive.cleanup_required" }], failed: [{ id: "fail", error: "disk failure" }] });
      }
      if (cmd === "list_tasks" && committed && failRefresh) return Promise.reject("refresh failed");
      // Keep returning the old list to exercise the removal tombstones.
      return base(cmd, args, options);
    });
    await renderBoard("done");
    fireEvent.click(screen.getByRole("button", { name: "一键归档" }));
    expect(await screen.findByText("归档完成：成功 1，跳过 1，失败 1")).toBeTruthy();
    expect(screen.queryByText(board.storedTask.name)).toBeNull();
    expect(screen.getByText("2 / 2 已完成")).toBeTruthy();
    fireEvent.click(screen.getByText("查看跳过及失败原因"));
    expect(screen.getByText("待清理：请先完成窗口清理再归档")).toBeTruthy();
    expect(screen.getByText("写入失败：disk failure")).toBeTruthy();
    failRefresh = false;
    await act(async () => { window.dispatchEvent(new Event("focus")); });
    expect(screen.queryByText(board.storedTask.name)).toBeNull();
    expect(screen.getByRole("button", { name: "一键归档" }).matches(":disabled")).toBe(false);
  });

  it("IPC 失败保留任务并允许重试", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    let fail = true;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => {
      if (cmd !== "archive_completed_tasks") return base(cmd, args, options);
      if (fail) return Promise.reject("保存失败");
      removeStored(["task-1"]);
      return Promise.resolve({ succeeded: ["task-1"], skipped: [], failed: [] });
    });
    await renderBoard("done");
    fireEvent.click(screen.getByRole("button", { name: "一键归档" }));
    expect((await screen.findByRole("alert")).textContent).toContain("保存失败");
    expect(screen.getByText(board.storedTask.name)).toBeTruthy();
    fail = false;
    fireEvent.click(screen.getByRole("button", { name: "一键归档" }));
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
    expect(screen.getByText("归档完成：成功 1，跳过 0，失败 0")).toBeTruthy();
  });

  it("切换项目后迟到结果不修改新项目的卡片或反馈", async () => {
    board.storedTask.status = "done";
    const base = vi.mocked(invoke).getMockImplementation()!;
    let finish!: (result: BatchTaskResult) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "archive_completed_tasks"
      ? new Promise(resolve => { finish = resolve; }) : base(cmd, args, options));
    const old = render(<KanbanBoard project={project} />);
    await screen.findByText(board.storedTask.name);
    fireEvent.click(screen.getByRole("button", { name: "一键归档" }));
    old.unmount();
    render(<KanbanBoard project={{ ...project, id: "next-project", name: "另一个项目" }} />);
    await screen.findByText(board.storedTask.name);
    await act(async () => finish({ succeeded: ["task-1"], skipped: [], failed: [] }));
    expect(screen.getByText(board.storedTask.name)).toBeTruthy();
    expect(screen.queryByText(/归档完成：/)).toBeNull();
    expect(screen.getByRole("button", { name: "一键归档" }).matches(":disabled")).toBe(false);
  });

  it("英文界面使用本地化按钮、确认与结果", async () => {
    document.documentElement.lang = "en-US";
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => {
      if (cmd !== "delete_completed_tasks") return base(cmd, args, options);
      removeStored(["task-1"]);
      return Promise.resolve({ succeeded: ["task-1"], skipped: [], failed: [] });
    });
    await renderBoard("done");
    fireEvent.click(screen.getByRole("button", { name: "Delete all completed tasks" }));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Delete 1 completed tasks in this project?"));
    expect(await screen.findByText("Delete complete: succeeded 1, skipped 0, failed 0")).toBeTruthy();
  });
});
