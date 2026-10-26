import { useState, useEffect, useRef, type RefObject } from "react";
import type { Task, Project, TaskStatus } from "./types";
import { TaskCard } from "./TaskCard";
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
import { taskAgentStatus } from "./agentStatus";

interface KanbanBoardProps {
  project: Project;
  machineName?: string;
  onNewTaskShortcut?: () => void;
  terminalMaximized?: boolean;
  onToggleTerminalMaximized?: () => void;
  editorRef?: RefObject<TaskEditorHandle | null>;
}

const COLUMNS: { status: TaskStatus; label: string; emptyIcon: IconName; emptyText: string; hint: string }[] = [
  { status: "todo", label: "board.todo", emptyIcon: "circle", emptyText: "board.todo_empty", hint: "task.empty_hint" },
  { status: "in_progress", label: "board.in_progress", emptyIcon: "progress", emptyText: "board.in_progress_empty", hint: "board.in_progress_hint" },
  { status: "done", label: "board.done", emptyIcon: "check", emptyText: "board.done_empty", hint: "board.done_hint" },
];

export function KanbanBoard({ project, machineName, terminalMaximized = false, onToggleTerminalMaximized, editorRef: externalEditorRef }: KanbanBoardProps) {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(true);
  const [creatingTask, setCreatingTask] = useState(false);
  const [dragOverCol, setDragOverCol] = useState<TaskStatus | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const draggingIdRef = useRef<string | null>(null);
  const dragFromControl = useRef(false);
  const [editingTaskId, setEditingTaskId] = useState<string | null>(null);
  const { agents, refreshAgents } = useAgentStatus(project);
  const [pendingDispatch, setPendingDispatch] = useState<{ task: Task; paneId: string } | null>(null);
  const [recoveringTaskId, setRecoveringTaskId] = useState<string | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
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
  const workspaceRef = useRef<HTMLDivElement>(null);
  const resizing = useRef(false);
  const widthAdjusted = useRef(false);
  const [terminalWidth, setTerminalWidth] = useState<number | null>(null);
  const [maximumWidth, setMaximumWidth] = useState(800);
  const loadSequence = useRef(0);
  const taskRequest = useRef<Promise<Task[]> | null>(null);
  const mounted = useRef(true);
  const agentStatuses = new Map(tasks.map(task => [task.id, taskAgentStatus(task, agents, busyTasks.has(task.id))]));
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

  async function load(refreshDiscovery = true) {
    if (!mounted.current) return;
    if (refreshDiscovery) refreshAgents();
    const sequence = ++loadSequence.current;
    setLoading(true);
    const request = !refreshDiscovery && taskRequest.current ? taskRequest.current : api.listTasks(project.id);
    taskRequest.current = request;
    try {
      const result = await request;
      const t = result.filter(task => !removedTaskIds.current.has(task.id)).map(task => optimisticStarts.current.has(task.id)
        ? { ...task, flow: optimisticStarts.current.get(task.id) } : task);
      if (sequence === loadSequence.current) {
        setTasks(t);
        setSelectedTodoId(current => t.some(task => task.id === current && taskColumn(task) === "todo") ? current : null);
        setSelectedTaskId((current) => t.some((task) => task.id === current && taskColumn(task) === "in_progress")
          ? current : t.find((task) => taskColumn(task) === "in_progress")?.id ?? null);
      }
    } catch (e) {
      if (sequence === loadSequence.current) notify({ error: String(e) }, "error");
    } finally {
      if (taskRequest.current === request) taskRequest.current = null;
      if (sequence === loadSequence.current) setLoading(false);
    }
  }

  useEffect(() => { mounted.current = true; load(); return () => { mounted.current = false; ++loadSequence.current; }; }, [project.id]);
  useTaskFlowUpdates(project.id, () => load(false));

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
    setPendingDispatch((current) => current && removed.has(current.task.id) ? null : current);
    void load();
  }

  async function handleBulkAction(action: "archive" | "delete") {
    if (bulkTaskIdsRef.current.size) return;
    const snapshot = tasks.filter(task => taskColumn(task) === "done");
    if (!snapshot.length) return;
    if (action === "delete" && !confirm(tr("batch.delete_confirm").replace("{count}", String(snapshot.length)))) return;
    const ids = snapshot.map(task => task.id);
    bulkTaskIdsRef.current = new Set(ids);
    setBulkTaskIds(new Set(ids));
    setBulkAction(action);
    setBulkFeedback(null);
    try {
      const result = await (action === "archive" ? api.archiveCompletedTasks : api.deleteCompletedTasks)(project.id, ids);
      if (!mounted.current) return;
      // Commit local removal before the reconciliation request can fail or return stale data.
      handleRemoved(result.succeeded);
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
        setCreatingTask(true);
        e.preventDefault();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [terminalMaximized]);

  async function leaveTodo() {
    if (await editorRef.current?.requestLeave() === false || !mounted.current) return false;
    setSelectedTodoId(null);
    return true;
  }

  async function selectTask(task: Task) {
    if (suppressClick.current || selectedTodoId === task.id) return;
    if (editorRef.current && !await leaveTodo()) return;
    if (taskColumn(task) === "todo") setSelectedTodoId(task.id);
    else setSelectedTaskId(task.id);
  }

  function editTask(id: string | null) {
    const task = tasks.find(task => task.id === id);
    if (task && taskColumn(task) === "todo") void selectTask(task);
    else setEditingTaskId(id);
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

  async function moveTask(task: Task, status: TaskStatus) {
    if (bulkTaskIdsRef.current.has(task.id)) return;
    if (activeFlows.current.has(task.id) || (flowUnresolved(task) && !(status === "done" && task.flow?.kind === "complete" && task.flow.state === "unknown"))) return;
    if (editorRef.current && (status === "in_progress" || selectedTodoId === task.id) && !await leaveTodo()) return;
    if (activeFlows.current.has(task.id) || !mounted.current) return;
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
    <div className="kanban-board board-workspace" ref={workspaceRef} style={terminalWidth === null ? undefined : { "--terminal-width": `${terminalWidth}px` } as React.CSSProperties} onDragOver={(e) => e.preventDefault()}>
      <div className="board-main" inert={terminalMaximized}>
        {loading && tasks.length === 0 ? <div className="board-loading"><span className="spinner" /><span>{tr("common.loading")}</span></div> : <>
        <div className="workspace-bar">
          <h1 className="workspace-project" title={project.name}>{project.name}</h1>
          {machineName && <span className="machine-source" title={machineName}>{machineName}</span>}
          <span className="breadcrumb-sep">/</span>
          <span className="workspace-current" title={project.herdr_workspace}>{project.herdr_workspace}</span>
          <span className="board-meta">
            <Icon name="folder" />
            <span className="project-path" title={project.root_path}>{project.root_path}</span>
          </span>
        </div>
        <div className="board-content">
          {flowError && <p role="alert" className="error">{flowError}</p>}
          <div className="board-toolbar">
            <div className="board-view"><Icon name="board" /><h2>{tr("board.title")}</h2><span className="toolbar-count">{tasks.length}</span></div>
            <div className="board-toolbar-actions">
              <span className="board-summary">{tr("board.summary", { done: tasks.filter(task => taskColumn(task) === "done").length, total: tasks.length })}</span>
              <button className="btn-primary" onClick={() => setCreatingTask(true)}><Icon name="plus" />{tr("task.create")}<kbd>N</kbd></button>
            </div>
          </div>
          <div className="kanban-columns">
          {COLUMNS.map(({ status, label, emptyIcon, emptyText, hint }) => {
            const col = tasks.filter((t) => taskColumn(t) === status);
            if (status === "in_progress") col.sort((a, b) =>
              Number(agentStatuses.get(b.id) === "blocked") - Number(agentStatuses.get(a.id) === "blocked"));
            return (
              <div
                key={status}
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
                          agentStatus={agentStatuses.get(task.id)!}
                          busy={busyTasks.has(task.id)}
                          removing={bulkTaskIds.has(task.id)}
                          onComplete={() => moveTask(task, "done")}
                          onStart={() => moveTask(task, "in_progress")}
                          onFlowRecover={() => recoverFlow(task)}
                          onChanged={load}
                          onRemoved={() => handleRemoved([task.id])}
                          onRecover={() => setRecoveringTaskId(task.id)}
                          selected={(selectedTodoId ?? selectedTaskId) === task.id}
                          onSelect={taskColumn(task) !== "done" ? () => { void selectTask(task); } : undefined}
                          onEditTask={editTask}
                          onDispatchTask={async (id) => {
                            if (editorRef.current && !await leaveTodo()) return;
                            const t = tasks.find((x) => x.id === id);
                            if (t) setPendingDispatch({ task: t, paneId: "" });
                          }}
                        />
                      </div>
                    ))}
                  </div>
                )}

              </div>
            );
          })}
          </div>
        </div>
        </>}
      </div>
      <div role="separator" tabIndex={0} inert={terminalMaximized} aria-label={tr("terminal.adjust_width")} aria-orientation="vertical"
        aria-valuemin={280} aria-valuemax={maximumWidth} aria-valuenow={terminalWidth ?? window.innerWidth / 3} className="terminal-splitter"
        onKeyDown={(e) => { if (e.key === "ArrowLeft" || e.key === "ArrowRight") { e.preventDefault(); setWidth((terminalWidth ?? window.innerWidth / 3) + (e.key === "ArrowLeft" ? 24 : -24)); } }}
        onPointerDown={(e) => { resizing.current = true; e.currentTarget.setPointerCapture(e.pointerId); e.preventDefault(); }}
        onPointerMove={(e) => { if (resizing.current && workspaceRef.current) setWidth(workspaceRef.current.getBoundingClientRect().right - e.clientX); }}
        onPointerUp={() => { resizing.current = false; }} onLostPointerCapture={() => { resizing.current = false; }} />
      {selectedTodo ? <TaskEditPanel key={selectedTodo.id} task={selectedTodo} project={project} editorRef={editorRef}
        onClose={() => setSelectedTodoId(null)} onSaved={() => load(false)} /> :
      <TerminalPanel task={selectedTask} machineName={machineName} maximized={terminalMaximized} onToggleMaximized={onToggleTerminalMaximized} onRepair={selectedTask ? () => {
        if (selectedTask.binding?.operation) setRecoveringTaskId(selectedTask.id);
        else setRepairingTaskId(selectedTask.id);
      } : undefined} />}
      {creatingTask && <TaskFormDialog project={project} onClose={() => setCreatingTask(false)} onSaved={load} />}
      {pendingDispatch && (
        <DispatchDialog
          key={pendingDispatch.task.id}
          task={tasks.find((task) => task.id === pendingDispatch.task.id) ?? pendingDispatch.task}
          project={project}
          preselectedPaneId={pendingDispatch.paneId || undefined}
          onClose={() => setPendingDispatch(null)}
          onDispatched={() => { setPendingDispatch(null); load(); }}
          onRefresh={load}
        />
      )}
      {recoveringTaskId && (() => {
        const task = tasks.find((task) => task.id === recoveringTaskId);
        return task ? <DispatchRecovery key={task.id} task={task} onClose={() => setRecoveringTaskId(null)} onSaved={() => { setRecoveringTaskId(null); load(); }} /> : null;
      })()}
      {repairingTaskId && (() => {
        const task = tasks.find((task) => task.id === repairingTaskId);
        return task ? <BindingDialog key={task.id} task={task} project={project} onClose={() => setRepairingTaskId(null)} onSaved={() => { setRepairingTaskId(null); load(); }} /> : null;
      })()}
      {editingTaskId && (() => {
        const t = tasks.find((x) => x.id === editingTaskId);
        return t ? (
          <TaskFormDialog
            key={t.id}
            task={t}
            project={project}
            onClose={() => setEditingTaskId(null)}
            onSaved={() => { setEditingTaskId(null); load(); }}
          />
        ) : null;
      })()}
    </div>
  );
}
