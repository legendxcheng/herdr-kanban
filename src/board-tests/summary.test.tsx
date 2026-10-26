// @vitest-environment jsdom
import { board, project, bindTask, secondTask, sendPrompt, flowEvents } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";
import { TaskSummaryBoard } from "../TaskSummaryBoard";
import { taskColumn } from "../taskFlow";
import { setLocale } from "../i18n";
import type { Project, Task, TaskSummary } from "../types";

const remote: Project = { ...project, id: "remote-project", machine_id: "devpc", workspace_id: "w1", name: project.name };
const machines = [{ id: "devpc", name: "Devpc", kind: "ssh" as const, session: "default", disabled: false, profile_id: "profile", ssh_target: "devpc" }];
let projects: Project[], stored: Task[], rows: TaskSummary[], offline: boolean;

beforeEach(() => {
  setLocale("zh-CN");
  projects = [project, remote]; stored = []; rows = []; offline = false;
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (command, args, options) => {
    if (command === "list_projects") return projects;
    if (command === "list_machines") return machines;
    if (command === "navigation_state") return { configured: true, recovery_projects: [] };
    if (command === "workspace_operations") return [];
    if (command === "sync_projects") {
      if (offline) throw new Error("machine.timeout");
      const machineId = (args as { machineId?: string })?.machineId ?? "local";
      return { projects: projects.filter(p => (p.machine_id ?? "local") === machineId), session: "default", workspaces: [], operations: [] };
    }
    if (command === "task_summary") return rows;
    if (command === "list_tasks") return stored.filter(t => t.project_id === (args as { projectId: string }).projectId);
    return base(command, args, options);
  });
});

function task(id: string, workspace = project, status: Task["status"] = "todo", priority: Task["priority"] = "normal"): Task {
  return { ...board.storedTask, id, name: id, project_id: workspace.id, status, priority };
}
function snapshot() { rows = stored.map(t => ({ ...t, status: taskColumn(t) })); }
function summary() { return screen.getByRole("region", { name: "汇总看板" }); }
function open(name: string) { return within(summary()).getByRole("button", { name: new RegExp(`^打开任务：${name} ·`) }); }
async function enterSummary() {
  fireEvent.click(await screen.findByRole("button", { name: "汇总看板" }));
  await waitFor(() => expect(within(summary()).queryByText("加载中…")).toBeNull());
}
function summaries(tasks: Task[]): TaskSummary[] { return tasks.map(t => ({ ...t, status: taskColumn(t) })); }

it("汇总跨电脑同名 Workspace，三列保持后端全局优先级顺序，查看不连接或启动 Agent", async () => {
  stored = [task("紧急远端", remote, "todo", "urgent"), task("优先本机", project, "todo", "high"), task("一般远端", remote), task("正在启动", remote), task("已完成", project, "done")];
  stored[3].flow = { kind: "start", state: "pending", requested_status: "in_progress", operation_id: "op", session_id: null, pane_id: null, error: "" };
  snapshot();
  const select = vi.fn();
  render(<TaskSummaryBoard projects={projects} machines={machines} onSelect={select} />);
  await screen.findByText("紧急远端");
  const todo = within(summary()).getByRole("region", { name: "未开发" });
  expect([...todo.querySelectorAll(".task-name")].map(e => e.textContent)).toEqual(["紧急远端", "优先本机", "一般远端"]);
  expect(open("紧急远端").textContent).toContain("Devpc");
  expect(open("优先本机").textContent).toContain("local");
  expect(within(within(summary()).getByRole("region", { name: "开发中" })).getByText("正在启动")).toBeTruthy();
  fireEvent.click(open("紧急远端"));
  expect(select).toHaveBeenCalledWith(remote.id, "紧急远端");
  expect(vi.mocked(invoke).mock.calls.some(([name]) => ["terminal_open", "start_task", "dispatch_to_herdr", "herdr_list_agents", "list_tasks"].includes(name))).toBe(false);
});

it.each(["todo", "in_progress", "done"] as const)("从汇总精确跳转远端 %s 任务，轮询不抢选择或误开首张任务", async status => {
  bindTask();
  const first = { ...secondTask(), id: "wrong-first", name: "其他任务", project_id: remote.id };
  const target = { ...secondTask(), id: "target", name: "目标任务", project_id: remote.id, status };
  stored = [first, target]; rows = summaries([target]);
  render(<App />); await enterSummary();
  vi.mocked(invoke).mockClear();
  fireEvent.click(open("目标任务"));
  if (status === "todo") {
    expect(await screen.findByRole("complementary", { name: "编辑任务" })).toBeTruthy();
    expect((screen.getByLabelText("标题") as HTMLInputElement).value).toBe("目标任务");
  } else if (status === "in_progress") {
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: "target" })));
    expect(screen.getByRole("button", { name: "查看终端：目标任务" }).getAttribute("aria-pressed")).toBe("true");
  } else {
    expect(await screen.findByRole("dialog", { name: "编辑任务" })).toBeTruthy();
    expect((screen.getByLabelText("标题") as HTMLInputElement).value).toBe("目标任务");
  }
  fireEvent(window, new Event("focus"));
  await act(async () => {});
  const connections = vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "terminal_open");
  expect(connections.every(([, args]) => (args as { taskId: string }).taskId === "target")).toBe(true);
  if (status !== "in_progress") expect(connections).toHaveLength(0);
  expect(sendPrompt).not.toHaveBeenCalled();
});

it("目标已删除或归档时提示，不打开其他任务的终端；重新打开汇总可再次跳转同一项目", async () => {
  bindTask(); stored = [{ ...secondTask(), project_id: remote.id }];
  rows = summaries([task("已消失", remote, "in_progress")]);
  render(<App />); await enterSummary();
  vi.mocked(invoke).mockClear();
  fireEvent.click(open("已消失"));
  await screen.findByText("该任务已删除、归档，或所属 Workspace 已不再显示。");
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_open")).toBe(false);
  rows = summaries(stored); await enterSummary(); fireEvent.click(open("任务 B"));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: stored[0].id })));
});

it("进入汇总保护 Todo 草稿：取消保留编辑，保存后进入汇总", async () => {
  stored = [board.storedTask]; snapshot();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: `查看并编辑：${board.storedTask.name}` }));
  fireEvent.change(screen.getByLabelText("标题"), { target: { value: "未保存汇总草稿" } });
  fireEvent.click(screen.getByRole("button", { name: "汇总看板" }));
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect(screen.queryByRole("region", { name: "汇总看板" })).toBeNull();
  expect((screen.getByLabelText("标题") as HTMLInputElement).value).toBe("未保存汇总草稿");
  fireEvent.click(screen.getByRole("button", { name: "汇总看板" }));
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  await screen.findByRole("region", { name: "汇总看板" });
  expect(board.storedTask.name).toBe("未保存汇总草稿");
});

it("只读汇总在远端离线仍显示缓存，进入汇总关闭原查看连接但不停止任务", async () => {
  bindTask(); board.storedTask.status = "in_progress";
  stored = [board.storedTask, task("离线远端任务", remote)]; snapshot(); offline = true;
  render(<App />); await screen.findByText("Herdr 已打开");
  const original = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "terminal_open")![1] as { connectionId: string };
  await enterSummary(); await screen.findByText("离线远端任务");
  expect(invoke).toHaveBeenCalledWith("terminal_close", { connectionId: original.connectionId });
  expect(screen.getByRole("button", { name: "汇总看板" }).getAttribute("aria-pressed")).toBe("true");
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => ["complete_task", "start_task", "dispatch_to_herdr"].includes(cmd))).toBe(false);
});

it("慢刷新保留卡片和列，失败标记旧快照，重试更新标题优先级与归档结果", async () => {
  stored = [task("旧标题")]; snapshot();
  const base = vi.mocked(invoke).getMockImplementation()!;
  render(<TaskSummaryBoard projects={projects} machines={machines} onSelect={vi.fn()} />);
  await screen.findByText("旧标题"); await act(async () => {});
  const old = screen.getByText("旧标题");
  let reject!: (reason: Error) => void;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "task_summary" ? new Promise((_resolve, no) => { reject = no; }) : base(cmd, args, options));
  fireEvent(window, new Event("focus"));
  expect(screen.getByText("旧标题")).toBe(old);
  expect(within(summary()).queryByText("加载中…")).toBeNull();
  await act(async () => reject(new Error("read failed")));
  expect(within(summary()).getByRole("alert").textContent).toContain("上次成功读取");
  expect(screen.getByText("旧标题")).toBe(old);
  rows = summaries([task("新标题", project, "done", "urgent")]);
  vi.mocked(invoke).mockImplementation(base);
  fireEvent.click(within(summary()).getByRole("button", { name: "重试" }));
  await screen.findByText("新标题"); expect(screen.queryByText("旧标题")).toBeNull();
  expect(open("新标题").textContent).toContain("紧急");
  rows = []; fireEvent(window, new Event("focus"));
  await waitFor(() => expect(screen.queryByText("新标题")).toBeNull());
  const column = within(summary()).getByRole("region", { name: "未开发" });
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect(within(summary()).getByRole("region", { name: "未开发" })).toBe(column);
});

it("隐藏 Workspace 立即隔离旧范围及迟到响应；所有项目隐藏后仍保留汇总入口", async () => {
  stored = [task("本机"), task("远端", remote)]; snapshot();
  const base = vi.mocked(invoke).getMockImplementation()!;
  const view = render(<TaskSummaryBoard projects={projects} machines={machines} onSelect={vi.fn()} />);
  await screen.findByText("远端"); await act(async () => {});
  let finish!: (value: TaskSummary[]) => void;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "task_summary" ? new Promise(resolve => { finish = resolve; }) : base(cmd, args, options));
  fireEvent(window, new Event("focus"));
  const finishOld = finish;
  view.rerender(<TaskSummaryBoard projects={[project]} machines={machines} onSelect={vi.fn()} />);
  expect(screen.queryByText("远端")).toBeNull();
  await act(async () => finish(summaries([stored[0]])));
  await act(async () => finishOld(rows));
  expect(screen.queryByText("远端")).toBeNull(); expect(screen.getByText("本机")).toBeTruthy();
  view.rerender(<TaskSummaryBoard projects={[]} machines={machines} onSelect={vi.fn()} />);
  await screen.findByText("尚未选择显示的 Workspace，请从侧栏添加。");
  expect(screen.queryByText("本机")).toBeNull();
});

it("标题事件更新结果且关闭汇总后退订；切换语言仍可访问汇总", async () => {
  stored = [task("原始标题")]; snapshot();
  const view = render(<TaskSummaryBoard projects={projects} machines={machines} onSelect={vi.fn()} />);
  await screen.findByText("原始标题"); await act(async () => {});
  rows = summaries([task("自动标题")]);
  await act(async () => { flowEvents.forEach(fn => fn({ payload: { project_id: project.id, task_id: "task", operation_id: "op" } })); });
  await screen.findByText("自动标题");
  act(() => setLocale("en-US"));
  expect(screen.getByRole("region", { name: "All tasks" })).toBeTruthy();
  expect(screen.getByRole("button", { name: /^Open task: 自动标题/ })).toBeTruthy();
  view.unmount(); expect(flowEvents.size).toBe(0);
  setLocale("zh-CN");
});

it("初次查询失败显示重试，慢请求跨轮询仍能完成而不反复清空", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "task_summary" ? Promise.reject(new Error("database unavailable")) : base(cmd, args, options));
  render(<TaskSummaryBoard projects={projects} machines={machines} onSelect={vi.fn()} />);
  await within(summary()).findByRole("alert");
  expect(within(summary()).queryByText("暂无待办任务")).toBeNull();
  await act(async () => {});
  let finish!: (value: TaskSummary[]) => void;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "task_summary" ? new Promise(resolve => { finish = resolve; }) : base(cmd, args, options));
  vi.useFakeTimers();
  fireEvent.click(within(summary()).getByRole("button", { name: "重试" }));
  await act(async () => { vi.advanceTimersByTime(6000); });
  await act(async () => finish(summaries([task("慢查询完成")])));
  expect(screen.getByText("慢查询完成")).toBeTruthy();
  expect(within(summary()).queryByRole("alert")).toBeNull();
  vi.useRealTimers();
});

it("汇总旧快照的目标已转为完成时，按新鲜状态打开完成任务弹窗", async () => {
  bindTask();
  const target = task("刚刚完成", remote, "done");
  stored = [{ ...secondTask(), project_id: remote.id }, target];
  rows = [{ ...target, status: "in_progress" }];
  render(<App />); await enterSummary();
  fireEvent.click(open("刚刚完成"));
  await screen.findByRole("dialog", { name: "编辑任务" });
  expect((screen.getByLabelText("标题") as HTMLInputElement).value).toBe("刚刚完成");
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_open")).toBe(false);
});

it("从汇总打开 Todo 不连接其他任务，明确关闭编辑器后恢复当前 Workspace 的进行中任务", async () => {
  bindTask();
  const working = { ...secondTask(), project_id: remote.id };
  stored = [working, task("待办详情", remote)]; snapshot();
  render(<App />); await enterSummary();
  fireEvent.click(open("待办详情"));
  const editor = await screen.findByRole("complementary", { name: "编辑任务" });
  fireEvent(window, new Event("focus")); await act(async () => {});
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "terminal_open")).toBe(false);
  fireEvent.click(within(editor).getAllByRole("button", { name: "关闭" })[0]);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_open", expect.objectContaining({ taskId: working.id })));
  expect(screen.getByRole("button", { name: "查看终端：任务 B" }).getAttribute("aria-pressed")).toBe("true");
  expect(sendPrompt).not.toHaveBeenCalled();
});
