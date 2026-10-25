// @vitest-environment jsdom
import { board, project, sendPrompt, renderBoard, column, expectNoStatusWrite, bindTask, secondTask, liveAgent, cardDot, cardOrder } from "./fixtures";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { KanbanBoard } from "../KanbanBoard";
import type { HerdrAgent } from "../types";

describe("实时 Agent 状态", () => {
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
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
    ["idle", "空闲，等待新指令"], ["unknown", "状态未知／连接不可用"], ["new-state", "状态未知／连接不可用"],
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
      if (cmd === "herdr_list_agents" && offline) throw new Error("offline");
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
    let resolve!: (agents: HerdrAgent[]) => void;
    let reads = 0;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_list_agents"
      ? (reads++, new Promise(r => { resolve = r as typeof resolve; })) : base(cmd, args, opts));
    // Recreate interval under fake timers using a fresh mounted board.
    cleanup(); render(<KanbanBoard project={project} />);
    await act(async () => {});
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(reads).toBe(1);
    await act(async () => { resolve(board.availableAgents); });
    // load invalidated the first read; settle its single follow-up.
    await act(async () => { resolve(board.availableAgents); });
    const baseline = reads;
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(reads).toBe(baseline + 1);
    await act(async () => { resolve(board.availableAgents); });
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    fireEvent(document, new Event("visibilitychange"));
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(reads).toBe(baseline + 1);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    fireEvent(document, new Event("visibilitychange"));
    expect(reads).toBe(baseline + 2);
    await act(async () => { resolve(board.availableAgents); });
  });

  it("切项目后旧请求不能覆盖新项目状态", async () => {
    bindTask();
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (agents: HerdrAgent[]) => void;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_list_agents" && (args as {projectId:string}).projectId === project.id
      ? new Promise(r => { resolve = r as typeof resolve; }) : base(cmd, args, opts));
    board.storedTask.status = "in_progress";
    const view = render(<KanbanBoard key={project.id} project={project} />);
    await screen.findByText(board.storedTask.name, { selector: ".task-name" });
    board.availableAgents = [liveAgent(board.storedTask, "working")];
    view.rerender(<KanbanBoard key="other" project={{ ...project, id: "other" }} />);
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working"));
    await act(async () => { resolve([liveAgent(board.storedTask, "blocked")]); });
    expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("working");
  });

  it("编辑刷新使进行中的旧查询失效，并补查新状态", async () => {
    bindTask(); board.availableAgents = [liveAgent(board.storedTask, "idle")];
    await renderBoard("in_progress");
    await waitFor(() => expect(cardDot(board.storedTask.name).getAttribute("data-status")).toBe("idle"));
    const base = vi.mocked(invoke).getMockImplementation()!;
    let resolve!: (agents: HerdrAgent[]) => void;
    let delayed = false;
    vi.mocked(invoke).mockImplementation((cmd, args, opts) => {
      if (cmd === "herdr_list_agents" && !delayed) {
        delayed = true; return new Promise(r => { resolve = r as typeof resolve; });
      }
      return base(cmd, args, opts);
    });
    fireEvent(window, new Event("focus"));
    fireEvent.click(within(column("in_progress")).getByRole("button", { name: "编辑" }));
    fireEvent.click(await screen.findByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "保存" })).toBeNull());
    board.availableAgents = [liveAgent(board.storedTask, "working")];
    await act(async () => { resolve([liveAgent(board.storedTask, "blocked")]); });
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
