import { invoke, Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { tr } from "./i18n";
import type {
  Project, ProjectGitStatus, ProjectSnapshot, WorkspaceOperation, Task, TaskPriority, FileEntry, SkillEntry, HerdrAgent, DispatchRecord,
} from "./types";

// Projects
export const listProjects = () => invoke<Project[]>("list_projects");
export const syncProjects = () => invoke<ProjectSnapshot>("sync_projects");
export const projectGitStatuses = (projectIds: string[]) => invoke<ProjectGitStatus[]>("project_git_statuses", { projectIds });
export const moveProject = (projectId: string, beforeProjectId: string | null) =>
  invoke<ProjectSnapshot>("move_project", { projectId, beforeProjectId });
export const workspaceOperations = () => invoke<WorkspaceOperation[]>("workspace_operations");
export const createProject = (name: string, rootPath: string, operationId: string) => invoke<Project>("create_project", { name, rootPath, operationId });
export const updateProject = (id: string, name: string, fallbackPath: string | null) => invoke<Project>("update_project", { id, name, fallbackPath });
export const linkWorkspace = (projectId: string | null, workspaceId: string, fallbackPath: string | null) => invoke<Project>("link_workspace", { projectId, workspaceId, fallbackPath });
export const recoverWorkspace = (operationId: string, workspaceId: string | null = null) => invoke<Project>("recover_workspace", { operationId, workspaceId });
export const deleteProject = (id: string) => invoke<void>("delete_project", { id });

// Tasks
export interface ProjectCounts { project_id: string; todo: number; in_progress: number; done: number; today_done: number }
export const taskOverview = (projectIds: string[], dayStart: string, dayEnd: string) =>
  invoke<ProjectCounts[]>("task_overview", { projectIds, dayStart, dayEnd });
export const listTasks = (projectId: string) => invoke<Task[]>("list_tasks", { projectId });
export const createTask = (projectId: string, priority: TaskPriority, promptJson: string, draftId?: string) =>
  invoke<Task>("create_task", { projectId, priority, promptJson, draftId, locale: document.documentElement.lang });
export const updateTask = (id: string, name: string | undefined, promptJson: string, priority: TaskPriority, draftId?: string) =>
  invoke<void>("update_task", { id, name, promptJson, priority, draftId });
export interface PromptImage { id: string; name: string; mime: string; bytes: number; width: number; height: number }
export const openImageDraft = (projectId: string, taskId?: string) => invoke<string>("open_image_draft", { projectId, taskId });
export const importPromptImage = (draftId: string, name: string, mime: string, data: string) => invoke<PromptImage>("import_prompt_image", { draftId, name, mime, data });
export const previewPromptImage = (draftId: string, imageId: string) => invoke<string>("preview_prompt_image", { draftId, imageId });
export const closeImageDraft = (draftId: string) => invoke<void>("close_image_draft", { draftId });
export const updateTaskStatus = (id: string, status: string) =>
  status === "done" ? completeTask(id, crypto.randomUUID()) : invoke<FlowOutcome>("update_task_status", { id, status });
export interface FlowOutcome { warning?: string | null }
export const startTask = (taskId: string, operationId: string) =>
  invoke<NonNullable<Task["flow"]>>("start_task", { taskId, operationId });
export interface StartChanged { project_id: string; task_id: string; operation_id: string }
export const onStartChanged = (onChange: (event: StartChanged) => void) =>
  listen<StartChanged>("task-start-changed", event => onChange(event.payload));
export const onTitleChanged = (onChange: (event: { project_id: string }) => void) =>
  listen<{ project_id: string }>("task-title-changed", event => onChange(event.payload));
export const completeTask = (taskId: string, operationId: string) =>
  invoke<FlowOutcome>("complete_task", { taskId, operationId });
export const recoverLifecycle = (taskId: string, operationId: string) =>
  invoke<void>("recover_lifecycle", { taskId, operationId });
export const deleteTask = (id: string) => invoke<void>("delete_task", { id });
export const archiveTask = (id: string) => invoke<void>("archive_task", { id });
export interface BatchTaskResult {
  succeeded: string[];
  skipped: { id: string; error: string }[];
  failed: { id: string; error: string }[];
}
export const archiveCompletedTasks = (projectId: string, taskIds: string[]) =>
  invoke<BatchTaskResult>("archive_completed_tasks", { projectId, taskIds });
export const deleteCompletedTasks = (projectId: string, taskIds: string[]) =>
  invoke<BatchTaskResult>("delete_completed_tasks", { projectId, taskIds });

// Discovery
export const searchFiles = (rootPath: string, query: string) =>
  invoke<FileEntry[]>("search_files", { rootPath, query });
export const searchSkills = (rootPath: string, query: string) =>
  invoke<SkillEntry[]>("search_skills", { rootPath, query });
export const readFileContent = (absPath: string) =>
  invoke<string>("read_file_content", { absPath });
export const readSkillContent = (skillMdPath: string) =>
  invoke<string>("read_skill_content", { skillMdPath });

// Herdr
export const herdrListAgents = (projectId: string) =>
  invoke<HerdrAgent[]>("herdr_list_agents", { projectId });
export const dispatchToHerdr = (
  taskId: string, agentName: string, workspace: string, promptText: string, operationId: string, promptJson?: string,
) => invoke<DispatchRecord>("dispatch_to_herdr", {
  taskId, agentName, workspace, promptText, operationId, ...(promptJson === undefined ? {} : { promptJson }),
});
export const resolveDispatch = (taskId: string, operationId: string, sent: boolean, evidence: string) =>
  invoke<void>("resolve_dispatch", { taskId, operationId, sent, evidence });
export const repairBinding = (taskId: string, paneId: string) => invoke<void>("repair_binding", { taskId, paneId });
export interface TerminalOutput { kind: "closed"; message: string }
export interface TerminalBounds { x: number; y: number; width: number; height: number; viewport_width: number; viewport_height: number; visible: boolean }
export const openTerminal = (taskId: string, connectionId: string, bounds: TerminalBounds, onOutput: (packet: TerminalOutput) => void) => {
  const output = new Channel<TerminalOutput>();
  output.onmessage = onOutput;
  return invoke<string>("terminal_open", { taskId, connectionId, bounds, output });
};
export const closeTerminal = (connectionId: string) => invoke<void>("terminal_close", { connectionId });
export const setTerminalBounds = (connectionId: string, bounds: TerminalBounds) => invoke<void>("terminal_bounds", { connectionId, bounds });

// Native folder picker (returns null if cancelled)
export const pickFolder = () => invoke<string | null>("pick_folder", { title: tr("folder.title") });
