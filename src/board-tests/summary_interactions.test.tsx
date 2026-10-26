// @vitest-environment jsdom
import { board, project, bindTask, liveAgent, cardDot } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";
import { TaskSummaryBoard } from "../TaskSummaryBoard";
import type { Project, Task } from "../types";

const remote: Project = { ...project, id: "remote-project", name: "远端项目", machine_id: "remote", root_path: "D:/remote", workspace_id: "w2", herdr_workspace: "remote-workspace" };
let projects: Project[], stored: Task[];
beforeEach(() => {
  projects = [project, remote];
  stored = [board.storedTask, { ...board.storedTask, id: "remote-task", project_id: remote.id, name: "远端待办" }];
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (cmd, args, opts) => {
    const input = args as Record<string, any>;
    if (cmd === "list_projects") return projects;
    if (cmd === "list_machines") return [{ id: "local", name: "local", kind: "local", session: "default" }, { id: "remote", name: "Devpc", kind: "ssh", session: "default" }];
    if (cmd === "navigation_state") return { configured: true, recovery_projects: [] };
    if (cmd === "workspace_operations") return [];
    if (cmd === "sync_projects") return { projects: projects.filter(p => (p.machine_id ?? "local") === (input?.machineId ?? "local")), session: "default", workspaces: [], operations: [] };
    if (cmd === "task_summary" || cmd === "board_tasks") return stored.filter(t => input.projectIds.includes(t.project_id)).map(t => ({ ...t }));
    if (cmd === "list_tasks") return stored.filter(t => t.project_id === input.projectId).map(t => ({ ...t }));
    if (cmd === "update_task") { const task = stored.find(t => t.id === input.id)!; Object.assign(task, { name: input.name ?? task.name, priority: input.priority, prompt_json: input.promptJson }); return; }
    if (cmd === "start_task") {
      const task = stored.find(t => t.id === input.taskId)!;
      task.status = "in_progress";
      task.flow = { kind: "start", state: "succeeded", operation_id: input.operationId, session_id: null, pane_id: "p1", error: "" };
      return task.flow;
    }
    if (cmd === "complete_task" || cmd === "update_task_status") { stored.find(t => t.id === (input.taskId ?? input.id))!.status = input.status ?? "done"; return {}; }
    if (cmd === "create_task") { const task = { ...board.storedTask, id: "created", name: "新任务", project_id: input.projectId, prompt_json: input.promptJson, priority: input.priority }; stored.push(task); return task; }
    if (cmd === "dispatch_to_herdr") {
      const task = stored.find(t => t.id === input.taskId)!;
      task.status = "in_progress";
      task.binding = { state: "valid", operation: null, target: { machine_id: "remote", session: "default", workspace_id: "w2", pane_id: input.agentName, terminal_id: "remote-term", agent: "codex", process_id: "200", title: task.name, status: "idle" } };
      return;
    }
    if (cmd === "delete_task" || cmd === "archive_task") return;
    return base(cmd, args, opts);
  });
});

it("汇总删除选中远端 Todo 后清理面板，旧读取不能复活任务", async () => {
  await enter();
  fireEvent.click(card("远端待办"));
  await screen.findByRole("complementary", { name: "编辑任务" });
  vi.spyOn(window, "confirm").mockReturnValue(true);
  fireEvent.click(within(card("远端待办")).getByRole("button", { name: "删除" }));
  await waitFor(() => expect(screen.queryByText("远端待办", { selector: ".task-name" })).toBeNull());
  expect(screen.queryByRole("complementary", { name: "编辑任务" })).toBeNull();
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect(screen.queryByText("远端待办", { selector: ".task-name" })).toBeNull();
  expect(card(board.storedTask.name)).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("delete_task", { id: "remote-task" });
});

it("汇总隐藏正在编辑的远端 Workspace 时仍保护草稿", async () => {
  await enter();
  fireEvent.click(card("远端待办"));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  fireEvent.change(within(editor).getByLabelText("标题"), { target: { value: "隐藏前的草稿" } });
  fireEvent.click(screen.getByRole("button", { name: "从侧栏隐藏 远端项目" }));
  const confirm = await screen.findByRole("dialog", { name: "隐藏 Workspace？" });
  fireEvent.click(within(confirm).getByRole("button", { name: "确认隐藏" }));
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "hide_project")).toBe(false);
  expect((screen.getByLabelText("标题") as HTMLInputElement).value).toBe("隐藏前的草稿");
});

it("汇总未知派发核实使用原操作 ID 且不会重新发送", async () => {
  bindTask();
  stored[1].binding = { state: "unknown", target: null, operation: { id: "remote-op", prompt: "原提示词", created_at: "2026-10-26", target: { ...board.storedTask.binding!.target!, machine_id: "remote", workspace_id: "w2" } } };
  await enter();
  fireEvent.click(within(card("远端待办")).getByRole("button", { name: "核实派发结果" }));
  fireEvent.change(screen.getByLabelText("核实依据"), { target: { value: "已核对原远端窗口" } });
  fireEvent.click(screen.getByRole("button", { name: "确认已发送" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("resolve_dispatch", { taskId: "remote-task", operationId: "remote-op", sent: true, evidence: "已核对原远端窗口" }));
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "dispatch_to_herdr")).toBe(false);
});

it("汇总派发使用远端 Workspace 和保存后的内容，成功后显示该任务终端", async () => {
  await enter();
  fireEvent.click(card("远端待办"));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  fireEvent.paste(within(editor).getByRole("textbox", { name: "提示词" }), { clipboardData: { getData: (type: string) => type === "text/plain" ? "更新的派发内容" : "", files: [] } });
  fireEvent.click(within(card("远端待办")).getByRole("button", { name: "派发" }));
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  const select = await screen.findByRole("combobox", { name: "独立 Agent" });
  fireEvent.change(select, { target: { value: "pane-1" } });
  const dialog = select.closest(".dialog")! as HTMLElement;
  fireEvent.click(within(dialog).getByRole("button", { name: "派发" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("dispatch_to_herdr", expect.objectContaining({ taskId: "remote-task", workspace: "remote-workspace", promptText: expect.stringContaining("更新的派发内容") })));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: "remote-task" })));
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "dispatch_to_herdr")).toHaveLength(1);
});

it("汇总新建明确选择远端 Workspace，保存到该项目并留在汇总", async () => {
  await enter();
  fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
  const choose = await screen.findByRole("dialog", { name: "选择任务所属 Workspace" });
  fireEvent.change(within(choose).getByRole("combobox"), { target: { value: remote.id } });
  fireEvent.click(within(choose).getByRole("button", { name: "继续" }));
  const form = await screen.findByRole("dialog", { name: "新建任务" });
  fireEvent.paste(within(form).getByRole("textbox", { name: "提示词" }), { clipboardData: { getData: (type: string) => type === "text/plain" ? "远端新任务正文" : "", files: [] } });
  fireEvent.click(within(form).getByRole("button", { name: "添加任务" }));
  await screen.findByText("新任务", { selector: ".task-name" });
  expect(invoke).toHaveBeenCalledWith("create_task", expect.objectContaining({ projectId: remote.id, promptJson: expect.stringContaining("远端新任务正文") }));
  expect(screen.getByRole("button", { name: "汇总看板" }).getAttribute("aria-pressed")).toBe("true");
});

function card(name: string) { return screen.getByText(name, { selector: ".task-name" }).closest(".task-card")! as HTMLElement; }
function drag(name: string, status: string) {
  const wrapper = card(name).closest(".card-wrapper")!;
  const dataTransfer = { setData: vi.fn(), getData: () => "remote-task", effectAllowed: "move" };
  fireEvent.dragStart(wrapper, { dataTransfer });
  fireEvent.drop(document.querySelector(`.col-${status}`)!, { dataTransfer });
  fireEvent.dragEnd(wrapper, { dataTransfer });
}

it("汇总远端拖拽先保存草稿，再按任务 ID 启动与完成，同列和重复拖拽不重发", async () => {
  await enter();
  fireEvent.click(card("远端待办"));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  fireEvent.change(within(editor).getByLabelText("标题"), { target: { value: "保存后启动" } });
  drag("远端待办", "in_progress");
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "start_task")).toBe(false);
  drag("远端待办", "in_progress");
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_task", expect.objectContaining({ taskId: "remote-task" })));
  await waitFor(() => expect(card("保存后启动").closest(".col-in_progress")).toBeTruthy());
  drag("保存后启动", "in_progress");
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "start_task")).toHaveLength(1);
  fireEvent.click(within(card("保存后启动")).getByRole("button", { name: "标记完成" }));
  await waitFor(() => expect(card("保存后启动").closest(".col-done")).toBeTruthy());
  expect(invoke).toHaveBeenCalledWith("complete_task", expect.objectContaining({ taskId: "remote-task" }));
  expect(stored[0].status).toBe("todo");
  expect(screen.getByRole("button", { name: "汇总看板" }).getAttribute("aria-pressed")).toBe("true");
});

async function enter() {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "汇总看板" }));
  await screen.findByText("远端待办", { selector: ".task-name" });
}

it("All Tasks 在页内编辑远端 Todo，保存与切回原项目都保护正确草稿", async () => {
  await enter();
  fireEvent.click(screen.getByText("远端待办", { selector: ".task-name" }));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  expect(screen.getByRole("button", { name: "汇总看板" }).getAttribute("aria-pressed")).toBe("true");
  fireEvent.change(within(editor).getByLabelText("标题"), { target: { value: "远端已修改" } });
  fireEvent.click(within(editor).getByRole("button", { name: "保存" }));
  await screen.findByText("远端已修改", { selector: ".task-name" });
  expect(screen.getByRole("complementary", { name: "编辑任务" })).toBe(editor);
  expect(invoke).toHaveBeenCalledWith("update_task", expect.objectContaining({ id: "remote-task", name: "远端已修改" }));
  fireEvent.change(within(editor).getByLabelText("标题"), { target: { value: "继续编辑的草稿" } });
  fireEvent.click(screen.getByRole("button", { name: new RegExp(`^${project.name} ·`) }));
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect((screen.getByLabelText("标题") as HTMLInputElement).value).toBe("继续编辑的草稿");
  expect(screen.getByRole("button", { name: "汇总看板" }).getAttribute("aria-pressed")).toBe("true");
});

it("汇总隔离跨机器实时状态，手选终端不重发且保持全局优先级顺序", async () => {
  bindTask();
  stored = [{ ...board.storedTask, status: "in_progress", priority: "urgent" }, {
    ...board.storedTask, id: "remote-task", project_id: remote.id, name: "远端待办", status: "in_progress", priority: "normal",
    binding: { ...board.storedTask.binding!, target: { ...board.storedTask.binding!.target!, machine_id: "remote" } },
  }];
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "herdr_list_agents"
    ? Promise.resolve([(args as { projectId: string }).projectId === remote.id ? liveAgent(stored[1], "blocked") : liveAgent(stored[0], "working")])
    : base(cmd, args, opts));
  await enter();
  await waitFor(() => expect(cardDot("远端待办").getAttribute("data-status")).toBe("blocked"));
  expect([...document.querySelectorAll(".col-in_progress .task-name")].map(e => e.textContent)).toEqual([board.storedTask.name, "远端待办"]);
  vi.mocked(invoke).mockClear();
  fireEvent.click(screen.getByRole("button", { name: "查看终端：远端待办" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: "remote-task" })));
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect(screen.getByRole("button", { name: "汇总看板" }).getAttribute("aria-pressed")).toBe("true");
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => ["start_task", "dispatch_to_herdr"].includes(cmd))).toBe(false);
});

it("汇总批量删除固定跨项目快照，部分失败保留卡片且成功项不被旧列表复活", async () => {
  stored.forEach(t => { t.status = "done"; });
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "delete_completed_tasks"
    ? (args as { projectId: string }).projectId === remote.id ? Promise.reject(new Error("remote DB failure"))
      : Promise.resolve({ succeeded: ["task-1"], skipped: [], failed: [] })
    : base(cmd, args, opts));
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
  await enter();
  fireEvent.click(screen.getByRole("button", { name: "一键删除" }));
  await screen.findByText("删除完成：成功 1，跳过 0，失败 1");
  expect(confirm).toHaveBeenCalledWith(expect.stringContaining("2 个 Workspace 的 2 个已完成任务"));
  expect(invoke).toHaveBeenCalledWith("delete_completed_tasks", { projectId: project.id, taskIds: ["task-1"] });
  expect(invoke).toHaveBeenCalledWith("delete_completed_tasks", { projectId: remote.id, taskIds: ["remote-task"] });
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect(screen.queryByText(board.storedTask.name, { selector: ".task-name" })).toBeNull();
  expect(card("远端待办")).toBeTruthy();
});

it("保存并继续派发不复用保存前的旧轮询正文", async () => {
  await enter();
  fireEvent.click(card("远端待办"));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  fireEvent.paste(within(editor).getByRole("textbox", { name: "提示词" }), { clipboardData: { getData: (type: string) => type === "text/plain" ? "保存后的新正文" : "", files: [] } });
  const oldRows = stored.map(t => ({ ...t }));
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: (rows: Task[]) => void;
  let delayed = false;
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => {
    if (cmd === "board_tasks" && !delayed) { delayed = true; return new Promise(resolve => { finish = resolve; }); }
    return base(cmd, args, opts);
  });
  fireEvent(window, new Event("focus"));
  fireEvent.click(within(card("远端待办")).getByRole("button", { name: "派发" }));
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  await act(async () => {});
  await act(async () => finish(oldRows));
  const select = await screen.findByRole("combobox", { name: "独立 Agent" });
  fireEvent.change(select, { target: { value: "pane-1" } });
  fireEvent.click(within(select.closest(".dialog")! as HTMLElement).getByRole("button", { name: "派发" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("dispatch_to_herdr", expect.objectContaining({ taskId: "remote-task", promptText: expect.stringContaining("保存后的新正文") })));
});

it("慢查询期间后台刷新不吞掉最后一次卡片选择", async () => {
  await enter();
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: (rows: Task[]) => void;
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "board_tasks" ? new Promise(resolve => { finish = resolve; }) : base(cmd, args, opts));
  fireEvent.click(card("远端待办"));
  fireEvent(window, new Event("focus"));
  await act(async () => finish(stored.map(t => ({ ...t }))));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  expect((within(editor).getByLabelText("标题") as HTMLInputElement).value).toBe("远端待办");
});

it("四个远端慢查询期间本机状态仍可刷新", async () => {
  bindTask(); stored[0] = { ...board.storedTask, status: "in_progress" };
  const scope = [project, ...Array.from({ length: 4 }, (_, i) => ({ ...remote, id: `remote-${i}` }))];
  const base = vi.mocked(invoke).getMockImplementation()!;
  const pending: ((value: []) => void)[] = [];
  let localStatus = "working";
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => {
    if (cmd === "herdr_list_agents") return (args as { projectId: string }).projectId === project.id
      ? Promise.resolve([liveAgent(stored[0], localStatus)]) : new Promise(resolve => { pending.push(resolve); });
    return base(cmd, args, opts);
  });
  const view = render(<TaskSummaryBoard projects={scope} />);
  try {
    await waitFor(() => expect(cardDot(stored[0].name).getAttribute("data-status")).toBe("working"));
    await act(async () => {});
    localStatus = "blocked";
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(cardDot(stored[0].name).getAttribute("data-status")).toBe("blocked"));
  } finally {
    view.unmount();
    while (pending.length) await act(async () => { pending.splice(0).forEach(resolve => resolve([])); });
  }
});

it("旧范围批次完成不能用旧查询清空新 Workspace 的编辑草稿", async () => {
  stored[0].status = "done";
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: (result: { succeeded: string[]; skipped: []; failed: [] }) => void;
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "archive_completed_tasks"
    ? new Promise(resolve => { finish = resolve; }) : base(cmd, args, opts));
  const view = render(<TaskSummaryBoard projects={[project]} />);
  await screen.findByText(board.storedTask.name, { selector: ".task-name" });
  fireEvent.click(screen.getByRole("button", { name: "一键归档" }));
  view.rerender(<TaskSummaryBoard projects={projects} />);
  await screen.findByText("远端待办", { selector: ".task-name" });
  fireEvent.click(card("远端待办"));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  fireEvent.change(within(editor).getByLabelText("标题"), { target: { value: "新范围的草稿" } });
  vi.mocked(invoke).mockClear();
  await act(async () => finish({ succeeded: ["task-1"], skipped: [], failed: [] }));
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect((within(editor).getByLabelText("标题") as HTMLInputElement).value).toBe("新范围的草稿");
  expect(screen.getByRole("complementary", { name: "编辑任务" })).toBe(editor);
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "board_tasks").every(([, args]) => (args as { projectIds: string[] }).projectIds.includes(remote.id))).toBe(true);
});

it("派发准备读取迟到时，不在用户已选择另一任务后打开旧派发框", async () => {
  await enter();
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: (rows: Task[]) => void;
  vi.mocked(invoke).mockImplementation((cmd, args, opts) => cmd === "board_tasks" ? new Promise(resolve => { finish = resolve; }) : base(cmd, args, opts));
  fireEvent.click(within(card("远端待办")).getByRole("button", { name: "派发" }));
  fireEvent.click(card(board.storedTask.name));
  await act(async () => finish(stored.map(t => ({ ...t }))));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  expect((within(editor).getByLabelText("标题") as HTMLInputElement).value).toBe(board.storedTask.name);
  expect(screen.queryByRole("combobox", { name: "独立 Agent" })).toBeNull();
});
