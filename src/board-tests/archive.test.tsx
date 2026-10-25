// @vitest-environment jsdom
import { board, sendPrompt, renderBoard, bindTask } from "./fixtures";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

describe("紧凑卡片与归档", () => {
  it("从编辑图标按下拖动不会开始任务拖拽", async () => {
    await renderBoard();
    const edit = screen.getByRole("button", { name: "编辑" });
    const transfer = { setData: vi.fn(), effectAllowed: "none" };
    fireEvent.pointerDown(edit);
    const accepted = fireEvent.dragStart(edit.closest(".card-wrapper")!, { dataTransfer: transfer });
    expect(accepted).toBe(false);
    expect(transfer.setData).not.toHaveBeenCalled();
  });

  it("归档成功后即使列表刷新失败也移除卡片并更新统计", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command, args, options) => command === "list_tasks" && board.deleted
      ? Promise.reject(new Error("列表暂时不可用")) : base(command, args, options));
    await renderBoard("done");
    fireEvent.click(screen.getByRole("button", { name: "归档" }));
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
    expect(await screen.findByText("0 / 0 已完成")).toBeTruthy();
  });

  it("卡片不显示时间、修复绑定或再派发，保留图标编辑与完整标题", async () => {
    bindTask();
    board.storedTask.dispatched_to = "workspace/target";
    board.storedTask.dispatched_at = "2026-10-24T12:00:00Z";
    await renderBoard("in_progress");
    const card = screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` }).closest(".task-card") as HTMLElement;
    expect(within(card).queryByText(/2026/)).toBeNull();
    expect(within(card).queryByRole("button", { name: "修复绑定" })).toBeNull();
    expect(within(card).queryByRole("button", { name: "再派发" })).toBeNull();
    expect(within(card).getByRole("button", { name: "编辑" }).textContent).toBe("");
    expect(within(card).getByTitle(board.storedTask.name)).toBeTruthy();
    expect(within(card).getByTitle("派发至 workspace/target")).toBeTruthy();
    expect(within(card).queryByRole("button", { name: "归档" })).toBeNull();
  });

  it("归档提交防重复，失败保留卡片并允许重试，不弹确认", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    let rejectArchive!: (reason: Error) => void;
    const confirm = vi.spyOn(window, "confirm");
    vi.mocked(invoke).mockImplementation((command, args, options) => command === "archive_task"
      ? new Promise((_resolve, reject) => { rejectArchive = reject; }) : base(command, args, options));
    await renderBoard("done");
    const archive = screen.getByRole("button", { name: "归档" });
    fireEvent.click(archive); fireEvent.click(archive);
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "archive_task")).toHaveLength(1);
    expect(archive.matches(":disabled")).toBe(true);
    rejectArchive(new Error("保存失败"));
    expect((await screen.findByRole("alert")).textContent).toContain("保存失败");
    expect(screen.getByText(board.storedTask.name)).toBeTruthy();
    expect(archive.matches(":disabled")).toBe(false);
    expect(confirm).not.toHaveBeenCalled(); confirm.mockRestore();
  });

  it("后续启动失败不会隐藏完成清理问题，归档禁用且可重试清理", async () => {
    board.storedTask.cleanup_required = true;
    board.storedTask.flow = { session_id: null, operation_id: "restart", kind: "start", state: "failed", error: "binding.identity_changed", pane_id: "p1" };
    await renderBoard("done");
    const archive = screen.getByRole("button", { name: "归档" });
    expect(archive.matches(":disabled")).toBe(true);
    expect(screen.getByText("请先完成窗口清理再归档")).toBeTruthy();
    expect(screen.getByRole("button", { name: "重试关闭窗口" })).toBeTruthy();
    fireEvent.click(archive);
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "archive_task")).toBe(false);
  });

  it("已完成任务点击归档后消失并更新数量，不派发或连接终端", async () => {
    await renderBoard("done");
    expect(screen.getByText("1 / 1 已完成")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "归档" }));
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
    expect(screen.getByText("0 / 0 已完成")).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith("archive_task", { id: board.storedTask.id });
    expect(sendPrompt).not.toHaveBeenCalled();
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_open")).toBe(false);
  });
});
