// @vitest-environment jsdom
import { board, project, sendPrompt, renderBoard, column, expectNoStatusWrite, bindTask, secondTask, liveAgent, cardDot, cardOrder } from "./fixtures";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { KanbanBoard } from "../KanbanBoard";
import type { AgentSnapshot } from "../types";

describe("实时 Agent 状态", () => {
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
  it("未点开的任务在后台变为等待输入，焦点刷新期间保留已知状态", async () => {
    bindTask(); board.extraTasks = [secondTask()];
    board.availableAgents = [liveAgent(board.storedTask, "working"), liveAgent(board.extraTasks[0], "working")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot("任务 B").getAttribute("data-status")).toBe("working"));
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (agents: AgentSnapshot) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_agent_snapshot"
      ? new Promise(r => { resolve = r as typeof resolve; }) : base(cmd, args, opts));
    fireEvent(window, new Event("focus"));
    expect(cardDot("任务 B").getAttribute("data-status")).toBe("working");
    await act(async () => { resolve({ agents: [liveAgent(board.storedTask, "working"), liveAgent(board.extraTasks[0], "idle")], failures: [] }); });
    await waitFor(() => expect(cardDot("任务 B").getAttribute("data-status")).toBe("idle"));
    expect(screen.getByRole("button", { name: "查看终端：任务 B" }).getAttribute("aria-pressed")).toBe("false");
    expectNoStatusWrite(); expect(sendPrompt).not.toHaveBeenCalled();
  });
  it("查询失败标明过期和上次状态，恢复后自动消除提示", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "idle")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("idle"));
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => {
      if (cmd === "herdr_agent_snapshot") throw new Error("offline");
      return base(cmd, args, opts);
    });
    fireEvent(window, new Event("focus"));
    await screen.findByText("查询失败");
    expect(cardDot(board.storedTask.name).getAttribute("title")).toContain("上次：空闲，等待新指令（已过期）");
    expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("unknown");
    vi.mocked(invoke).mockImplementation(base);
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("idle"));
    expect(screen.queryByText("查询失败")).toBeNull();
  });
  it("反复切回窗口不会丢弃同一轮后台查询的有效结果", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "working")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working"));
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (snapshot: AgentSnapshot) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_agent_snapshot"
      ? new Promise(r => { resolve = r as typeof resolve; }) : base(cmd, args, opts));
    fireEvent(window, new Event("focus"));
    fireEvent(window, new Event("focus"));
    await act(async () => { resolve({ agents: [liveAgent(board.storedTask, "idle")], failures: [] }); });
    // Settle any follow-up even on failure so shared discovery does not leak to another test.
    const status = cardDot(board.storedTask.name).getAttribute("data-status");
    await act(async () => { resolve({ agents: [liveAgent(board.storedTask, "idle")], failures: [] }); });
    expect(status).toBe("idle");
  });
  it("慢查询超过有效期后不继续显示旧完成灯", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "done")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("done"));
    vi.useFakeTimers();
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (snapshot: AgentSnapshot) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_agent_snapshot"
      ? new Promise(r => { resolve = r as typeof resolve; }) : base(cmd, args, opts));
    try {
      fireEvent(window, new Event("focus"));
      await act(async () => { vi.setSystemTime(Date.now() + 16_000); fireEvent(window, new Event("focus")); });
      expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("unknown");
      expect(screen.getByText("状态过期")).toBeTruthy();
      expect(cardDot(board.storedTask.name).getAttribute("title")).toContain("上次：本轮已结束，待查看（已过期）");
    } finally {
      await act(async () => { resolve({ agents: [], failures: [] }); });
    }
  });
  it("延迟送达的快照不能用服务端较短采样年龄重置有效期", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "working")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working"));
    vi.useFakeTimers();
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (snapshot: AgentSnapshot) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_agent_snapshot"
      ? new Promise(r => { resolve = r as typeof resolve; }) : base(cmd, args, opts));
    fireEvent(window, new Event("focus"));
    await act(async () => {
      vi.setSystemTime(Date.now() + 20_000);
      resolve({ agents: [liveAgent(board.storedTask, "done")], failures: [], age_ms: 100 });
    });
    expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("unknown");
    expect(screen.getByText("状态过期")).toBeTruthy();
  });
  it.each([
    ["herdr.timeout", "查询失败"], ["binding.identity_changed", "绑定异常"],
  ])("单个会话的 %s 不被误报为会话不存在", async (error, label) => {
    bindTask(); board.extraTasks = [secondTask()];
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => cmd === "herdr_agent_snapshot"
      ? { agents: [liveAgent(board.extraTasks[0], "idle")], failures: [{ pane_id: board.storedTask.binding!.target!.pane_id, error }] }
      : base(cmd, args, opts));
    await renderBoard("in_progress");
    await screen.findByText(label);
    expect(cardDot("任务 B").getAttribute("data-status")).toBe("idle");
    expect(screen.queryByText("会话未找到")).toBeNull();
  });
  it("单 pane 查询失败保留该任务的过期历史而不影响其他任务", async () => {
    bindTask(); board.extraTasks = [secondTask()];
    board.availableAgents = [liveAgent(board.storedTask, "done"), liveAgent(board.extraTasks[0], "working")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("done"));
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => cmd === "herdr_agent_snapshot"
      ? { agents: [liveAgent(board.extraTasks[0], "idle")], failures: [{ pane_id: board.storedTask.binding!.target!.pane_id, error: "herdr.timeout" }] }
      : base(cmd, args, opts));
    fireEvent(window, new Event("focus"));
    await screen.findByText("查询失败");
    expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("unknown");
    expect(cardDot(board.storedTask.name).getAttribute("title")).toContain("上次：本轮已结束，待查看（已过期）");
    expect(cardDot("任务 B").getAttribute("data-status")).toBe("idle");
    vi.mocked(invoke).mockImplementation(base);
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("done"));
    expect(screen.queryByText("查询失败")).toBeNull();
  });
  it("等待置顶并在恢复后回到原顺序，刷新不抢选中任务或重连终端", async () => {
    bindTask(); board.storedTask.priority = "urgent";
    board.extraTasks = [secondTask(), { ...secondTask(), id: "task-3", name: "任务 C", priority: "normal",
      binding: { state: "valid", operation: null, target: { ...board.storedTask.binding!.target!, pane_id: "w1:p3" } } }];
    board.availableAgents = [liveAgent(board.storedTask, "working"), liveAgent(board.extraTasks[0], "blocked"), liveAgent(board.extraTasks[1], "blocked")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardOrder()).toEqual(["任务 B", "任务 C", board.storedTask.name]));
    const selected = screen.getByRole("button", { name: `查看终端：${board.storedTask.name}` });
    expect(selected.getAttribute("aria-pressed")).toBe("true");
    await screen.findByText("Herdr 已打开");
    const opened = vi.mocked(invoke).mock.calls.filter(([name]) => name === "terminal_open").length;
    board.availableAgents = [liveAgent(board.storedTask, "working"), ...board.extraTasks.map(t => liveAgent(t, "working"))];
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(cardOrder()).toEqual([board.storedTask.name, "任务 B", "任务 C"]));
    expect(selected.getAttribute("aria-pressed")).toBe("true");
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "terminal_open")).toHaveLength(opened);
    expectNoStatusWrite(); expect(sendPrompt).not.toHaveBeenCalled();
  });

  it.each([
    ["working", "运行中"], ["blocked", "等待用户反馈／审批"], ["done", "本轮已结束，待查看"],
    ["idle", "空闲，等待新指令"], ["unknown", "状态未知／连接不可用 · 未识别状态"], ["new-state", "状态未知／连接不可用 · 未识别状态"],
  ])("圆点 %s 显示文字且不自动完成任务", async (status, label) => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, status)];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("title")).toBe(label));
    expect(cardDot(board.storedTask.name).getAttribute("aria-label")).toBe(label);
    expect(board.storedTask.status).toBe("in_progress"); expectNoStatusWrite();
  });

  it("未启动、已关闭和清理未决有不同提示", async () => {
    board.extraTasks = [{ ...board.storedTask, id: "closed", name: "已关闭", status: "done" },
      { ...board.storedTask, id: "pending", name: "待清理", status: "done", cleanup_required: true }];
    await renderBoard();
    expect(cardDot(board.storedTask.name).getAttribute("title")).toBe("未启动");
    expect(cardDot("已关闭").getAttribute("title")).toBe("会话已关闭");
    expect(cardDot("待清理").getAttribute("data-status")).toBe("unknown");
  });

  it("身份变化、服务失败及恢复不会沿用旧状态", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "blocked")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("blocked"));
    board.availableAgents = [{ ...liveAgent(board.storedTask, "blocked"), target: { ...board.storedTask.binding!.target!, process_id: "replacement" } }];
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("unknown"));
    const base = vi.mocked(invoke).getMockImplementation()!;
    let offline = true;
    vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => {
      if (cmd === "herdr_agent_snapshot" && offline) throw new Error("offline");
      return base(cmd, args, opts);
    });
    board.availableAgents = [liveAgent(board.storedTask, "working")];
    await act(async () => { fireEvent(window, new Event("focus")); });
    expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("unknown");
    offline = false; fireEvent(window, new Event("focus"));
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working"));
  });

  it("前台三秒刷新、后台暂停、回前台立即刷新且慢请求不重叠", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "idle")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("idle"));
    vi.useFakeTimers();
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (agents: AgentSnapshot) => void;
    let reads = 0;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_agent_snapshot"
      ? (reads++, new Promise(r => { resolve = r as typeof resolve; })) : base(cmd, args, opts));
    // Recreate interval under fake timers using a fresh mounted board.
    cleanup(); render(<KanbanBoard project={project} />);
    await act(async () => {});
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(reads).toBe(1);
    await act(async () => { resolve({ agents: board.availableAgents, failures: [] }); });
    // load invalidated the first read; settle its single follow-up.
    await act(async () => { resolve({ agents: board.availableAgents, failures: [] }); });
    const baseline = reads;
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(reads).toBe(baseline + 1);
    await act(async () => { resolve({ agents: board.availableAgents, failures: [] }); });
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    fireEvent(document, new Event("visibilitychange"));
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(reads).toBe(baseline + 1);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    fireEvent(document, new Event("visibilitychange"));
    expect(reads).toBe(baseline + 2);
    await act(async () => { resolve({ agents: board.availableAgents, failures: [] }); });
  });

  it("切项目后旧请求不能覆盖新项目状态", async () => {
    bindTask();
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (agents: AgentSnapshot) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_agent_snapshot" && (args as {projectId:string}).projectId === project.id
      ? new Promise(r => { resolve = r as typeof resolve; }) : base(cmd, args, opts));
    board.storedTask.status = "in_progress";
    const view = render(<KanbanBoard key={project.id} project={project} />);
    await screen.findByText(board.storedTask.name, { selector: ".task-name" });
    board.availableAgents = [liveAgent(board.storedTask, "working")];
    board.storedTask.project_id = "other";
    view.rerender(<KanbanBoard key="other" project={{ ...project, id: "other" }} />);
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working"));
    await act(async () => { resolve({ agents: [liveAgent(board.storedTask, "blocked")], failures: [] }); });
    expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working");
  });

  it("编辑刷新使进行中的旧查询失效，并补查新状态", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "idle")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("idle"));
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (agents: AgentSnapshot) => void;
    let delayed = false;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => {
      if (cmd === "herdr_agent_snapshot" && !delayed) {
        delayed = true; return new Promise(r => { resolve = r as typeof resolve; });
      }
      return base(cmd, args, opts);
    });
    fireEvent(window, new Event("focus"));
    fireEvent.click(within(column("in_progress")).getByRole("button", { name: "编辑" }));
    fireEvent.click(await screen.findByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "保存" })).toBeNull());
    board.availableAgents = [liveAgent(board.storedTask, "working")];
    await act(async () => { resolve({ agents: [liveAgent(board.storedTask, "blocked")], failures: [] }); });
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working"));
  });

  it("未决绑定即使收到匹配的 blocked 快照也不置顶", async () => {
    bindTask(); board.extraTasks = [secondTask()];
    board.extraTasks[0].flow = { session_id: null, operation_id: "unresolved", kind: "start", state: "unknown", error: "", pane_id: "w1:p2" };
    board.availableAgents = [liveAgent(board.storedTask, "working"), liveAgent(board.extraTasks[0], "blocked")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working"));
    expect(cardDot("任务 B").getAttribute("data-status")).toBe("unknown");
    expect(cardOrder()).toEqual([board.storedTask.name, "任务 B"]);
  });
});
