import { useEffect, useId, useImperativeHandle, useRef, useState, type Ref } from "react";
import { createPortal } from "react-dom";
import type { Project, Task, TaskPriority } from "./types";
import { PromptEditor } from "./PromptEditor";
import { PrioritySelect } from "./PrioritySelect";
import * as api from "./api";
import { tr, useLocalizedMessage } from "./i18n";
import { useImageDraft } from "./useImageDraft";

const emptyPrompt = JSON.stringify({ type: "doc", content: [{ type: "paragraph" }] });

function hasTaskContent(json: string): boolean {
  function valid(node: any): boolean {
    if (node?.type === "text") return typeof node.text === "string" && !!node.text.trim();
    if (node?.type === "mention") return ["file", "skill"].includes(node.attrs?.kind) && !!node.attrs?.ref?.trim() && !!node.attrs?.label?.trim();
    if (node?.type === "promptImage") return !!node.attrs?.imageId;
    return Array.isArray(node?.content) && node.content.some(valid);
  }
  try { return valid(JSON.parse(json)); } catch { return false; }
}

export interface TaskEditorHandle { requestLeave: () => Promise<boolean>; projectId?: string; }
interface TaskFormProps {
  project: Project; task?: Task; onClose: () => void; onSaved: () => void | Promise<void>;
  unavailable?: boolean;
}

export function TaskFormDialog(props: TaskFormProps) { return <TaskForm {...props} />; }

export function TaskEditPanel(props: TaskFormProps & { task: Task; editorRef: Ref<TaskEditorHandle> }) {
  return <TaskForm {...props} panel />;
}

function TaskForm({ project, task, onClose, onSaved, panel = false, editorRef, unavailable = false }: TaskFormProps & {
  panel?: boolean; editorRef?: Ref<TaskEditorHandle>;
}) {
  const [name, setName] = useState(task?.name ?? "");
  const [promptJson, setPromptJson] = useState(task?.prompt_json ?? emptyPrompt);
  const [priority, setPriority] = useState<TaskPriority>(task?.priority ?? "normal");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useLocalizedMessage();
  const [imageBusy, setImageBusy] = useState(false);
  const [imageError, setImageError] = useLocalizedMessage();
  const draft = useImageDraft(project.id, task?.id);
  const submitting = useRef(false);
  const titleInput = useRef<HTMLInputElement>(null);
  const form = useRef<HTMLFormElement>(null);
  const initialName = useRef(task?.name ?? "");
  const [saved, setSaved] = useState({ name: task?.name ?? "", promptJson: task?.prompt_json ?? emptyPrompt, priority: task?.priority ?? "normal" });
  const dirty = name.trim() !== saved.name.trim() || promptJson !== saved.promptJson || priority !== saved.priority;
  const [leaving, setLeaving] = useState(false);
  const leaveResolver = useRef<((allow: boolean) => void) | null>(null);
  const leaveDialog = useRef<HTMLDivElement>(null);
  const canSubmit = !unavailable && (task ? !!name.trim() : hasTaskContent(promptJson));
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  const id = useId();

  useEffect(() => {
    if (panel) return;
    const root = document.getElementById("root");
    const wasInert = root?.inert ?? false;
    if (root) root.inert = true;
    titleInput.current?.focus();
    return () => {
      if (root) root.inert = wasInert;
      if (opener?.isConnected) opener.focus();
    };
  }, [opener, panel]);

  function requestLeave(): Promise<boolean> {
    if (submitting.current || imageBusy || leaveResolver.current) return Promise.resolve(false);
    if (!dirty) return Promise.resolve(true);
    setLeaving(true);
    return new Promise(resolve => { leaveResolver.current = resolve; });
  }
  useImperativeHandle(editorRef, () => ({ requestLeave, projectId: project.id }));
  useEffect(() => () => { leaveResolver.current?.(false); leaveResolver.current = null; }, []);
  useEffect(() => {
    if (!leaving) return;
    const root = document.getElementById("root");
    const wasInert = root?.inert ?? false;
    const active = document.activeElement as HTMLElement | null;
    if (root) root.inert = true;
    leaveDialog.current?.querySelector<HTMLButtonElement>("button")?.focus();
    return () => { if (root) root.inert = wasInert; if (active?.isConnected) active.focus(); };
  }, [leaving]);

  function resolveLeave(allow: boolean) {
    setLeaving(false);
    leaveResolver.current?.(allow);
    leaveResolver.current = null;
  }

  useEffect(() => {
    if (error && !saving && !leaving) (titleInput.current ?? form.current?.querySelector<HTMLElement>('[contenteditable="true"]'))?.focus();
  }, [!!error, saving, leaving]);

  async function close() {
    if (submitting.current) return;
    if (!panel || await requestLeave()) onClose();
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    await save();
  }

  async function save(): Promise<boolean> {
    if (!canSubmit || submitting.current || imageBusy) return false;
    submitting.current = true; setSaving(true); setError("");
    try {
      const ready = draft.existingId();
      const draftId = ready ? await ready : undefined;
      if (task) await api.updateTask(task.id, name.trim() === initialName.current.trim() ? undefined : name.trim(), promptJson, priority, draftId);
      else await api.createTask(project.id, priority, promptJson, draftId);
      initialName.current = name.trim();
      setSaved({ name: name.trim(), promptJson, priority });
    } catch (reason) {
      setError(String(reason));
      return false;
    } finally {
      submitting.current = false; setSaving(false);
    }
    if (!panel) onClose();
    await onSaved();
    return true;
  }

  function keyDown(event: React.KeyboardEvent<HTMLFormElement>) {
    if (event.defaultPrevented) return;
    if (event.nativeEvent.isComposing && event.key === "Enter") { event.preventDefault(); return; }
    if (event.key === "Escape") {
      event.preventDefault(); event.stopPropagation(); close();
    } else if (!panel && event.key === "Tab") {
      const fields = [...event.currentTarget.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), [contenteditable="true"]',
      )];
      const first = fields[0]; const last = fields[fields.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
  }

  const content = <form ref={form} className={panel ? "task-edit-form" : "dialog task-form-dialog"} role={panel ? undefined : "dialog"} aria-modal={panel ? undefined : true} aria-labelledby={`${id}-heading`}
      aria-busy={saving} onSubmit={submit} onKeyDown={keyDown}>
      <div className="task-form-header">
        <h3 id={`${id}-heading`}>{tr(task ? "task.edit_dialog" : "task.create")}</h3>
        <button type="button" className="btn-icon" disabled={saving} onClick={close} aria-label={tr("common.close")}>✕</button>
      </div>
      <div className="task-form-body">
        {task && <div>
          <label className="field-label" htmlFor={`${id}-title`}>{tr("task.title")}</label>
          <input ref={titleInput} id={`${id}-title`} className="field-input" value={name} disabled={saving}
            placeholder={tr("task.title_placeholder")} onChange={(event) => setName(event.target.value)} />
        </div>}
        <div>
          <label id={`${id}-prompt`} className="field-label">{tr("task.prompt")}</label>
          <PromptEditor content={promptJson} rootPath={project.root_path} projectId={project.machine_id && project.machine_id !== "local" ? project.id : undefined} onChange={setPromptJson}
            getDraftId={draft.getId} onImageBusy={busy => { setImageBusy(busy); if (busy) setImageError(""); }} onImageError={reason => setImageError(String(reason))}
            labelledBy={`${id}-prompt`} disabled={saving} autoFocus={!task} />
          {imageError && <p role="alert" className="error">{imageError}</p>}
        </div>
        <div>
          <label className="field-label" htmlFor={`${id}-priority`}>{tr("priority.label")}</label>
          <PrioritySelect id={`${id}-priority`} label={tr("priority.label")} value={priority} onChange={setPriority} disabled={saving} />
        </div>
        {unavailable && <p role="alert" className="error">{tr("summary.project_unavailable")}</p>}
        {error && <p role="alert" className="error">{error}</p>}
      </div>
      <div className="task-form-footer">
        {panel && <span role="status" className="task-save-state">{tr(dirty ? "task.unsaved" : "task.saved")}</span>}
        <button type="button" className="btn-secondary" disabled={saving || (panel && imageBusy)} onClick={close}>{tr(panel ? "common.close" : "common.cancel")}</button>
        <button type="submit" className="btn-primary" disabled={saving || imageBusy || !canSubmit}>
          {saving && <span className="spinner" />}{tr(task ? "common.save" : "task.add")}
        </button>
      </div>
    </form>;

  if (!panel) return createPortal(<div className="dialog-overlay" onMouseDown={(event) => {
    if (event.target === event.currentTarget) void close();
  }}>{content}</div>, document.body);

  return <aside className="task-edit-panel" aria-label={tr("task.edit_dialog")}>
    {content}
    {leaving && createPortal(<div className="dialog-overlay">
      <div ref={leaveDialog} className="dialog" role="dialog" aria-modal="true" aria-labelledby={`${id}-leave`}
        aria-busy={saving} onKeyDown={event => {
          if (event.key === "Escape" && !saving) { event.preventDefault(); event.stopPropagation(); resolveLeave(false); }
          if (event.key === "Tab") {
            const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
            if (event.shiftKey && document.activeElement === buttons[0]) { event.preventDefault(); buttons[buttons.length - 1]?.focus(); }
            else if (!event.shiftKey && document.activeElement === buttons[buttons.length - 1]) { event.preventDefault(); buttons[0]?.focus(); }
          }
        }}>
        <h3 id={`${id}-leave`}>{tr("task.unsaved")}</h3>
        <p>{tr("task.unsaved_message")}</p>
        {error && <p role="alert" className="error">{error}</p>}
        <div className="task-leave-actions">
          <button className="btn-secondary" disabled={saving} onClick={() => resolveLeave(false)}>{tr("task.keep_editing")}</button>
          <button className="btn-secondary" disabled={saving} onClick={() => resolveLeave(true)}>{tr("task.discard")}</button>
          <button className="btn-primary" disabled={saving || imageBusy || !canSubmit} onClick={async () => { if (await save()) resolveLeave(true); }}>{tr("task.save_continue")}</button>
        </div>
      </div>
    </div>, document.body)}
  </aside>;
}
