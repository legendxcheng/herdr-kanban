import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Project, Task, TaskPriority } from "./types";
import { PromptEditor } from "./PromptEditor";
import { PrioritySelect } from "./PrioritySelect";
import * as api from "./api";
import { errorText, tr } from "./i18n";
import { useImageDraft } from "./useImageDraft";

const emptyPrompt = JSON.stringify({ type: "doc", content: [{ type: "paragraph" }] });

export function TaskFormDialog({ project, task, onClose, onSaved }: {
  project: Project; task?: Task; onClose: () => void; onSaved: () => void;
}) {
  const [name, setName] = useState(task?.name ?? "");
  const [promptJson, setPromptJson] = useState(task?.prompt_json ?? emptyPrompt);
  const [priority, setPriority] = useState<TaskPriority>(task?.priority ?? "normal");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [imageBusy, setImageBusy] = useState(false);
  const [imageError, setImageError] = useState("");
  const draft = useImageDraft(project.id, task?.id);
  const submitting = useRef(false);
  const titleInput = useRef<HTMLInputElement>(null);
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  const id = useId();

  useEffect(() => {
    const root = document.getElementById("root");
    const wasInert = root?.inert ?? false;
    if (root) root.inert = true;
    titleInput.current?.focus();
    return () => {
      if (root) root.inert = wasInert;
      if (opener?.isConnected) opener.focus();
    };
  }, [opener]);

  useEffect(() => {
    if (error && !saving) titleInput.current?.focus();
  }, [error, saving]);

  function close() { if (!submitting.current) onClose(); }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!name.trim() || submitting.current || imageBusy) return;
    submitting.current = true; setSaving(true); setError("");
    try {
      const ready = draft.existingId();
      const draftId = ready ? await ready : undefined;
      if (task) await api.updateTask(task.id, name.trim(), promptJson, priority, draftId);
      else await api.createTask(project.id, name.trim(), priority, promptJson, draftId);
    } catch (reason) {
      setError(errorText(reason));
      return;
    } finally {
      submitting.current = false; setSaving(false);
    }
    onClose();
    onSaved();
  }

  function keyDown(event: React.KeyboardEvent<HTMLFormElement>) {
    if (event.defaultPrevented) return;
    if (event.nativeEvent.isComposing && event.key === "Enter") { event.preventDefault(); return; }
    if (event.key === "Escape") {
      event.preventDefault(); event.stopPropagation(); close();
    } else if (event.key === "Tab") {
      const fields = [...event.currentTarget.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), [contenteditable="true"]',
      )];
      const first = fields[0]; const last = fields[fields.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
  }

  return createPortal(<div className="dialog-overlay" onMouseDown={(event) => {
    if (event.target === event.currentTarget) close();
  }}>
    <form className="dialog task-form-dialog" role="dialog" aria-modal="true" aria-labelledby={`${id}-heading`}
      aria-busy={saving} onSubmit={submit} onKeyDown={keyDown}>
      <div className="task-form-header">
        <h3 id={`${id}-heading`}>{tr(task ? "task.edit_dialog" : "task.create")}</h3>
        <button type="button" className="btn-icon" disabled={saving} onClick={close} aria-label={tr("common.close")}>✕</button>
      </div>
      <div className="task-form-body">
        <div>
          <label className="field-label" htmlFor={`${id}-title`}>{tr("task.title")}</label>
          <input ref={titleInput} id={`${id}-title`} className="field-input" value={name} disabled={saving}
            placeholder={tr("task.title_placeholder")} onChange={(event) => setName(event.target.value)} />
        </div>
        <div>
          <label id={`${id}-prompt`} className="field-label">{tr("task.prompt")}</label>
          <PromptEditor content={promptJson} rootPath={project.root_path} onChange={setPromptJson}
            getDraftId={draft.getId} onImageBusy={busy => { setImageBusy(busy); if (busy) setImageError(""); }} onImageError={reason => setImageError(errorText(reason))}
            labelledBy={`${id}-prompt`} disabled={saving} />
          {imageError && <p role="alert" className="error">{imageError}</p>}
        </div>
        <div>
          <label className="field-label" htmlFor={`${id}-priority`}>{tr("priority.label")}</label>
          <PrioritySelect id={`${id}-priority`} label={tr("priority.label")} value={priority} onChange={setPriority} disabled={saving} />
        </div>
        {error && <p role="alert" className="error">{error}</p>}
      </div>
      <div className="task-form-footer">
        <button type="button" className="btn-secondary" disabled={saving} onClick={close}>{tr("common.cancel")}</button>
        <button type="submit" className="btn-primary" disabled={saving || imageBusy || !name.trim()}>
          {saving && <span className="spinner" />}{tr(task ? "common.save" : "task.add")}
        </button>
      </div>
    </form>
  </div>, document.body);
}
