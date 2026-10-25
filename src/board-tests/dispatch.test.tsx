// @vitest-environment jsdom
import { board, project, agent, sendPrompt, renderBoard, column, dropOn, expectNoStatusWrite, bindTask } from "./fixtures";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

describe("拖入开发中列", () => {
  it("连接等待期间改变布局，连接成功后同步最新区域", async () => {
    board.storedTask.binding = { state: "valid", operation: null, target: {
      session: "default", workspace_id: "w1", pane_id: "w1:p1", terminal_id: "term1", agent: "codex", process_id: "100", title: "A", status: "idle",
    } };
    const base = vi.mocked(invoke).getMockImplementation()!;
    let connected!: (value: string) => void;
    let connectionId = "";
    vi.mocked(invoke).mockImplementation((command, args, options) => {
      if (command === "terminal_open") {
        connectionId = (args as Record<string, string>).connectionId;
        return new Promise((resolve) => { connected = resolve; });
      }
      return base(command, args, options);
    });
    await renderBoard("in_progress");
    fireEvent.click(screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` }));
    await screen.findByText("连接中…");
    expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ bounds: expect.any(Object) }));
    fireEvent.keyDown(screen.getByRole("separator"), { key: "ArrowLeft" });
    connected(connectionId);
    await screen.findByText("Herdr 已打开");
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_bounds", { connectionId, bounds: expect.any(Object) }));
  });
  it("快速切换后迟到的旧连接只能关闭自身", async () => {
    const target = { session: "default", workspace_id: "w1", pane_id: "w1:p1", terminal_id: "term1", agent: "codex", process_id: "100", title: "A", status: "idle" };
    board.storedTask.binding = { state: "valid", operation: null, target };
    board.extraTasks = [{ ...board.storedTask, id: "task-2", name: "任务 B", status: "in_progress", binding: { state: "valid", operation: null, target: { ...target, pane_id: "w1:p2", terminal_id: "term2" } } }];
    const base = vi.mocked(invoke).getMockImplementation()!;
    let finishA!: (value: string) => void;
    let idA = ""; let idB = "";
    vi.mocked(invoke).mockImplementation((command, args, options) => {
      const input = args as Record<string, string>;
      if (command === "terminal_open") {
        if (input.taskId === "task-1") { idA = input.connectionId; return new Promise((resolve) => { finishA = resolve; }); }
        idB = input.connectionId;
      }
      return base(command, args, options);
    });
    await renderBoard("in_progress");
    fireEvent.click(screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` }));
    await screen.findByText("连接中…");
    fireEvent.click(screen.getByRole("button", { name: "查看终端：任务 B" }));
    await screen.findByText("Herdr 已打开");
    finishA(idA);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_close", { connectionId: idA }));
    expect(idB).not.toBe(idA);
    expect(vi.mocked(invoke).mock.calls.some(([name, args]) => name === "terminal_close" && (args as Record<string, string>).connectionId === idB)).toBe(false);
    expect(screen.getByText("Herdr 已打开")).toBeTruthy();
  });
  it("可通过分隔线键盘调整终端宽度", async () => {
    await renderBoard();
    const separator = screen.getByRole("separator", { name: "调整终端宽度" });
    const initial = Number(separator.getAttribute("aria-valuenow"));
    fireEvent.keyDown(separator, { key: "ArrowLeft" });
    expect(Number(separator.getAttribute("aria-valuenow"))).toBe(initial + 24);
  });
  it("旧绑定可以明确修复，修复本身不派发提示词", async () => {
    board.storedTask.binding = { state: "repair", target: null, operation: null };
    await renderBoard("in_progress");
    fireEvent.click(within(screen.getByRole("complementary", { name: "Herdr 终端" })).getByRole("button", { name: "修复绑定" }));
    fireEvent.change(await screen.findByLabelText("独立 Agent"), { target: { value: agent.pane_id } });
    fireEvent.click(screen.getByRole("button", { name: "保存绑定" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("repair_binding", { taskId: "task-1", paneId: agent.pane_id }));
    expect(sendPrompt).not.toHaveBeenCalled();
  });
  it("选择开发中卡片连接其终端，编辑按钮不会触发重复连接", async () => {
    board.storedTask.binding = { state: "valid", operation: null, target: {
      session: "default", workspace_id: "w1", pane_id: "w1:p1", terminal_id: "term1",
      agent: "codex", process_id: "100", title: "A", status: "idle",
    } };
    await renderBoard("in_progress");
    fireEvent.click(screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` }));
    await screen.findByText("Herdr 已打开");
    expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: "task-1", connectionId: expect.any(String) }));
    const opened = vi.mocked(invoke).mock.calls.filter(([name]) => name === "terminal_open").length;
    fireEvent.click(screen.getByRole("button", { name: "编辑" }));
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "terminal_open")).toHaveLength(opened);
    expect(sendPrompt).not.toHaveBeenCalled();
  });
  it("核实未知派发结果只记录确认，不重发提示词", async () => {
    board.storedTask.binding = { state: "unknown", target: null, operation: {
      id: "op-1", prompt: "一次任务", created_at: "2026-10-24", target: {
        session: "default", workspace_id: "w1", pane_id: "w1:p1", terminal_id: "term1",
        agent: "codex", process_id: "100", title: "A", status: "idle",
      },
    } };
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "核实派发结果" }));
    fireEvent.change(screen.getByLabelText("核实依据"), { target: { value: "已在目标终端确认收到" } });
    fireEvent.click(screen.getByRole("button", { name: "确认已发送" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("resolve_dispatch", {
      taskId: "task-1", operationId: "op-1", sent: true, evidence: "已在目标终端确认收到",
    }));
    expect(sendPrompt).not.toHaveBeenCalled();
  });
  it("其他任务占用的 Agent 不可选择，不能派发", async () => {
    board.availableAgents = [{ ...agent, occupied_by: "another-task" }];
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    const option = await screen.findByRole("option", { name: /已被其他任务占用/ });
    expect((option as HTMLOptionElement).disabled).toBe(true);
    expect(sendPrompt).not.toHaveBeenCalled();
  });
  it("打开派发弹窗，确认前保留原列和持久化状态", async () => {
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    await screen.findByRole("heading", { name: `派发任务: ${board.storedTask.name}` });
    expect(within(column("todo")).getByText(board.storedTask.name)).toBeTruthy();
    expect(board.storedTask.status).toBe("todo");
    expectNoStatusWrite();
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("取消派发不移动任务、不发送提示词", async () => {
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    fireEvent.click(await screen.findByRole("button", { name: "取消" }));
    expect(screen.queryByRole("heading", { name: /派发任务:/ })).toBeNull();
    expect(within(column("todo")).getByText(board.storedTask.name)).toBeTruthy();
    expect(board.storedTask.status).toBe("todo");
    expectNoStatusWrite();
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it("确认后发送到选定 Agent，仅收到成功响应后移动卡片", async () => {
    let finishDispatch!: () => void;
    sendPrompt.mockReturnValue(new Promise<void>((resolve) => { finishDispatch = resolve; }));
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    fireEvent.change(await screen.findByRole("combobox", { name: "独立 Agent" }), { target: { value: agent.pane_id } });
    const dialog = screen.getByRole("heading", { name: /派发任务:/ }).closest(".dialog") as HTMLElement;
    fireEvent.click(within(dialog).getByRole("button", { name: "派发" }));
    await waitFor(() => expect(sendPrompt).toHaveBeenCalledExactlyOnceWith({
      taskId: board.storedTask.id, agentName: agent.pane_id,
      workspace: project.herdr_workspace, promptText: "请修复拖放派发",
      operationId: expect.any(String),
    }));
    expect(board.storedTask.status).toBe("todo");
    expect(within(column("todo")).getByText(board.storedTask.name)).toBeTruthy();
    finishDispatch();
    await waitFor(() => expect(within(column("in_progress")).getByText(board.storedTask.name)).toBeTruthy());
    expect(screen.queryByRole("heading", { name: /派发任务:/ })).toBeNull();
    expect(screen.getByTitle("派发至 fixture/pane-1")).toBeTruthy();
    expect(screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` }).getAttribute("aria-pressed")).toBe("true");
    expectNoStatusWrite();
  });

  it("派发失败时显示错误，保留弹窗及原任务状态", async () => {
    sendPrompt.mockRejectedValue(new Error("Agent unavailable"));
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    fireEvent.change(await screen.findByRole("combobox", { name: "独立 Agent" }), { target: { value: agent.pane_id } });
    const dialog = screen.getByRole("heading", { name: /派发任务:/ }).closest(".dialog") as HTMLElement;
    fireEvent.click(within(dialog).getByRole("button", { name: "派发" }));
    await screen.findByText("Error: Agent unavailable");
    expect(screen.getByRole("heading", { name: /派发任务:/ })).toBeTruthy();
    expect(within(column("todo")).getByText(board.storedTask.name)).toBeTruthy();
    expect(board.storedTask.status).toBe("todo");
    expectNoStatusWrite();
  });

  it("没有 Agent 时仍显示弹窗提示，不提前移动任务", async () => {
    board.availableAgents = [];
    await renderBoard();
    fireEvent.click(screen.getByRole("button", { name: "派发" }));
    await screen.findByText(/未找到 Agent/);
    const dialog = screen.getByRole("heading", { name: /派发任务:/ }).closest(".dialog") as HTMLElement;
    expect((within(dialog).getByRole("button", { name: "派发" }) as HTMLButtonElement).disabled).toBe(true);
    expect(board.storedTask.status).toBe("todo");
    expectNoStatusWrite();
    expect(sendPrompt).not.toHaveBeenCalled();
  });

  it.each([
    ["WebView2 的 dataTransfer 为空时使用拖拽引用", true, true],
    ["没有拖拽引用时使用 dataTransfer", false, false],
  ])("%s", async (_label, emptyTransfer, withDragStart) => {
    await renderBoard();
    dropOn(column("in_progress"), emptyTransfer, withDragStart);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_task", expect.objectContaining({ taskId: board.storedTask.id })));
    expectNoStatusWrite();
  });

  it("已完成任务恢复不发送原提示词，也不读取已失效引用", async () => {
    board.storedTask.flow = { session_id: "saved-id", operation_id: null, kind: null, state: null, error: "", pane_id: null };
    board.storedTask.prompt_json = "invalid old JSON";
    await renderBoard("done");
    dropOn(column("in_progress"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_task", { taskId: board.storedTask.id, operationId: expect.any(String) }));
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "read_file_content")).toBe(false);
    await screen.findByText("Herdr 已打开");
    expect(screen.queryByRole("heading", { name: /派发任务:/ })).toBeNull();
    expectNoStatusWrite();
  });
  it("首次拖入立即自动分配，不重复提交快速拖拽", async () => {
    let finish!: () => void;
    sendPrompt.mockReturnValue(new Promise<void>((resolve) => { finish = resolve; }));
    board.availableAgents = [];
    await renderBoard();
    dropOn(column("in_progress"));
    dropOn(column("in_progress"));
    await waitFor(() => expect(sendPrompt).toHaveBeenCalledTimes(1));
    expect(invoke).toHaveBeenCalledWith("start_task", { taskId: board.storedTask.id, operationId: expect.any(String) });
    expect(board.storedTask.status).toBe("todo");
    expect(screen.queryByRole("heading", { name: /派发任务:/ })).toBeNull();
    finish();
    await screen.findByText("Herdr 已打开");
    expect(within(column("in_progress")).getByText(board.storedTask.name)).toBeTruthy();
  });
  it("自动启动失败保留原列并显示错误", async () => {
    sendPrompt.mockRejectedValue(new Error("herdr.unavailable"));
    await renderBoard(); dropOn(column("in_progress"));
    await screen.findByRole("alert");
    expect(board.storedTask.status).toBe("todo");
    expect(screen.getByRole("alert").textContent).toContain("无法启动 Herdr");
  });
  it("完成清理失败仍刷新到完成列并显示重试", async () => {
    bindTask();
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, args, options) => {
      if (command === "complete_task") {
        board.storedTask.status = "done";
        board.storedTask.cleanup_required = true;
        board.storedTask.flow = { session_id: null, operation_id: "close", kind: "complete", state: "failed", error: "flow.session_missing", pane_id: "w1:p1" };
        return { warning: "flow.session_missing" };
      }
      return base(command, args, options);
    });
    await renderBoard("in_progress");
    fireEvent.click(screen.getByRole("button", { name: "标记完成" }));
    await screen.findByRole("button", { name: "重试关闭窗口" });
    expect(within(column("done")).getByText(board.storedTask.name)).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("任务已完成");
  });
});
