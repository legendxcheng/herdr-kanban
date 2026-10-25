// @vitest-environment jsdom
import { board, project, agent, sendPrompt, flowEvents, renderBoard, column, dropOn, expectNoStatusWrite, bindTask } from "./fixtures";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

describe("保留其他拖放与派发行为", () => {
  it("启动仍未返回时卡片立即进入开发中且不阻塞新建任务", async () => {
    sendPrompt.mockReturnValue(new Promise(() => {}));
    await renderBoard();
    dropOn(column("in_progress"));
    expect(within(column("in_progress")).getByText(board.storedTask.name)).toBeTruthy();
    expect(screen.getAllByText("窗口准备中…").length).toBeGreaterThan(0);
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "terminal_open")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
    expect(await screen.findByRole("dialog")).toBeTruthy();
  });

  it("受理后重新进入看板保留准备状态，就绪通知不抢回手动选择", async () => {
    const original = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, args, options) => {
      if (command === "start_task") {
        const input = args as { operationId: string };
        board.storedTask.flow = { session_id: null, operation_id: input.operationId, kind: "start", state: "pending",
          error: "", pane_id: null, requested_status: "in_progress", stage: "starting_agent" };
        return board.storedTask.flow;
      }
      return original(command, args, options);
    });
    await renderBoard();
    dropOn(column("in_progress"));
    await screen.findAllByText("正在启动或恢复 Agent…");
    cleanup();
    bindTask();
    board.extraTasks = [{ ...board.storedTask, id: "other", name: "其他任务", status: "in_progress", flow: null }];
    board.storedTask.binding = null;
    await renderBoard();
    expect(within(column("in_progress")).getByText(board.storedTask.name)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "修复绑定" })).toBeNull();
    expect(screen.queryByRole("button", { name: "派发" })).toBeNull();
    fireEvent.click(screen.getByText("其他任务"));
    await screen.findByText("已连接");
    bindTask(); board.storedTask.status = "in_progress"; board.storedTask.flow!.state = "succeeded";
    await act(async () => { for (const callback of flowEvents) callback({ payload: {
      project_id: project.id, task_id: board.storedTask.id, operation_id: board.storedTask.flow!.operation_id!
    } }); });
    await waitFor(() => expect(screen.getByRole("complementary").querySelector("strong")!.textContent).toBe("其他任务"));
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "start_task")).toHaveLength(1);
  });

  it("恢复焦点补查遗漏的启动完成事件并自动连接当前任务", async () => {
    board.storedTask.flow = { session_id: null, operation_id: "pending-start", kind: "start", state: "pending",
      error: "", pane_id: null, stage: "queued", requested_status: "in_progress" };
    await renderBoard();
    expect(screen.getAllByText("窗口准备中…").length).toBeGreaterThan(0);
    bindTask(); board.storedTask.status = "in_progress"; board.storedTask.flow.state = "succeeded";
    fireEvent(window, new Event("focus"));
    await screen.findByText("已连接");
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "start_task")).toHaveLength(0);
  });

  it("预览和派发都保留技能调用，不读取技能正文", async () => {
    board.storedTask.prompt_json = JSON.stringify({ type: "doc", content: [
      { type: "paragraph", content: [
        { type: "text", text: "开始实现 " },
        { type: "mention", attrs: { kind: "skill", label: "to-spec", ref: "C:/skills/to-spec/SKILL.md" } },
      ] },
    ] });
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    const select = await screen.findByRole("combobox", { name: "独立 Agent" });
    fireEvent.change(select, { target: { value: agent.pane_id } });
    fireEvent.click(screen.getByRole("button", { name: "预览提示词" }));
    expect(screen.getByText("开始实现 $to-spec")).toBeTruthy();
    const dialog = screen.getByRole("heading", { name: /派发任务:/ }).closest(".dialog") as HTMLElement;
    fireEvent.click(within(dialog).getByRole("button", { name: "派发" }));
    await waitFor(() => expect(sendPrompt).toHaveBeenCalledWith(expect.objectContaining({ promptText: "开始实现 $to-spec" })));
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "read_skill_content")).toBe(false);
  });

  it.each(["todo", "in_progress", "done"] as const)("拖回原列 %s 不触发派发或状态更新", async (status) => {
    await renderBoard(status);
    dropOn(column(status));
    expect(screen.queryByRole("heading", { name: /派发任务:/ })).toBeNull();
    expectNoStatusWrite();
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it.each(["todo", "done"] as const)("拖到 %s 列仍直接更新状态", async (status) => {
    await renderBoard("in_progress");
    dropOn(column(status));
    await waitFor(() => expect(within(column(status)).getByText(board.storedTask.name)).toBeTruthy());
    if (status === "done") expect(invoke).toHaveBeenCalledWith("complete_task", expect.objectContaining({ taskId: board.storedTask.id }));
    else expect(invoke).toHaveBeenCalledWith("update_task_status", { id: board.storedTask.id, status });
    expect(screen.queryByRole("heading", { name: /派发任务:/ })).toBeNull();
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("有 Agent 时不展示顶部派发栏，仍可从卡片选择 Agent", async () => {
    await renderBoard();
    expect(screen.queryByText("拖拽任务到 Agent 即可选择派发")).toBeNull();
    expect(screen.queryByText(agent.title)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    const select = await screen.findByRole("combobox", { name: "独立 Agent" }) as HTMLSelectElement;
    expect(within(select).getByRole("option", { name: new RegExp(agent.title) })).toBeTruthy();
    expectNoStatusWrite();
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("未开发卡片保留首次派发按钮", async () => {
    await renderBoard("todo");
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    await screen.findByRole("heading", { name: /派发任务:/ });
    expectNoStatusWrite();
    expect(sendPrompt).not.toHaveBeenCalled();
  });
});
