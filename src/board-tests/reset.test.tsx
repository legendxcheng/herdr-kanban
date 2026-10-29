// @vitest-environment jsdom
import { board, project, renderBoard, column, sendPrompt } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TaskBoard } from "../KanbanBoard";

function failed(state = "unknown") {
  board.storedTask.execution_epoch = 0;
  board.storedTask.flow = { session_id: "old-session", operation_id: "old-flow", kind: "start", state,
    error: "timeout", pane_id: "old-pane", requested_status: "in_progress" };
}

async function openResetTask() {
  if (!screen.queryByRole("button", { name: "退回 Todo" })) {
    fireEvent.click(await screen.findByRole("button", { name: "需核实" }));
  }
  fireEvent.click(screen.getByRole("button", { name: "退回 Todo" }));
}

function mockReset() {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (cmd, args, options) => {
    if (cmd === "reset_task") {
      board.storedTask = { ...board.storedTask, status: "todo", execution_epoch: 1, flow: null, binding: null,
        cleanup_required: false, codex_settings_editable: true, dispatched_at: null, dispatched_to: null, prompt_snapshot: null };
      return { ...board.storedTask };
    }
    if (cmd === "board_tasks") return [{ ...board.storedTask }];
    return base(cmd, args, options);
  });
}

describe("放弃执行并退回 Todo", () => {
  it.each(["pending", "unknown", "failed"])("%s 时可以只读查看提示词，不启动、不改写任务", async state => {
    failed(state);
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "查看提示词" }));
    const dialog = await screen.findByRole("dialog", { name: "查看提示词" });
    expect(within(dialog).getByRole("textbox", { name: "提示词" }).getAttribute("contenteditable")).toBe("false");
    expect(within(dialog).getByText("请修复拖放派发")).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "保存" })).toBeNull();
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => ["update_task", "start_task", "dispatch_to_herdr", "reset_task"].includes(cmd))).toBe(false);
  });

  it("待核实时显示确认，取消不改动；确认后保留内容并打开 Todo 编辑面板", async () => {
    failed(); mockReset();
    const prompt = board.storedTask.prompt_json;
    await renderBoard();
    await openResetTask();
    let dialog = await screen.findByRole("dialog", { name: "放弃本次执行，退回 Todo" });
    expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "取消" }));
    expect(within(dialog).getByText(/旧 Agent 可能仍在运行/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "reset_task")).toBe(false);
    await openResetTask();
    dialog = await screen.findByRole("dialog", { name: "放弃本次执行，退回 Todo" });
    fireEvent.click(within(dialog).getByRole("button", { name: "放弃本次执行并退回" }));
    await waitFor(() => expect(document.querySelector(".task-edit-panel")).toBeTruthy());
    expect(within(column("todo")).getByText(board.storedTask.name)).toBeTruthy();
    expect(board.storedTask.prompt_json).toBe(prompt);
    expect(invoke).toHaveBeenCalledWith("reset_task", { id: board.storedTask.id, expectedEpoch: 0,
      flowId: "old-flow", dispatchId: null, requestId: expect.any(String) });
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("重复点击只提交一次，失败保持卡片和弹窗，允许重试", async () => {
    failed();
    const base = vi.mocked(invoke).getMockImplementation()!;
    let reject!: (reason: string) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "reset_task"
      ? new Promise((_resolve, fail) => { reject = fail; }) : base(cmd,args,options));
    await renderBoard();
    await openResetTask();
    const button = await screen.findByRole("button", { name: "放弃本次执行并退回" });
    fireEvent.click(button); fireEvent.click(button);
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "reset_task")).toHaveLength(1);
    await act(async () => reject("reset.stale"));
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByText("任务状态已变化，请关闭弹窗并重新操作。")).toBeTruthy();
    expect(within(column("in_progress")).getByText(board.storedTask.name)).toBeTruthy();
    expect(button.matches(":disabled")).toBe(false);
  });

  it("启动或派发在途只提供查看入口", async () => {
    failed("pending");
    await renderBoard();
    expect(screen.queryByRole("button", { name: "退回 Todo" })).toBeNull();
    expect(screen.getByRole("button", { name: "查看提示词" }).matches(":disabled")).toBe(false);
  });

  it("All Tasks 在原页面退回并编辑任务", async () => {
    failed(); mockReset();
    render(<TaskBoard projects={[project]} />);
    await openResetTask();
    fireEvent.click(await screen.findByRole("button", { name: "放弃本次执行并退回" }));
    await waitFor(() => expect(document.querySelector(".task-edit-panel")).toBeTruthy());
    expect(document.querySelector(".summary-board")).toBeTruthy();
    expect(within(column("todo")).getByText(board.storedTask.name)).toBeTruthy();
  });

  it("重置成功后列表刷新失败仍保留 Todo 和编辑面板", async () => {
    failed(); mockReset();
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((cmd,args,options) => cmd === "list_tasks" && board.storedTask.execution_epoch === 1
      ? Promise.reject("read failed") : base(cmd,args,options));
    await renderBoard();
    await openResetTask();
    fireEvent.click(await screen.findByRole("button", { name: "放弃本次执行并退回" }));
    await waitFor(() => expect(document.querySelector(".task-edit-panel")).toBeTruthy());
    expect(within(column("todo")).getByText(board.storedTask.name)).toBeTruthy();
  });

  it("进入重置确认前保护另一张 Todo 的未保存修改", async () => {
    failed(); mockReset();
    board.extraTasks = [{ ...board.storedTask, id: "draft", name: "另一个待办", status: "todo", flow: null }];
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "查看并编辑：另一个待办" }));
    const panel = screen.getByRole("complementary", { name: "编辑任务" });
    fireEvent.change(within(panel).getByRole("textbox", { name: "标题" }), { target: { value: "尚未保存" } });
    await openResetTask();
    fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
    expect(screen.queryByRole("dialog", { name: "放弃本次执行，退回 Todo" })).toBeNull();
    expect((within(panel).getByRole("textbox", { name: "标题" }) as HTMLInputElement).value).toBe("尚未保存");
    await openResetTask();
    fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
    expect(await screen.findByRole("dialog", { name: "放弃本次执行，退回 Todo" })).toBeTruthy();
    expect(board.extraTasks[0].name).toBe("尚未保存");
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "reset_task")).toBe(false);
  });
});
