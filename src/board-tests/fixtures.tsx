// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { KanbanBoard } from "../KanbanBoard";
import { taskColumn } from "../taskFlow";
import type { HerdrAgent, Project, Task, TaskStatus } from "../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class { onmessage = () => {}; } }));
const mockFlowEvents = vi.hoisted(() => new Set<(event: { payload: { project_id: string; task_id: string; operation_id: string } }) => void>());
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (_name, callback) => {
  mockFlowEvents.add(callback);
  return () => { mockFlowEvents.delete(callback); };
}) }));
export const flowEvents = mockFlowEvents;

export const project: Project = {
  id: "project-1", name: "测试项目", root_path: "E:/fixture",
  herdr_workspace: "fixture", cs_task_id: null, created_at: "2026-10-23",
  herdr_session: "default", workspace_id: "w1", sync_status: "ready", directory_source: "herdr", herdr_directory: "E:/fixture",
};
export const agent: HerdrAgent = {
  pane_id: "pane-1", name: "codex", title: "测试 Agent", status: "idle",
};


export const board = {} as { storedTask: Task; availableAgents: HerdrAgent[]; extraTasks: Task[]; deleted: boolean };
export const sendPrompt = vi.fn<(input: Record<string, string>) => Promise<void>>();

beforeEach(() => {
  localStorage.clear();
  // jsdom has no text-range geometry; focused Tiptap pastes request it.
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  flowEvents.clear();
  vi.mocked(invoke).mockReset();
  board.storedTask = {
    id: "task-1", project_id: project.id, name: "修复拖放派发", status: "todo", priority: "normal", archived_at: null, cleanup_required: false,
    prompt_json: JSON.stringify({ type: "doc", content: [
      { type: "paragraph", content: [{ type: "text", text: "请修复拖放派发" }] },
    ] }),
    prompt_snapshot: null, dispatched_at: null, dispatched_to: null,
    created_at: "2026-10-23", updated_at: "2026-10-23",
  };
  board.availableAgents = [agent];
  board.extraTasks = [];
  board.deleted = false;
  sendPrompt.mockReset().mockResolvedValue(undefined);
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    const input = args as Record<string, string>;
    switch (command) {
      case "ensure_local_herdr": return;
      case "last_codex_settings": return { model: null, effort: null };
      case "codex_model_catalog": return { models: [], source: "cli_catalog", cached: false, error: null };
      case "list_projects": return [];
      case "project_git_statuses": return (args as { projectIds: string[] }).projectIds.map(project_id => ({ project_id, root_path: project.root_path, state: "branch", head: "feature/fixture" }));
      case "task_overview": return (args as { projectIds: string[] }).projectIds.map(project_id => ({
        project_id, todo: 0, done: 0, today_done: 0,
        in_progress: [...(board.deleted ? [] : [board.storedTask]), ...board.extraTasks]
          .filter(task => task.project_id === project_id && !task.archived_at && taskColumn(task) === "in_progress").length,
      }));
      case "list_tasks": return [...(board.deleted ? [] : [{ ...board.storedTask }]), ...board.extraTasks];
      case "delete_task": board.deleted = true; return;
      case "archive_task": board.deleted = true; return;
      case "create_task": {
        const task = { ...board.storedTask, id: "new-task", name: input.name ?? "自动标题", prompt_json: input.promptJson, priority: input.priority };
        board.extraTasks.push(task as Task);
        return task;
      }
      case "update_task": {
        const task = input.id === board.storedTask.id ? board.storedTask : board.extraTasks.find((t) => t.id === input.id)!;
        if (input.name !== undefined) task.name = input.name;
        task.prompt_json = input.promptJson; task.priority = input.priority as Task["priority"]; return;
      }
      case "herdr_list_agents": return board.availableAgents;
      case "herdr_agent_snapshot": return { agents: board.availableAgents, failures: [] };
      case "complete_task": board.storedTask.status = "done"; return {};
      case "start_task":
        await sendPrompt(input);
        board.storedTask.status = "in_progress"; bindTask();
        board.storedTask.flow = { session_id: null, operation_id: input.operationId, kind: "start", state: "succeeded", error: "", pane_id: "pane-1" };
        return board.storedTask.flow;
      case "update_task_status":
        board.storedTask.status = input.status as TaskStatus;
        return;
      case "resolve_dispatch":
        board.storedTask.binding = { state: "valid", target: board.storedTask.binding?.target ?? null, operation: null };
        return;
      case "repair_binding": board.storedTask.binding = { state: "valid", target: null, operation: null }; return;
      case "terminal_open": return input.connectionId;
      case "terminal_close": case "terminal_bounds": return;
      case "dispatch_to_herdr":
        await sendPrompt(input);
        // Mirror the backend contract: only successful dispatch changes status.
        board.storedTask = { ...board.storedTask, status: "in_progress",
          dispatched_to: `${input.workspace}/${input.agentName}` };
        return;
      default: throw new Error(`Unexpected IPC command: ${command}`);
    }
  });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

export async function renderBoard(status: TaskStatus = "todo") {
  board.storedTask.status = status;
  render(<KanbanBoard project={project} />);
  await screen.findByText(board.storedTask.name, { selector: ".task-name" });
}

export function column(status: TaskStatus) {
  return document.querySelector(`.col-${status}`) as HTMLElement;
}

export function dropOn(target: HTMLElement, emptyTransfer = false, withDragStart = true) {
  const card = screen.getAllByText(board.storedTask.name).find(element => element.closest(".card-wrapper"))!.closest(".card-wrapper")!;
  const dataTransfer = {
    setData: vi.fn(), getData: vi.fn(() => emptyTransfer ? "" : board.storedTask.id),
    effectAllowed: "move",
  };
  if (withDragStart) fireEvent.dragStart(card, { dataTransfer });
  fireEvent.dragOver(target, { dataTransfer });
  fireEvent.drop(target, { dataTransfer });
  fireEvent.dragEnd(card, { dataTransfer });
}

export function expectNoStatusWrite() {
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "update_task_status")).toBe(false);
}

export function bindTask() {
  board.storedTask.binding = { state: "valid", operation: null, target: {
    session: "default", workspace_id: "w1", pane_id: "w1:p1", terminal_id: "term1",
    agent: "codex", process_id: "100", title: "A", status: "idle",
  } };
}

export function secondTask(): Task {
  return { ...board.storedTask, id: "task-2", name: "任务 B", status: "in_progress",
    binding: { state: "valid", operation: null, target: { ...board.storedTask.binding!.target!, pane_id: "w1:p2", terminal_id: "term2" } } };
}

export function liveAgent(task: Task, status: string): HerdrAgent {
  return { ...agent, pane_id: task.binding!.target!.pane_id, status,
    verified_task_id: task.id, target: { ...task.binding!.target!, status } };
}
export function cardDot(name: string) {
  return screen.getByText(name, { selector: ".task-name" }).closest(".task-card")!.querySelector(".agent-status-dot")!;
}
export function cardOrder() {
  return Array.from(column("in_progress").querySelectorAll(".task-name"), element => element.textContent);
}
