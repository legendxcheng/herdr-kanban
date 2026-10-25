// @vitest-environment jsdom
import { board, agent, sendPrompt, renderBoard } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ToastContainer } from "../Toast";

describe("删除异常任务", () => {
  it("删除成功后即使列表刷新失败也立即移除卡片", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "list_tasks" && board.deleted
      ? Promise.reject("list unavailable") : base(cmd, args, options));
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "删除" }));
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
  });

  it("手动派发等待期间可收起弹窗并删除，迟到结果不恢复卡片", async () => {
    let finish!: () => void;
    sendPrompt.mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    const select = await screen.findByRole("combobox", { name: "独立 Agent" });
    fireEvent.change(select, { target: { value: agent.pane_id } });
    const dialog = screen.getByRole("heading", { name: /派发任务:/ }).closest(".dialog") as HTMLElement;
    fireEvent.click(within(dialog).getByRole("button", { name: "派发" }));
    await waitFor(() => expect(sendPrompt).toHaveBeenCalledTimes(1));
    const cancel = within(dialog).getByRole("button", { name: "取消" });
    expect(cancel.matches(":disabled")).toBe(false);
    fireEvent.click(cancel);
    expect(screen.queryByRole("heading", { name: /派发任务:/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "删除" }));
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
    await act(async () => finish());
    expect(screen.queryByText(board.storedTask.name)).toBeNull();
    expect(sendPrompt).toHaveBeenCalledTimes(1);
  });

  it.each(["pending", "unknown", "failed", "cleanup", "dispatch-pending", "dispatch-unknown"])("%s 状态仍可使用原有删除按钮", async (state) => {
    if (state.startsWith("dispatch-")) {
      board.storedTask.binding = { state: state === "dispatch-pending" ? "pending" : "unknown", target: null, operation: {
        id: "dispatch", target: { session: "default", workspace_id: "w1", pane_id: "gone", terminal_id: "term", agent: "codex", process_id: "1", title: "", status: "unknown" },
        prompt: "once", created_at: "now",
      } };
    } else {
      board.storedTask.flow = { session_id: null, operation_id: "flow", kind: state === "cleanup" ? "complete" : "start",
        state: state === "cleanup" ? "unknown" : state, error: "", pane_id: null };
      board.storedTask.cleanup_required = state === "cleanup";
    }
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderBoard();
    const button = screen.getByRole("button", { name: "删除" });
    expect(button.matches(":disabled")).toBe(false);
    fireEvent.click(button);
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
  });

  it("取消不删除，失败保留卡片可重试，删除在途阻止重复提交", async () => {
    render(<ToastContainer />);
    const confirmation = vi.spyOn(window, "confirm").mockReturnValue(false);
    const base = vi.mocked(invoke).getMockImplementation()!;
    let reject!: (error: Error) => void;
    const pending = new Promise<never>((_, fail) => { reject = fail; });
    let fail = true;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "delete_task" && fail ? pending : base(cmd, args, options));
    await renderBoard();
    const button = screen.getByRole("button", { name: "删除" });
    fireEvent.click(button);
    expect(invoke).not.toHaveBeenCalledWith("delete_task", { id: board.storedTask.id });
    confirmation.mockReturnValue(true);
    fireEvent.click(button); fireEvent.click(button);
    expect(button.matches(":disabled")).toBe(true);
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "delete_task")).toHaveLength(1);
    await act(async () => reject(new Error("disk failure")));
    expect(screen.getByText(/disk failure/)).toBeTruthy();
    expect(screen.getByText(board.storedTask.name)).toBeTruthy();
    expect(button.matches(":disabled")).toBe(false);
    fail = false;
    fireEvent.click(button);
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
  });

  it("启动结果未知时可以直接确认删除，无需核实或连接 Herdr", async () => {
    board.storedTask.flow = { session_id: null, operation_id: "lost-start", kind: "start", state: "unknown", error: "herdr.unavailable", pane_id: "gone" };
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await renderBoard();
    const button = screen.getByRole("button", { name: "删除" });
    expect(button.matches(":disabled")).toBe(false);
    fireEvent.click(button);
    await waitFor(() => expect(screen.queryByText(board.storedTask.name)).toBeNull());
    expect(invoke).toHaveBeenCalledWith("delete_task", { id: board.storedTask.id });
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => ["recover_lifecycle", "start_task", "resolve_dispatch"].includes(cmd))).toBe(false);
  });
});
