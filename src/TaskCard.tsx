import { useId, useRef, useState } from "react";
import type { Task } from "./types";
import * as api from "./api";
import { notify } from "./Toast";
import { Icon } from "./Icon";
import { errorText, tr, useLocalizedMessage } from "./i18n";
import { canResetTask, flowPending, flowUnresolved, preparationLabel, taskColumn } from "./taskFlow";
import { AgentStatusDot, type AgentStatus } from "./agentStatus";
import { identityColor } from "./identityColors";

interface TaskCardProps {
  task: Task;
  source?: { machineId: string; machineName: string; workspaceId: string; workspaceName: string };
  agentStatus: AgentStatus;
  onChanged: () => void;
  onRemoved: () => void;
  onEditTask: (id: string | null) => void;
  onDispatchTask: (id: string) => void;
  onRecover?: () => void;
  onSelect?: () => void;
  selected?: boolean;
  busy?: boolean;
  removing?: boolean;
  onComplete: () => void;
  onStart: () => void;
  onFlowRecover?: () => void;
  onViewPrompt?: () => void;
  onReset?: () => void;
}

export function TaskCard({ task, source, agentStatus, onChanged, onRemoved, onEditTask, onDispatchTask, onRecover, onSelect, selected, busy, removing, onComplete, onStart, onFlowRecover, onViewPrompt, onReset }: TaskCardProps) {
  const archivingRef = useRef(false);
  const deletingRef = useRef(false);
  const [deleting, setDeleting] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const [archiveError, setArchiveError] = useLocalizedMessage();
  const archiveReason = task.binding?.operation ? "dispatch.verify_required"
    : ["pending", "unknown"].includes(task.flow?.state ?? "") ? "flow.verify_required"
    : task.cleanup_required ? "archive.cleanup_required" : "";
  const disabled = !!busy || !!removing || archiving || deleting;
  const controlsDisabled = disabled || flowUnresolved(task);
  const preparation = preparationLabel(task);
  const compact = taskColumn(task) === "in_progress";
  const [detailsOpen, setDetailsOpen] = useState(false);
  const detailsId = useId();
  const hasModel = !!(task.codex_settings?.model || task.codex_settings?.effort);
  const statusKey = task.flow?.state === "unknown" || task.binding?.operation ? "task.compact_verify"
    : task.flow?.state === "failed" || task.flow?.error || task.cleanup_required || canResetTask(task) ? "task.compact_failed"
    : preparation || busy ? "task.compact_preparing" : null;
  const toggleDetails = () => setDetailsOpen(open => !open);
  const sourceLabel = source ? `${source.machineName} · ${source.workspaceName}` : "";
  const sourceContent = source && <>
    <span className="machine-source" style={{ color: identityColor("machine", source.machineId) }}>{source.machineName}</span>
    <span className="summary-source-separator"> · </span>
    <span className="summary-workspace" style={{ color: identityColor("workspace", source.workspaceId) }}>{source.workspaceName}</span>
  </>;
  const controls = {
    onClick: (event: React.MouseEvent) => event.stopPropagation(),
    onKeyDown: (event: React.KeyboardEvent) => event.stopPropagation(),
  };

  async function handleArchive() {
    if (archivingRef.current || busy || removing || archiveReason) return;
    archivingRef.current = true;
    setArchiving(true); setArchiveError("");
    try {
      await api.archiveTask(task.id);
      notify({ key: "archive.success" }, "success");
      onRemoved();
    } catch (error) {
      setArchiveError(String(error));
    } finally {
      archivingRef.current = false; setArchiving(false);
    }
  }

  async function handleDelete() {
    if (deletingRef.current || archivingRef.current || removing) return;
    if (!confirm(tr("task.delete_confirm", { name: task.name }))) return;
    deletingRef.current = true;
    setDeleting(true);
    try {
      await api.deleteTask(task.id);
      notify({ key: "task.deleted" }, "info");
      onRemoved();
    } catch (e) {
      notify({ error: String(e) }, "error");
    } finally {
      deletingRef.current = false;
      setDeleting(false);
    }
  }

  return (
    <div className={`task-card status-${taskColumn(task)}${selected ? " selected" : ""}`} onClick={onSelect}>
      {!compact && source && <div className="summary-card-source" title={sourceLabel}>{sourceContent}</div>}
      <div className="task-heading">
        <div className="task-header" role={onSelect ? "button" : undefined} tabIndex={onSelect ? 0 : undefined}
          aria-label={onSelect ? `${tr(taskColumn(task) !== "in_progress" ? "task.view_edit" : "terminal.view")}${task.name}` : undefined} aria-pressed={onSelect ? !!selected : undefined}
          onKeyDown={(event) => { if (onSelect && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); onSelect(); } }}>
          <AgentStatusDot status={agentStatus} />
          <span className="task-name" title={task.name}>{task.name}</span>
        </div>
        <div className="task-tools" {...controls}>
        {compact && (hasModel || source) && <button className="btn-icon" onClick={toggleDetails}
          aria-label={tr("task.details")} title={tr("task.details")} aria-expanded={detailsOpen} aria-controls={detailsId}><Icon name="info" /></button>}
        <fieldset disabled={controlsDisabled} className="task-tools">
          {taskColumn(task) === "done" && <button className="btn-icon" onClick={handleArchive} disabled={!!archiveReason}
            aria-label={tr("archive.action")} title={tr(archiveReason || "archive.action")}>
            {archiving ? <span className="spinner" /> : <Icon name="archive" />}
          </button>}
          <button onClick={() => onEditTask(task.id)} className="btn-icon" aria-label={tr("task.edit")} title={tr("task.edit")}><Icon name="edit" /></button>
        </fieldset>
        </div>
      </div>
      <div className="task-footer">
        {compact && statusKey ? <button className={`task-compact-state${statusKey !== "task.compact_preparing" ? " attention" : ""}`} {...controls}
          onClick={event => { event.stopPropagation(); toggleDetails(); }} aria-expanded={detailsOpen} aria-controls={detailsId}
          title={tr(preparation ?? statusKey)}><span role="status">{tr(statusKey)}</span></button> : <div className="task-footer-meta">
        <span className={`task-priority priority-${task.priority}`} title={task.dispatched_to ? `${tr("task.target")}${task.dispatched_to}` : tr("priority.label")}>
          {tr(`priority.${task.priority}`)}
        </span>
        {compact && source && <button className="task-compact-source" {...controls} title={sourceLabel} aria-label={sourceLabel}
          aria-expanded={detailsOpen} aria-controls={detailsId} onClick={event => { event.stopPropagation(); toggleDetails(); }}>{sourceContent}</button>}
        </div>}
        <div className="task-actions" {...controls}>
          {compact && <button className="btn-icon" disabled={deleting || removing} onClick={onViewPrompt}
            aria-label={tr("task.view_prompt")} title={tr("task.view_prompt")}><Icon name="document" /></button>}
          <fieldset disabled={controlsDisabled} className="task-actions">
            {!compact && task.flow?.kind === "start" && task.flow.state === "failed" && <button onClick={onStart} className="btn-secondary">{tr("flow.retry_start")}</button>}
            {taskColumn(task) === "todo" && !task.binding?.operation && <button onClick={() => onDispatchTask(task.id)} className="btn-primary">{tr("dispatch.action")}<Icon name="arrow" size={13} /></button>}
            {compact && <button onClick={onComplete} className="btn-success" aria-label={tr("task.complete")} title={tr("task.complete")}>{tr("task.complete_short")}</button>}
          </fieldset>
          <button disabled={deleting || archiving || removing} onClick={handleDelete} className="btn-icon btn-danger" aria-label={tr("task.delete")} title={tr("task.delete")}><Icon name="trash" /></button>
        </div>
      </div>
      <div id={detailsId} hidden={compact && !(detailsOpen && (hasModel || source || statusKey))} className={compact ? "task-compact-details" : undefined} {...(compact ? controls : {})}>
      {compact && source && <p className="task-notice">{sourceContent}</p>}
      {compact && <p className="task-notice">{tr("priority.label")}: {tr(`priority.${task.priority}`)}</p>}
      {(task.codex_settings?.model || task.codex_settings?.effort) && <p className="task-model" title={tr("model.startup")}>{task.codex_settings.model ?? tr("model.default")} · {task.codex_settings.effort ?? tr("model.default")}</p>}
      {(preparation || busy) && <p role="status" className="task-notice">{tr(preparation ?? "flow.busy")}</p>}
      {task.flow?.error && <p className="error task-notice">{errorText(task.flow.error)}</p>}
      {(flowUnresolved(task) || canResetTask(task) || taskColumn(task) === "in_progress") && <div className="task-recovery" {...controls}>
        {!compact && <button className="btn-secondary" disabled={deleting || removing} onClick={onViewPrompt}>{tr("task.view_prompt")}</button>}
        {compact && task.flow?.kind === "start" && task.flow.state === "failed" && <button disabled={controlsDisabled} onClick={onStart} className="btn-secondary">{tr("flow.retry_start")}</button>}
        {canResetTask(task) && <button className="btn-secondary" disabled={disabled} onClick={onReset}>{tr("reset.action")}</button>}
      </div>}
      {taskColumn(task) === "done" && archiveReason && <p className="task-notice">{tr(archiveReason)}</p>}
      {archiveError && <p role="alert" className="error task-notice">{archiveError}</p>}
      {(task.flow?.state === "unknown" || task.cleanup_required || task.binding?.operation) &&
        <fieldset disabled={disabled || flowPending(task)} className="task-recovery" {...controls}>
          {task.flow?.state === "unknown" && <button className="btn-secondary" onClick={onFlowRecover}>{tr("flow.recover")}</button>}
          {task.cleanup_required && <button className="btn-secondary" onClick={onComplete}>{tr("flow.retry_cleanup")}</button>}
          {task.binding?.state === "unknown" && <button className="btn-secondary" onClick={onRecover}>{tr("dispatch.verify")}</button>}
          {task.binding?.state === "pending" && <button className="btn-ghost" onClick={onChanged}>{tr("dispatch.pending")}</button>}
        </fieldset>}
      </div>
    </div>
  );
}
