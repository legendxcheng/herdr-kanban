import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Machine, Project } from "./types";
import { TaskFormDialog } from "./TaskFormDialog";
import { machineLabel } from "./WorkspaceAddDialog";
import { tr } from "./i18n";

export function CreateTaskDialog({ projects, machines, onClose, onSaved }: {
  projects: Project[]; machines: Machine[]; onClose: () => void; onSaved: () => Promise<void>;
}) {
  const [target, setTarget] = useState<Project | null>(null);
  return target ? <TaskFormDialog project={target} unavailable={!projects.some(p => p.id === target.id)} onClose={onClose} onSaved={onSaved} />
    : <ProjectChoice projects={projects} machines={machines} onClose={onClose} onChoose={setTarget} />;
}

function ProjectChoice({ projects, machines, onClose, onChoose }: {
  projects: Project[]; machines: Machine[]; onClose: () => void; onChoose: (project: Project) => void;
}) {
  const [selected, setSelected] = useState("");
  const select = useRef<HTMLSelectElement>(null);
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  const id = useId();
  useEffect(() => {
    const root = document.getElementById("root");
    const inert = root?.inert ?? false;
    if (root) root.inert = true;
    select.current?.focus();
    return () => { if (root) root.inert = inert; if (opener?.isConnected) opener.focus(); };
  }, [opener]);
  const target = projects.find(p => p.id === selected);
  return createPortal(<div className="dialog-overlay" onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}>
    <section className="dialog" role="dialog" aria-modal="true" aria-labelledby={id} onKeyDown={e => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onClose(); }
      if (e.key === "Tab") {
        const controls = e.currentTarget.querySelectorAll<HTMLElement>("select, button:not(:disabled)");
        const first = controls[0], last = controls[controls.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
      }
    }}>
      <h3 id={id}>{tr("summary.choose_project")}</h3>
      {!projects.length && <p>{tr("summary.empty_workspaces")}</p>}
      <label>{tr("summary.choose_project")}<select ref={select} value={target?.id ?? ""} onChange={e => setSelected(e.target.value)}>
        <option value="">{tr("summary.choose_placeholder")}</option>
        {projects.map(p => <option key={p.id} value={p.id}>{machineLabel(p.machine_id ?? "local", machines.find(m => m.id === p.machine_id)?.name)} · {p.name}</option>)}
      </select></label>
      <div className="dialog-actions"><button className="btn-secondary" onClick={onClose}>{tr("common.cancel")}</button>
        <button className="btn-primary" disabled={!target} onClick={() => { if (target) onChoose(target); }}>{tr("summary.continue")}</button></div>
    </section>
  </div>, document.body);
}
