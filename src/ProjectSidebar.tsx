import { useRef, useState } from "react";
import type { DragEvent } from "react";
import type { Machine, Project, ProjectGitStatus, WorkspaceOperation } from "./types";
import * as api from "./api";
import { ProjectFormDialog } from "./ProjectFormDialog";
import { Icon } from "./Icon";
import { WorkspaceRecovery } from "./WorkspaceRecovery";
import { tr, useLocale, errorText } from "./i18n";
import { notify } from "./Toast";
import { LanguageSelect } from "./LanguageSelect";
import { MachineDialog } from "./MachineDialog";
import { WorkspaceAddDialog, machineLabel } from "./WorkspaceAddDialog";

interface ProjectSidebarProps {
  machines?: Machine[];
  machineErrors?: Record<string,string>;
  machineBusy?: Record<string,boolean>;
  inert?: boolean;
  projects: Project[];
  gitStatuses?: Record<string, ProjectGitStatus>;
  inProgressCounts?: Record<string, number>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onHide?: (id: string) => Promise<void>;
  recoveryProjects?: Project[];
  onRecovery?: (id: string) => void;
  onChanged: () => void;
  onOverview?: () => void;
  summarySelected?: boolean;
  onSummary?: () => void;
  operations?: WorkspaceOperation[];
  syncing?: boolean;
  syncError?: string;
  session?: string | null;
  reordering?: boolean;
  onMove?: (projectId: string, beforeProjectId: string | null) => Promise<void>;
}

export function ProjectSidebar({ machines = [], machineErrors = {}, machineBusy = {}, inert = false, projects, gitStatuses = {}, inProgressCounts = {}, selectedId, onSelect, onHide, recoveryProjects = [], onRecovery, onChanged, onOverview, summarySelected = false, onSummary, operations = [], syncing = false, syncError = "", session = null, reordering = false, onMove }: ProjectSidebarProps) {
  useLocale();
  const [showNew, setShowNew] = useState(false);
  const [manageMachines, setManageMachines] = useState(false);
  const [editTarget, setEditTarget] = useState<Project | null>(null);
  const [hiding, setHiding] = useState<string | null>(null);
  const dragging = useRef<string | null>(null);
  const suppressClick = useRef(false);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [insertion, setInsertion] = useState<{ id: string; after: boolean } | null>(null);
  const machineOf = (p: Project) => p.machine_id ?? "local";
  const groups = [{ id: "local", name: tr("machine.local"), disabled: false }, ...machines];
  const movable = (p: Project) => !!onMove && !reordering && !!p.workspace_id && p.sync_status === "ready" && (machineOf(p) === "local" ? p.herdr_session === session : !machines.find(m => m.id === p.machine_id)?.disabled && !machineErrors[machineOf(p)] && !machineBusy[machineOf(p)]);

  function clearDrag() { dragging.current = null; setDraggingId(null); setInsertion(null); }
  function dropPosition(e: DragEvent, p: Project) {
    if (!dragging.current || dragging.current === p.id || !movable(p) || !projects.some(source => source.id === dragging.current && machineOf(source) === machineOf(p) && movable(source))) return null;
    const rect = e.currentTarget.getBoundingClientRect();
    return { id: p.id, after: e.clientY >= rect.top + rect.height / 2 };
  }
  function handleDrop(e: DragEvent, p: Project) {
    const position = dropPosition(e, p);
    if (!position) { clearDrag(); return; }
    e.preventDefault(); e.stopPropagation();
    const source = dragging.current!;
    const current = projects.filter(item => machineOf(item) === machineOf(p) && movable(item)).map(p => p.id);
    const reordered = current.filter(id => id !== source);
    const index = reordered.indexOf(p.id) + (position.after ? 1 : 0);
    reordered.splice(index, 0, source);
    clearDrag();
    if (reordered.every((id, i) => id === current[i])) return;
    void onMove!(source, reordered[index + 1] ?? null);
  }

  async function handleDelete(id: string) {
    if (!confirm(tr("workspace.remove_confirm"))) return;
    try { await api.deleteProject(id); onChanged(); } catch (e) { notify({ error: String(e) },"error"); }
  }
  async function handleHide(id: string) {
    if (hiding || !onHide) return;
    setHiding(id);
    try { await onHide(id); } catch(e) { notify({ error: String(e) }, "error"); }
    finally { setHiding(null); }
  }

  return (
    <aside className="sidebar" aria-label={tr("sidebar.navigation")} inert={inert}>
      <div className="sidebar-brand">
        <span className="brand-mark"><img src="/app-icon.svg" width={32} height={32} alt="" /></span>
        <span>herdr<span className="brand-subtitle"> / kanban</span></span>
      </div>
      <div className="sidebar-intro">{tr("sidebar.intro")}</div>
      {onSummary && <button className="sidebar-summary" aria-pressed={summarySelected} onClick={onSummary}><Icon name="board" />{tr("summary.title")}</button>}
      <div className="sidebar-header">
        <h2>{tr("sidebar.projects")} <span className="sidebar-count">{projects.length}</span></h2>
        {onOverview && <button onClick={onOverview} className="btn-icon" aria-label={tr("overview.title")} title={tr("overview.title")}><Icon name="board" /></button>}
        <button onClick={() => setShowNew(true)} className="btn-icon" aria-label={tr("workspace.add")} title={tr("workspace.add")}><Icon name="plus" /></button>
      </div>
      <div className="project-list" aria-busy={reordering} data-dragging={draggingId !== null} onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setInsertion(null); }}>
        {groups.map(group => {
          const groupProjects = projects.filter(p => machineOf(p) === group.id);
          const machineState = group.disabled ? "disabled" : machineErrors[group.id] ? "offline"
            : machineBusy[group.id] ? "syncing"
            : group.id !== "local" && machineErrors[group.id] === "" ? "online" : "unknown";
          const status = machineState === "syncing" ? tr("workspace.syncing")
            : machineState === "offline" ? `${tr("machine.offline")} · ${errorText(machineErrors[group.id])}`
            : machineState === "unknown" ? "" : tr(`machine.${machineState}`);
          const machineTitle = [machineLabel(group.id, group.name), status,
            machineState === "offline" && machineBusy[group.id] ? tr("workspace.syncing") : "",
          ].filter(Boolean).join(" · ");
          if (!groupProjects.length && !machineErrors[group.id]) return null;
          return <section key={group.id} className="machine-group" aria-label={machineLabel(group.id, group.name)}>
        {!groupProjects.length && machineErrors[group.id] && <p className="error" role="alert">{machineTitle}</p>}
        {groupProjects.map((p) => {
          const git = gitStatuses[p.id];
          const inProgress = inProgressCounts[p.id];
          const countLabel = `${tr("overview.in_progress")} ${inProgress ?? "—"}`;
          const label = p.sync_status !== "ready" ? tr(`workspace.${p.sync_status}`)
            : !git ? tr("workspace.git_loading")
            : git.state === "branch" ? git.head
            : git.state === "detached" ? `${tr("workspace.git_detached")} · ${git.head ?? ""}`
            : tr(`workspace.git_${git.state}`);
          return (
          <div
            key={p.id}
            className={"project-item" + (p.id === selectedId ? " selected" : "") + (p.id === draggingId ? " project-dragging" : "") + (insertion?.id === p.id ? insertion.after ? " insert-after" : " insert-before" : "")}
            data-project-id={p.id}
            draggable={movable(p)}
            onDragStart={(e) => {
              if (!movable(p) || (e.target as HTMLElement).closest(".project-actions")) { e.preventDefault(); return; }
              dragging.current = p.id; suppressClick.current = true; setDraggingId(p.id);
              e.dataTransfer.effectAllowed = "move";
              e.dataTransfer.setData("application/x-kanban-workspace", p.id);
            }}
            onDragEnd={clearDrag}
            onDragOver={(e) => { const position = dropPosition(e, p); if (position) { e.preventDefault(); e.dataTransfer.dropEffect = "move"; setInsertion(position); } }}
            onDrop={(e) => handleDrop(e, p)}
          >
            <button className="project-row" onPointerDown={() => { suppressClick.current = false; }} onKeyDown={() => { suppressClick.current = false; }} onClick={() => { if (!suppressClick.current) onSelect(p.id); }} aria-current={p.id === selectedId ? "page" : undefined} aria-label={`${p.name} · ${machineTitle} · ${label} · ${countLabel}`} title={[p.name, machineTitle, p.root_path, movable(p) ? tr("workspace.drag_sort") : ""].filter(Boolean).join("\n")}>
              <span className="project-info">
                <span className="project-title">
                  <span className={`project-machine machine-${machineState}`} title={machineTitle}>
                    <span className="project-machine-name">{group.id === "local" ? "local" : group.name}</span>
                    {machineState !== "unknown" && <span className="project-machine-status" aria-hidden="true" />}
                  </span>
                  <span className="project-name">{p.name}</span>
                </span>
                <span className="project-meta">
                  <span className="project-ws" title={`${label}\n${p.root_path}`}><Icon name="branch" size={13} /><span>{label}</span></span>
                  <span className="project-in-progress" title={tr(inProgress === undefined ? "workspace.count_unavailable" : "workspace.count_help")}>{countLabel}</span>
                </span>
              </span>
            </button>
            <div className="project-actions">
              {onHide && <button onClick={() => void handleHide(p.id)} disabled={hiding !== null} className="btn-icon" title={tr("workspace.hide_help")} aria-label={`${tr("workspace.hide")} ${p.name}`}><Icon name="hide" size={14} /></button>}
              <button onClick={() => setEditTarget(p)} className="btn-icon" title={tr("project.edit")} aria-label={`${tr("project.edit")} ${p.name}`}><Icon name="edit" size={14} /></button>
              <button onClick={() => handleDelete(p.id)} className="btn-icon" title={tr("project.delete")} aria-label={`${tr("project.delete")} ${p.name}`}><Icon name="trash" size={14} /></button>
            </div>
          </div>
          );
        })}
        </section>;
        })}
      </div>
      <div className="workspace-sync"><button className="btn-secondary" disabled={syncing} onClick={onChanged}>{tr(syncing ? "workspace.syncing" : "workspace.refresh")}</button>
        {reordering && <p role="status">{tr("workspace.reordering")}</p>}
        {syncError && <p className="error" role="alert">{syncError}</p>}
      </div>
      {operations.length > 0 && <div className="workspace-pending"><strong>{tr("workspace.pending_title")}</strong>{operations.map((op) => <WorkspaceRecovery key={op.id} operation={op} onChanged={onChanged} />)}</div>}
      {recoveryProjects.length > 0 && <div className="workspace-pending"><strong>{tr("workspace.hidden_recovery")}</strong>{recoveryProjects.map(p => <button key={p.id} className="btn-ghost" onClick={() => onRecovery?.(p.id)}>{machineLabel(p.machine_id ?? "local", machines.find(m => m.id === p.machine_id)?.name)} · {p.name}</button>)}</div>}
      <button className="sidebar-create btn-secondary" onClick={() => setShowNew(true)}><Icon name="plus" />{tr("workspace.add")}</button>
      <LanguageSelect />
      <button className="btn-secondary" onClick={() => setManageMachines(true)}>{tr("machine.manage")}</button>
      {manageMachines && <MachineDialog machines={machines} onChanged={onChanged} onClose={() => setManageMachines(false)} />}
      <div className="sidebar-footer"><Icon name="terminal" /><div><strong>{tr("sidebar.local")}</strong><span>{tr("sidebar.footer")}</span></div></div>

      {showNew && (
        <WorkspaceAddDialog
          key="new-workspace"
          machines={machines}
          onClose={() => setShowNew(false)}
          onSaved={onChanged}
        />
      )}
      {editTarget && (
        <ProjectFormDialog
          existing={editTarget}
          key={editTarget.id}
          onClose={() => setEditTarget(null)}
          onSaved={() => { setEditTarget(null); onChanged(); }}
        />
      )}
    </aside>
  );
}
