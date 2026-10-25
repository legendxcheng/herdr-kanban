import { useRef, useState } from "react";
import type { DragEvent } from "react";
import type { Project, ProjectGitStatus, WorkspaceOperation } from "./types";
import * as api from "./api";
import { ProjectFormDialog } from "./ProjectFormDialog";
import { Icon } from "./Icon";
import { WorkspaceRecovery } from "./WorkspaceRecovery";
import { tr, errorText } from "./i18n";
import { notify } from "./Toast";

interface ProjectSidebarProps {
  inert?: boolean;
  projects: Project[];
  gitStatuses?: Record<string, ProjectGitStatus>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onChanged: () => void;
  onOverview?: () => void;
  operations?: WorkspaceOperation[];
  syncing?: boolean;
  syncError?: string;
  session?: string | null;
  reordering?: boolean;
  onMove?: (projectId: string, beforeProjectId: string | null) => Promise<void>;
}

export function ProjectSidebar({ inert = false, projects, gitStatuses = {}, selectedId, onSelect, onChanged, onOverview, operations = [], syncing = false, syncError = "", session = null, reordering = false, onMove }: ProjectSidebarProps) {
  const [showNew, setShowNew] = useState(false);
  const [editTarget, setEditTarget] = useState<Project | null>(null);
  const dragging = useRef<string | null>(null);
  const suppressClick = useRef(false);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [insertion, setInsertion] = useState<{ id: string; after: boolean } | null>(null);
  const movable = (p: Project) => !!onMove && !reordering && !!p.workspace_id && p.sync_status === "ready" && p.herdr_session === session;

  function clearDrag() { dragging.current = null; setDraggingId(null); setInsertion(null); }
  function dropPosition(e: DragEvent, p: Project) {
    if (!dragging.current || dragging.current === p.id || !movable(p) || !projects.some(p => p.id === dragging.current && movable(p))) return null;
    const rect = e.currentTarget.getBoundingClientRect();
    return { id: p.id, after: e.clientY >= rect.top + rect.height / 2 };
  }
  function handleDrop(e: DragEvent, p: Project) {
    const position = dropPosition(e, p);
    if (!position) { clearDrag(); return; }
    e.preventDefault(); e.stopPropagation();
    const source = dragging.current!;
    const current = projects.filter(movable).map(p => p.id);
    const reordered = current.filter(id => id !== source);
    const index = reordered.indexOf(p.id) + (position.after ? 1 : 0);
    reordered.splice(index, 0, source);
    clearDrag();
    if (reordered.every((id, i) => id === current[i])) return;
    void onMove!(source, reordered[index + 1] ?? null);
  }

  async function handleDelete(id: string) {
    if (!confirm(tr("workspace.remove_confirm"))) return;
    try { await api.deleteProject(id); onChanged(); } catch (e) { notify(errorText(e),"error"); }
  }

  return (
    <aside className="sidebar" aria-label="项目导航" inert={inert}>
      <div className="sidebar-brand">
        <span className="brand-mark"><img src="/app-icon.svg" width={32} height={32} alt="" /></span>
        <span>herdr<span className="brand-subtitle"> / kanban</span></span>
      </div>
      <div className="sidebar-intro">把想法变成下一步。</div>
      <div className="sidebar-header">
        <h2>项目 <span className="sidebar-count">{projects.length}</span></h2>
        {onOverview && <button onClick={onOverview} className="btn-icon" aria-label={tr("overview.title")} title={tr("overview.title")}><Icon name="board" /></button>}
        <button onClick={() => setShowNew(true)} className="btn-icon" aria-label="新建项目" title="新建项目"><Icon name="plus" /></button>
      </div>
      <div className="project-list" aria-busy={reordering} onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setInsertion(null); }}>
        {projects.map((p) => {
          const git = gitStatuses[p.id];
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
            <button className="project-row" onPointerDown={() => { suppressClick.current = false; }} onKeyDown={() => { suppressClick.current = false; }} onClick={() => { if (!suppressClick.current) onSelect(p.id); }} aria-current={p.id === selectedId ? "page" : undefined} title={movable(p) ? `${p.name} · ${tr("workspace.drag_sort")}` : p.name}>
              <Icon name="folder" />
              <span className="project-info">
                <span className="project-name">{p.name}</span>
                <span className="project-ws" title={`${label}\n${p.root_path}`}><Icon name="branch" size={13} /><span>{label}</span></span>
              </span>
            </button>
            <div className="project-actions">
              <button onClick={() => setEditTarget(p)} className="btn-icon" title="编辑项目" aria-label={`编辑项目 ${p.name}`}><Icon name="edit" size={14} /></button>
              <button onClick={() => handleDelete(p.id)} className="btn-icon" title="删除项目" aria-label={`删除项目 ${p.name}`}><Icon name="trash" size={14} /></button>
            </div>
          </div>
          );
        })}
      </div>
      <div className="workspace-sync"><button className="btn-secondary" disabled={syncing} onClick={onChanged}>{tr(syncing ? "workspace.syncing" : "workspace.refresh")}</button>
        {reordering && <p role="status">{tr("workspace.reordering")}</p>}
        {syncError && <p className="error" role="alert">{syncError}</p>}
      </div>
      {operations.length > 0 && <div className="workspace-pending"><strong>{tr("workspace.pending_title")}</strong>{operations.map((op) => <WorkspaceRecovery key={op.id} operation={op} onChanged={onChanged} />)}</div>}
      <button className="sidebar-create btn-secondary" onClick={() => setShowNew(true)}><Icon name="plus" />新建项目</button>
      <div className="sidebar-footer"><Icon name="terminal" /><div><strong>你的本地工作空间</strong><span>组织任务 · 协作开发</span></div></div>

      {showNew && (
        <ProjectFormDialog
          key="new-workspace"
          onClose={() => setShowNew(false)}
          onSaved={() => { setShowNew(false); onChanged(); }}
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
