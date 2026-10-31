import { useState, useEffect, useRef, type RefObject } from "react";
import type { Task, Project, TaskStatus, Machine } from "./types";
import { TaskCard } from "./TaskCard";
import { ResetTaskDialog } from "./ResetTaskDialog";
import { DispatchDialog } from "./DispatchDialog";
import { TaskEditPanel, TaskFormDialog, type TaskEditorHandle } from "./TaskFormDialog";
import * as api from "./api";
import { notify } from "./Toast";
import { Icon, type IconName } from "./Icon";
import { DispatchRecovery } from "./DispatchRecovery";
import { TerminalPanel } from "./TerminalPanel";
import { BindingDialog } from "./BindingDialog";
import { tr, errorText, messageText, useLocalizedMessage, type Message } from "./i18n";
import { flowUnresolved, taskColumn } from "./taskFlow";
import { useTaskFlowUpdates } from "./useTaskFlowUpdates";
import { useAgentStatus } from "./useAgentStatus";
import { taskAgentPresentation } from "./agentStatus";
import { machineLabel } from "./WorkspaceAddDialog";
import "./taskSummary.css";
import { CreateTaskDialog } from "./CreateTaskDialog";
import { IdleCleanupButton } from "./IdleCleanupButton";
import { readWorkspaceTaskSelection, rememberWorkspaceTaskSelection } from "./workspaceTaskSelection";

interface KanbanBoardProps {
  project: Project;
  machineName?: string;
  onNewTaskShortcut?: () => void;
  terminalMaximized?: boolean;
  onToggleTerminalMaximized?: () => void;
  editorRef?: RefObject<TaskEditorHandle | null>;
  initialTaskId?: string;
}

const COLUMNS: { status: TaskStatus; label: string; emptyIcon: IconName; emptyText: string; hint: string }[] = [
  { status: "todo", label: "board.todo", emptyIcon: "circle", emptyText: "board.todo_empty", hint: "task.empty_hint" },
  { status: "in_progress", label: "board.in_progress", emptyIcon: "progress", emptyText: "board.in_progress_empty", hint: "board.in_progress_hint" },
  { status: "done", label: "board.done", emptyIcon: "check", emptyText: "board.done_empty", hint: "board.done_hint" },
];

export function KanbanBoard(props: KanbanBoardProps) {
  return <TaskBoard {...props} projects={[props.project]} />;
}

export interface TaskBoardProps extends Omit<KanbanBoardProps, "project"> {
  project?: Project;
  projects: Project[];
  machines?: Machine[];
}

export function TaskBoard({ project, projects, machines = [], machineName, terminalMaximized = false, onToggleTerminalMaximized, editorRef: externalEditorRef, initialTaskId }: TaskBoardProps) {
  const summary = !project;
  const scope = JSON.stringify(projects.map(p => p.id).sort());
  const projectMap = new Map(projects.map(p => [p.id, p]));
  const projectFor = (task: Task) => projectMap.get(task.project_id)!;
  const machineFor = (task?: Task | null) => {
    if (!summary) return machineName;
    const owner = task && projectMap.get(task.project_id);
    return owner ? machineLabel(owner.machine_id ?? "local", machines.find(m => m.id === owner.machine_id)?.name) : undefined;
  };

  const [storedTasks, setTasks] = useState<Task[]>([]);
  const tasks = storedTasks.filter(t => projectMap.has(t.project_id));
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;
  const scopeRef = useRef(scope);
  scopeRef.current = scope;
  const [readError, setReadError] = useLocalizedMessage();
  const [hasLoaded, setHasLoaded] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  // App keys this board by Project ID. Only the initial read replaces its UI;
  // an empty result is still loaded data and must stay visible during refreshes.
  const [initialLoading, setInitialLoading] = useState(true);
  const [creatingTask, setCreatingTask] = useState(false);
  const [dragOverCol, setDragOverCol] = useState<TaskStatus | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const draggingIdRef = useRef<string | null>(null);
  const dragFromControl = useRef(false);
  const [editingTaskId, setEditingTaskId] = useState<string | null>(null);
  const [viewingPromptId, setViewingPromptId] = useState<string | null>(null);
  const [pendingReset, setPendingReset] = useState<{ task: Task; requestId: string } | null>(null);
  const resetInFlight = useRef(false);
  const [resetBusy, setResetBusy] = useState(false);
  const [resetError, setResetError] = useLocalizedMessage();
  const { agents, reads, now, refreshAgents } = useAgentStatus(projects);
  const [pendingDispatch, setPendingDispatch] = useState<{ task: Task; paneId: string } | null>(null);
  const [recoveringTaskId, setRecoveringTaskId] = useState<string | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [shellProjectId, setShellProjectId] = useState<string | null>(null);
  const shellWorkspace = project && shellProjectId === project.id ? project : undefined;
  const [selectedTodoId, setSelectedTodoId] = useState<string | null>(null);
  const localEditorRef = useRef<TaskEditorHandle>(null);
  const editorRef = externalEditorRef ?? localEditorRef;
  const selectedTodo = tasks.find(task => task.id === selectedTodoId && taskColumn(task) === "todo") ?? null;
  const [repairingTaskId, setRepairingTaskId] = useState<string | null>(null);
  const activeFlows = useRef(new Set<string>());
  const optimisticStarts = useRef(new Map<string, Task["flow"]>());
  const [busyTasks, setBusyTasks] = useState(new Set<string>());
  const [flowError, setFlowError] = useLocalizedMessage();
  const bulkTaskIdsRef = useRef(new Set<string>());
  const [bulkTaskIds, setBulkTaskIds] = useState(new Set<string>());
  const [bulkAction, setBulkAction] = useState<"archive" | "delete" | null>(null);
  const [bulkFeedback, setBulkFeedback] = useState<{ message: Message; issues: { id: string; name: string; error: string }[]; failed: boolean } | null>(null);
  const removedTaskIds = useRef(new Set<string>());
  const selectedTask = tasks.find((task) => task.id === selectedTaskId && taskColumn(task) === "in_progress") ?? null;
  const suppressClick = useRef(false);
  const selectionSequence = useRef(0);
  const workspaceRef = useRef<HTMLDivElement>(null);
  const resizing = useRef(false);
  const widthAdjusted = useRef(false);
  const [terminalWidth, setTerminalWidth] = useState<number | null>(null);
  const [maximumWidth, setMaximumWidth] = useState(800);
  const loadSequence = useRef(0);
  const taskRequest = useRef<{ scope: string; sequence: number; promise: Promise<Task[] | undefined> } | null>(null);
  const mounted = useRef(true);
  const pendingTaskSelection = useRef(initialTaskId);
  const restoreWorkspaceSelection = useRef(!!project);
  const defaultSelectionSuppressed = useRef(summary || !!initialTaskId);
  const [locatedTaskId, setLocatedTaskId] = useState<string | null>(null);
  const agentPresentations = new Map(tasks.map(task => [task.id, taskAgentPresentation(task, agents[task.project_id] ?? [], busyTasks.has(task.id), reads[task.project_id], now)]));
  const agentStatuses = new Map([...agentPresentations].map(([id, observation]) => [id, observation.status]));
  const activeTaskId = selectedTodo?.id ?? selectedTask?.id ?? null;
  useEffect(() => {
    // Initial/failed reads and ordinary shells must not erase the remembered task.
    if (project && hasLoaded && !shellWorkspace) rememberWorkspaceTaskSelection(project.id, activeTaskId);
  }, [project?.id, hasLoaded, shellWorkspace, activeTaskId]);
  useEffect(() => {
    if (!workspaceRef.current) return;
    const observer = new ResizeObserver(() => {
      const width = workspaceRef.current?.clientWidth ?? 0;
      if (width) {
        const max = Math.max(280, width - 328);
        const appWidth = workspaceRef.current?.closest(".app")?.clientWidth ?? window.innerWidth;
        const defaultWidth = Math.max(280, Math.min(max, appWidth / 3));
        setMaximumWidth(max);
        setTerminalWidth((current) => widthAdjusted.current && current !== null ? Math.min(current, max) : defaultWidth);
      }
    });
    observer.observe(workspaceRef.current);
    return () => observer.disconnect();
  }, []);
  const setWidth = (width: number) => {
    widthAdjusted.current = true;
    setTerminalWidth(Math.max(280, Math.min(maximumWidth, width)));
  };

  async function load(refreshDiscovery = true): Promise<Task[] | undefined> {
    // Old dialog/batch callbacks cannot install a request into a new visible scope.
    if (!mounted.current || scope !== scopeRef.current) return;
    if (!refreshDiscovery && taskRequest.current?.scope === scope) return taskRequest.current.promise;
    if (refreshDiscovery) refreshAgents();
    const sequence = ++loadSequence.current;
    setRefreshing(true);
    // All readers of this request share its reconciliation and result. A poll must
    // not invalidate a click that is awaiting the very same database snapshot.
    const promise = (async () => {
    try {
      const result = await (summary ? api.boardTasks(JSON.parse(scope)) : api.listTasks(project!.id));
      const t = result.filter(task => !removedTaskIds.current.has(task.id)).map(task => optimisticStarts.current.has(task.id)
        ? { ...task, flow: optimisticStarts.current.get(task.id) } : task);
      if (mounted.current && scope === scopeRef.current && sequence === loadSequence.current) {
        tasksRef.current = t;
        setTasks(t);
        setHasLoaded(true);
        setReadError("");
        if (pendingTaskSelection.current) {
          restoreWorkspaceSelection.current = false;
          // Consume the request only on a successful fresh read. Never briefly connect the first task.
          const target = t.find(task => task.id === pendingTaskSelection.current);
          pendingTaskSelection.current = undefined;
          setSelectedTodoId(target && taskColumn(target) === "todo" ? target.id : null);
          setSelectedTaskId(target && taskColumn(target) === "in_progress" ? target.id : null);
          setEditingTaskId(target && taskColumn(target) === "done" ? target.id : null);
          setLocatedTaskId(target?.id ?? null);
          if (!target) notify({ key: "summary.unavailable" }, "error");
        } else if (restoreWorkspaceSelection.current && project) {
          restoreWorkspaceSelection.current = false;
          const rememberedId = readWorkspaceTaskSelection(project.id);
          const target = t.find(task => task.id === rememberedId && task.project_id === project.id
            && !task.archived_at && taskColumn(task) !== "done");
          // Restore only after a successful read; never connect the default task first.
          setSelectedTodoId(target && taskColumn(target) === "todo" ? target.id : null);
          setSelectedTaskId(target && taskColumn(target) === "in_progress" ? target.id
            : t.find(task => task.project_id === project.id && !task.archived_at && taskColumn(task) === "in_progress")?.id ?? null);
        } else {
          setSelectedTodoId(current => t.some(task => task.id === current && taskColumn(task) === "todo") ? current : null);
          setSelectedTaskId((current) => t.some((task) => task.id === current && taskColumn(task) === "in_progress")
            ? current : (!defaultSelectionSuppressed.current || current ? t.find((task) => taskColumn(task) === "in_progress")?.id ?? null : null));
        }
        return t;
      }
    } catch (e) {
      if (mounted.current && scope === scopeRef.current && sequence === loadSequence.current) { setReadError(String(e)); if (!summary) notify({ error: String(e) }, "error"); }
    } finally {
      if (taskRequest.current?.sequence === sequence) taskRequest.current = null;
      if (mounted.current && scope === scopeRef.current && sequence === loadSequence.current) { setInitialLoading(false); setRefreshing(false); }
    }
    })();
    taskRequest.current = { scope, sequence, promise };
    return promise;
  }

  useEffect(() => { mounted.current = true; taskRequest.current = null; void load(); return () => { mounted.current = false; ++loadSequence.current; }; }, [scope]);
  useEffect(() => { setViewingPromptId(null); setPendingReset(null); setShellProjectId(null); }, [scope]);
  useTaskFlowUpdates(projects.map(p => p.id), async () => { await load(false); });

  useEffect(() => {
    if (!locatedTaskId || initialLoading) return;
    const card = Array.from(workspaceRef.current?.querySelectorAll<HTMLElement>("[data-task-id]") ?? [])
      .find(element => element.dataset.taskId === locatedTaskId);
    card?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [locatedTaskId, initialLoading]);

  function handleRemoved(ids: string[]) {
    if (!mounted.current) return;
    // Removal already committed: a failed list refresh must not resurrect this card.
    const removed = new Set(ids);
    ids.forEach(id => removedTaskIds.current.add(id));
    ++loadSequence.current;
    setTasks((current) => current.filter((task) => !removed.has(task.id)));
    setEditingTaskId((current) => current && removed.has(current) ? null : current);
    setSelectedTodoId((current) => current && removed.has(current) ? null : current);
    setSelectedTaskId((current) => current && removed.has(current) ? null : current);
    setRepairingTaskId((current) => current && removed.has(current) ? null : current);
    setRecoveringTaskId((current) => current && removed.has(current) ? null : current);
    setViewingPromptId((current) => current && removed.has(current) ? null : current);
    setPendingReset((current) => current && removed.has(current.task.id) ? null : current);
    setPendingDispatch((current) => current && removed.has(current.task.id) ? null : current);
    void load();
  }

  async function handleBulkAction(action: "archive" | "delete") {
    if (bulkTaskIdsRef.current.size) return;
    const snapshot = tasks.filter(task => taskColumn(task) === "done");
    if (!snapshot.length) return;
    if (action === "delete" && !confirm(tr(summary ? "summary.delete_confirm" : "batch.delete_confirm", {
      count: snapshot.length, workspaces: new Set(snapshot.map(t => t.project_id)).size,
    }))) return;
    const ids = snapshot.map(task => task.id);
    bulkTaskIdsRef.current = new Set(ids);
    setBulkTaskIds(new Set(ids));
    setBulkAction(action);
    setBulkFeedback(null);
    try {
      const groups = new Map<string, string[]>();
      snapshot.forEach(task => groups.set(task.project_id, [...(groups.get(task.project_id) ?? []), task.id]));
      const batches = await Promise.all([...groups].map(async ([projectId, taskIds]): Promise<api.BatchTaskResult> => {
        try {
          const result = await (action === "archive" ? api.archiveCompletedTasks : api.deleteCompletedTasks)(projectId, taskIds);
          handleRemoved(result.succeeded);
          return result;
        } catch (error) {
          return { succeeded: [], skipped: [], failed: taskIds.map(id => ({ id, error: String(error) })) };
        }
      }));
      const result = { succeeded: batches.flatMap(r => r.succeeded), skipped: batches.flatMap(r => r.skipped), failed: batches.flatMap(r => r.failed) };
      if (!mounted.current) return;
      // Commit local removal before the reconciliation request can fail or return stale data.
      void load();
      const message = { key: `batch.${action}_result`, params: {
        succeeded: result.succeeded.length, skipped: result.skipped.length, failed: result.failed.length,
      } };
      setBulkFeedback({ message, failed: result.failed.length > 0,
        issues: [...result.skipped, ...result.failed].map(issue => ({ id: issue.id,
          name: snapshot.find(task => task.id === issue.id)?.name ?? issue.id, error: issue.error })) });
    } catch (error) {
      if (mounted.current) {
        setBulkFeedback({ message: { error: String(error) }, issues: [], failed: true });
        void load();
      }
    } finally {
      bulkTaskIdsRef.current.clear();
      if (mounted.current) { setBulkTaskIds(new Set()); setBulkAction(null); }
    }
  }

  // Keyboard shortcut: N opens the same form as the toolbar action.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (
        !terminalMaximized && e.key === "n" &&
        !e.ctrlKey && !e.metaKey && !e.altKey &&
        !document.querySelector(".dialog-overlay") &&
        !(document.activeElement instanceof HTMLElement && document.activeElement.closest(".terminal-panel")) &&
        !(document.activeElement instanceof HTMLElement && document.activeElement.isContentEditable) &&
        !(document.activeElement instanceof HTMLInputElement) &&
        !(document.activeElement instanceof HTMLSelectElement) &&
        !(document.activeElement instanceof HTMLTextAreaElement)
      ) {
        void createTask();
        e.preventDefault();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [terminalMaximized]);

  async function createTask() {
    const selection = ++selectionSequence.current;
    if (editorRef.current && !await leaveTodo()) return;
    if (!mounted.current || selection !== selectionSequence.current) return;
    setCreatingTask(true);
  }

  async function openWorkspaceTerminal() {
    if (!project) return;
    const selection = ++selectionSequence.current;
    const selectionScope = scope;
    if (await editorRef.current?.requestLeave() === false) return;
    if (!mounted.current || selectionScope !== scopeRef.current || selection !== selectionSequence.current) return;
    // Clear the editor and show the shell together, without recording the hidden task.
    setSelectedTodoId(null);
    setShellProjectId(project.id);
  }

  async function leaveTodo() {
    if (await editorRef.current?.requestLeave() === false || !mounted.current) return false;
    setSelectedTodoId(null);
    return true;
  }

  async function selectTask(task: Task) {
    if (suppressClick.current || selectedTodoId === task.id) return;
    const selection = ++selectionSequence.current;
    const selectionScope = scope;
    if (editorRef.current && !await leaveTodo()) return;
    if (summary) {
      const fresh = await load(false);
      if (!fresh) return;
      const target = fresh.find(t => t.id === task.id);
      if (!target) { notify({ key: "summary.unavailable" }, "error"); return; }
      task = target;
    }
    if (!mounted.current || selectionScope !== scopeRef.current || selection !== selectionSequence.current) return;
    setShellProjectId(null);
    if (taskColumn(task) === "todo") setSelectedTodoId(task.id);
    else if (taskColumn(task) === "done") setEditingTaskId(task.id);
    else { defaultSelectionSuppressed.current = false; setSelectedTaskId(task.id); }
  }

  function editTask(id: string | null) {
    const task = tasksRef.current.find(task => task.id === id);
    if (task && taskColumn(task) === "todo") void selectTask(task);
    else { void (async () => {
      const selection = ++selectionSequence.current;
      if (editorRef.current && !await leaveTodo()) return;
      if (mounted.current && selection === selectionSequence.current) setEditingTaskId(id);
    })(); }
  }

  async function handleDrop(e: React.DragEvent, targetStatus: TaskStatus) {
    e.preventDefault();
    const taskId = draggingIdRef.current ?? e.dataTransfer.getData("taskId");
    setDragOverCol(null);
    setDraggingId(null);
    draggingIdRef.current = null;
    if (!taskId) return;
    const task = tasks.find((t) => t.id === taskId);
    if (!task || taskColumn(task) === targetStatus) return;
    await moveTask(task, targetStatus);
  }

  async function openTaskAction(task: Task, action: "view" | "reset") {
    const selection = ++selectionSequence.current;
    const actionScope = scope;
    if (editorRef.current && !await leaveTodo()) return;
    if (!mounted.current || actionScope !== scopeRef.current || selection !== selectionSequence.current) return;
    const current = tasksRef.current.find(t => t.id === task.id);
    if (!current) return;
    task = current;
    if (action === "view") setViewingPromptId(task.id);
    else { setResetError(""); setPendingReset({ task, requestId: crypto.randomUUID() }); }
  }

  async function resetTask() {
    if (!pendingReset || resetInFlight.current) return;
    const { task, requestId } = pendingReset;
    const selection = ++selectionSequence.current;
    const resetScope = scope;
    resetInFlight.current = true; setResetBusy(true); setResetError("");
    try {
      const reset = await api.resetTask(task, requestId);
      optimisticStarts.current.delete(task.id);
      ++loadSequence.current;
      // A committed reset remains visible even if the subsequent refresh fails.
      setTasks(current => current.map(t => t.id === task.id ? reset : t));
      setPendingReset(null);
      if (mounted.current && resetScope === scopeRef.current && selection === selectionSequence.current) {
        if (taskColumn(reset) === "todo") { setSelectedTaskId(null); setSelectedTodoId(task.id); }
      }
      void load();
    } catch (error) { setResetError(String(error)); }
    finally { resetInFlight.current = false; setResetBusy(false); }
  }

  async function moveTask(task: Task, status: TaskStatus) {
    if (bulkTaskIdsRef.current.has(task.id)) return;
    if (activeFlows.current.has(task.id) || (flowUnresolved(task) && !(status === "done" && task.flow?.kind === "complete" && task.flow.state === "unknown"))) return;
    if (editorRef.current && (status === "in_progress" || selectedTodoId === task.id) && !await leaveTodo()) return;
    if (activeFlows.current.has(task.id) || !mounted.current) return;
    const current = tasksRef.current.find(t => t.id === task.id);
    if (!current) return;
    task = current;
    if (flowUnresolved(task) && !(status === "done" && task.flow?.kind === "complete" && task.flow.state === "unknown")) return;
    activeFlows.current.add(task.id);
    setBusyTasks(new Set(activeFlows.current));
    setFlowError("");
    const operationId = crypto.randomUUID();
    if (status === "in_progress") {
      const flow: Task["flow"] = { session_id: task.flow?.session_id ?? null, operation_id: operationId,
        kind: "start", state: "pending", error: "", pane_id: null, stage: "queued", requested_status: "in_progress" };
      optimisticStarts.current.set(task.id, flow);
      ++loadSequence.current;
      setTasks(current => current.map(t => t.id === task.id ? { ...t, flow } : t));
      setShellProjectId(null);
      setSelectedTaskId(task.id);
    }
    try {
      if (status === "in_progress") {
        await api.startTask(task.id, operationId);
      } else {
        const result = await api.updateTaskStatus(task.id, status);
        if (result?.warning) setFlowError({ prefix: "flow.cleanup_warning", error: result.warning });
      }
      optimisticStarts.current.delete(task.id);
      await load();
    } catch (e) {
      optimisticStarts.current.delete(task.id);
      setTasks(current => current.map(t => t.id === task.id ? { ...t, flow: task.flow } : t));
      setFlowError(String(e));
      await load();
    } finally {
      activeFlows.current.delete(task.id);
      setBusyTasks(new Set(activeFlows.current));
    }
  }

  async function recoverFlow(task: Task) {
    if (bulkTaskIdsRef.current.has(task.id)) return;
    if (!task.flow?.operation_id || activeFlows.current.has(task.id)) return;
    if (task.flow.kind === "complete") { await moveTask(task, "done"); return; }
    activeFlows.current.add(task.id); setBusyTasks(new Set(activeFlows.current));
    try { await api.recoverLifecycle(task.id, task.flow.operation_id); setFlowError(""); }
    catch (e) { setFlowError(String(e)); }
    finally { activeFlows.current.delete(task.id); setBusyTasks(new Set(activeFlows.current)); await load(); }
  }

  return (
    <div role={summary ? "region" : undefined} aria-label={summary ? tr("summary.title") : undefined} className={`kanban-board board-workspace${summary ? " summary-board" : ""}`} ref={workspaceRef} style={terminalWidth === null ? undefined : { "--terminal-width": `${terminalWidth}px` } as React.CSSProperties} onDragOver={(e) => e.preventDefault()}>
      <div className="board-main" inert={terminalMaximized}>
        {initialLoading ? <div className="board-loading"><span className="spinner" /><span>{tr("common.loading")}</span></div> : <>
        {project ? <div className="workspace-bar">
          <h1 className="workspace-project" title={project.name}>{project.name}</h1>
          {machineName && <span className="machine-source" title={machineName}>{machineName}</span>}
          <span className="breadcrumb-sep">/</span>
          <span className="workspace-current" title={project.herdr_workspace}>{project.herdr_workspace}</span>
          <span className="board-meta">
            <Icon name="folder" />
            <span className="project-path" title={project.root_path}>{project.root_path}</span>
          </span>
        </div> : <header className="summary-header"><div><h1><Icon name="board" />{tr("summary.title")}</h1><p>{tr("summary.scope")}</p></div><button className="btn-secondary" disabled={refreshing} onClick={() => void load()}>{tr(refreshing ? "overview.refreshing" : "overview.refresh")}</button></header>}
        <div className="board-content">
          {summary && <div className="summary-meta"><span>{tr("summary.count", { workspaces: projects.length, tasks: tasks.length })}</span><span>{tr("summary.order")}</span></div>}
          {readError && <p role="alert" className="error">{tr(hasLoaded ? "summary.stale" : "summary.failed")} {readError} <button className="btn-secondary" disabled={refreshing} onClick={() => void load()}>{tr("overview.retry")}</button></p>}
          {summary && !projects.length && <p>{tr("summary.empty_workspaces")}</p>}
          {flowError && <p role="alert" className="error">{flowError}</p>}
          <div className="board-toolbar">
            <div className="board-view"><Icon name="board" /><h2>{tr("board.title")}</h2><span className="toolbar-count">{tasks.length}</span></div>
            <div className="board-toolbar-actions">
              <span className="board-summary">{tr("board.summary", { done: tasks.filter(task => taskColumn(task) === "done").length, total: tasks.length })}</span>
              <button className="btn-primary" onClick={() => void createTask()}><Icon name="plus" />{tr("task.create")}<kbd>N</kbd></button>
              {project && <button className="btn-secondary" aria-pressed={!!shellWorkspace} onClick={() => void openWorkspaceTerminal()}>{tr("workspace_terminal.open")}</button>}
              {project && <IdleCleanupButton key={project.id} projectId={project.id} workspace={project.name}
                machine={machineName ?? machineLabel(project.machine_id ?? "local", undefined)} onChanged={() => void load()} />}
            </div>
          </div>
          {(hasLoaded || !summary) && <div className="kanban-columns">
          {COLUMNS.map(({ status, label, emptyIcon, emptyText, hint }) => {
            const col = tasks.filter((t) => taskColumn(t) === status);
            if (!summary && status === "in_progress") col.sort((a, b) =>
              Number(agentStatuses.get(b.id) === "blocked") - Number(agentStatuses.get(a.id) === "blocked"));
            return (
              <div
                key={status}
                role={summary ? "region" : undefined} aria-label={summary ? tr(label) : undefined}
                className={`kanban-col col-${status}${dragOverCol === status ? " drag-over" : ""}`}
                onDragOver={(e) => { e.preventDefault(); setDragOverCol(status); }}
                onDragLeave={(e) => {
                  if (!e.currentTarget.contains(e.relatedTarget as Node)) {
                    setDragOverCol(null);
                  }
                }}
                onDrop={(e) => handleDrop(e, status)}
              >
                <div className="col-header">
                  <span className="col-title"><Icon name={emptyIcon} />{tr(label)}</span>
                  <div className="col-header-actions">
                    <span className="col-count">{col.length}</span>
                    {status === "done" && <>
                      <button className="btn-icon" disabled={!col.length || !!bulkAction} onClick={() => handleBulkAction("archive")}
                        aria-label={tr("batch.archive")} title={tr("batch.archive")}>
                        {bulkAction === "archive" ? <span className="spinner" /> : <Icon name="archive" />}
                      </button>
                      <button className="btn-icon btn-danger" disabled={!col.length || !!bulkAction} onClick={() => handleBulkAction("delete")}
                        aria-label={tr("batch.delete")} title={tr("batch.delete")}>
                        {bulkAction === "delete" ? <span className="spinner" /> : <Icon name="trash" />}
                      </button>
                    </>}
                  </div>
                </div>
                {status === "done" && bulkFeedback && <div className="col-batch-result" role={bulkFeedback.failed ? "alert" : "status"}>
                  <p>{messageText(bulkFeedback.message)}</p>
                  {bulkFeedback.issues.length > 0 && <details>
                    <summary>{tr("batch.details")}</summary>
                    <ul>{bulkFeedback.issues.map(issue => <li key={issue.id}>{tr("batch.issue", { name: issue.name, error: errorText(issue.error) })}</li>)}</ul>
                  </details>}
                </div>}
                {col.length === 0 ? (
                  <div className="col-empty">
                    <span className="col-empty-icon"><Icon name={emptyIcon} size={22} /></span>
                    <span className="col-empty-text">{tr(emptyText)}</span>
                    <span className="col-empty-hint">{tr(hint)}</span>
                  </div>
                ) : (
                  <div className="col-cards">
                    {col.map((task) => (
                      <div
                        key={task.id}
                        data-task-id={task.id}
                        draggable={!bulkTaskIds.has(task.id) && !busyTasks.has(task.id) && !flowUnresolved(task)}
                        onPointerDownCapture={(event) => {
                          dragFromControl.current = event.target instanceof Element && !!event.target.closest("button, input, select, textarea");
                        }}
                        onDragStart={(e) => {
                          if (dragFromControl.current) { e.preventDefault(); e.stopPropagation(); return; }
                          e.dataTransfer.setData("taskId", task.id);
                          e.dataTransfer.effectAllowed = "move";
                          draggingIdRef.current = task.id;
                          setDraggingId(task.id);
                        }}
                        onDragEnd={() => { draggingIdRef.current = null; setDraggingId(null); suppressClick.current = true; setTimeout(() => { suppressClick.current = false; }, 0); }}
                        onDragOver={(e) => e.preventDefault()}
                        className={`card-wrapper${draggingId === task.id ? " dragging" : ""}`}
                      >
                        <TaskCard
                          task={task}
                          source={summary ? {
                            machineId: projectFor(task).machine_id ?? "local",
                            machineName: machineFor(task) ?? "",
                            workspaceId: projectFor(task).id,
                            workspaceName: projectFor(task).name,
                          } : undefined}
                          agentStatus={agentStatuses.get(task.id)!}
                          agentPresentation={agentPresentations.get(task.id)!}
                          busy={busyTasks.has(task.id)}
                          removing={bulkTaskIds.has(task.id)}
                          onComplete={() => moveTask(task, "done")}
                          onStart={() => moveTask(task, "in_progress")}
                          onFlowRecover={() => recoverFlow(task)}
                          onViewPrompt={() => { void openTaskAction(task, "view"); }}
                          onReset={() => { void openTaskAction(task, "reset"); }}
                          onChanged={load}
                          onRemoved={() => handleRemoved([task.id])}
                          onRecover={() => setRecoveringTaskId(task.id)}
                          selected={!shellWorkspace && (selectedTodoId ?? selectedTaskId) === task.id}
                          onSelect={summary || taskColumn(task) !== "done" ? () => { void selectTask(task); } : undefined}
                          onEditTask={editTask}
                          onDispatchTask={async (id) => {
                            const selection = ++selectionSequence.current;
                            if (editorRef.current && !await leaveTodo()) return;
                            // Mutation/refresh failure must never fall back to stale prompt content.
                            const fresh = await load();
                            const t = fresh?.find((x) => x.id === id);
                            if (t && mounted.current && scope === scopeRef.current && selection === selectionSequence.current) setPendingDispatch({ task: t, paneId: "" });
                          }}
                        />
                      </div>
                    ))}
                  </div>
                )}

              </div>
            );
          })}
          </div>}
        </div>
        </>}
      </div>
      <div role="separator" tabIndex={0} inert={terminalMaximized} aria-label={tr("terminal.adjust_width")} aria-orientation="vertical"
        aria-valuemin={280} aria-valuemax={maximumWidth} aria-valuenow={terminalWidth ?? window.innerWidth / 3} className="terminal-splitter"
        onKeyDown={(e) => { if (e.key === "ArrowLeft" || e.key === "ArrowRight") { e.preventDefault(); setWidth((terminalWidth ?? window.innerWidth / 3) + (e.key === "ArrowLeft" ? 24 : -24)); } }}
        onPointerDown={(e) => { resizing.current = true; e.currentTarget.setPointerCapture(e.pointerId); e.preventDefault(); }}
        onPointerMove={(e) => { if (resizing.current && workspaceRef.current) setWidth(workspaceRef.current.getBoundingClientRect().right - e.clientX); }}
        onPointerUp={() => { resizing.current = false; }} onLostPointerCapture={() => { resizing.current = false; }} />
      {selectedTodo ? <TaskEditPanel key={selectedTodo.id} task={selectedTodo} project={projectFor(selectedTodo)} editorRef={editorRef}
        onClose={() => {
          // An explicit return to the terminal ends the summary jump's selection guard.
          defaultSelectionSuppressed.current = false;
          setSelectedTodoId(null);
          setSelectedTaskId(current => current ?? tasks.find(task => taskColumn(task) === "in_progress")?.id ?? null);
        }} onSaved={async () => { await load(); }} /> :
      <TerminalPanel task={shellWorkspace ? null : selectedTask} workspace={shellWorkspace} machineName={shellWorkspace ? machineName ?? machineLabel(shellWorkspace.machine_id ?? "local", undefined) : machineFor(selectedTask)} maximized={terminalMaximized} onToggleMaximized={onToggleTerminalMaximized} onRepair={!shellWorkspace && selectedTask ? () => {
        if (selectedTask.binding?.operation) setRecoveringTaskId(selectedTask.id);
        else setRepairingTaskId(selectedTask.id);
      } : undefined} />}
      {creatingTask && project && <TaskFormDialog project={project} onClose={() => setCreatingTask(false)} onSaved={async () => { await load(); }} />}
      {creatingTask && summary && <CreateTaskDialog projects={projects} machines={machines} onClose={() => setCreatingTask(false)} onSaved={async () => { await load(); }} />}
      {pendingDispatch && tasks.some(t => t.id === pendingDispatch.task.id) && (
        <DispatchDialog
          key={pendingDispatch.task.id}
          task={tasks.find((task) => task.id === pendingDispatch.task.id) ?? pendingDispatch.task}
          project={projectFor(pendingDispatch.task)}
          preselectedPaneId={pendingDispatch.paneId || undefined}
          onClose={() => setPendingDispatch(null)}
          onDispatched={() => {
            defaultSelectionSuppressed.current = false;
            setShellProjectId(null);
            setSelectedTaskId(pendingDispatch.task.id);
            setPendingDispatch(null); void load();
          }}
          onRefresh={load}
        />
      )}
      {recoveringTaskId && (() => {
        const task = tasks.find((task) => task.id === recoveringTaskId);
        return task ? <DispatchRecovery key={task.id} task={task} onClose={() => setRecoveringTaskId(null)} onSaved={() => { setRecoveringTaskId(null); load(); }} /> : null;
      })()}
      {repairingTaskId && (() => {
        const task = tasks.find((task) => task.id === repairingTaskId);
        return task ? <BindingDialog key={task.id} task={task} project={projectFor(task)} onClose={() => setRepairingTaskId(null)} onSaved={() => { setRepairingTaskId(null); load(); }} /> : null;
      })()}
      {editingTaskId && (() => {
        const t = tasks.find((x) => x.id === editingTaskId);
        return t ? (
          <TaskFormDialog
            key={t.id}
            task={t}
            project={projectFor(t)}
            onClose={() => setEditingTaskId(null)}
            onSaved={() => { setEditingTaskId(null); load(); }}
          />
        ) : null;
      })()}
      {viewingPromptId && (() => {
        const task = tasks.find(t => t.id === viewingPromptId);
        return task ? <TaskFormDialog key={`view-${task.id}`} task={task} project={projectFor(task)} readOnly
          onClose={() => setViewingPromptId(null)} onSaved={() => {}} /> : null;
      })()}
      {pendingReset && <ResetTaskDialog name={pendingReset.task.name} busy={resetBusy} error={resetError}
        onCancel={() => setPendingReset(null)} onConfirm={() => { void resetTask(); }} />}
    </div>
  );
}
