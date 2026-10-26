import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import type { Machine, Project, TaskStatus, TaskSummary } from "./types";
import { Icon, type IconName } from "./Icon";
import { tr, useLocale, useLocalizedMessage } from "./i18n";
import { machineLabel } from "./WorkspaceAddDialog";
import "./taskSummary.css";

const columns: { status: TaskStatus; icon: IconName }[] = [
  { status: "todo", icon: "circle" }, { status: "in_progress", icon: "progress" }, { status: "done", icon: "check" },
];

export function TaskSummaryBoard({ projects, machines, onSelect }: {
  projects: Project[]; machines: Machine[]; onSelect: (projectId: string, taskId: string) => void;
}) {
  useLocale();
  const scope = JSON.stringify(projects.map(p => p.id).sort());
  const [snapshot, setSnapshot] = useState<{ scope: string; tasks: TaskSummary[] } | null>(null);
  const [error, setError] = useLocalizedMessage();
  const [refreshing, setRefreshing] = useState(false);
  const refresh = useRef(() => {});

  useEffect(() => {
    const ids: string[] = JSON.parse(scope);
    let active = true, running = false, queued = false;
    const stops: (() => void)[] = [];
    setError("");
    async function reload(queue = false) {
      if (!active || document.visibilityState === "hidden") return;
      if (running) { queued ||= queue; return; }
      running = true; setRefreshing(true);
      try {
        const tasks = ids.length ? await api.taskSummary(ids) : [];
        if (active) { setSnapshot({ scope, tasks }); setError(""); }
      } catch (reason) {
        if (active) setError(String(reason));
      } finally {
        running = false;
        if (active) {
          setRefreshing(false);
          if (queued) { queued = false; void reload(); }
        }
      }
    }
    const requestRefresh = () => { void reload(true); };
    refresh.current = requestRefresh;
    void reload();
    for (const subscribe of [api.onStartChanged, api.onTitleChanged]) {
      void subscribe(event => { if (ids.includes(event.project_id)) requestRefresh(); })
        .then(stop => { if (active) { stops.push(stop); requestRefresh(); } else stop(); })
        .catch(() => { /* Polling/focus still reconcile missed notifications. */ });
    }
    const timer = window.setInterval(() => void reload(), 2000);
    window.addEventListener("focus", requestRefresh);
    document.addEventListener("visibilitychange", requestRefresh);
    return () => {
      active = false; stops.forEach(stop => stop()); window.clearInterval(timer);
      window.removeEventListener("focus", requestRefresh);
      document.removeEventListener("visibilitychange", requestRefresh);
    };
  }, [scope]);

  const data = snapshot?.scope === scope ? snapshot.tasks : null;
  const byProject = new Map(projects.map(p => [p.id, p]));
  // Scope changes immediately hide old rows, even before the next effect/read completes.
  const tasks = data?.filter(task => byProject.has(task.project_id)) ?? [];

  return <section className="summary-board board-main" aria-label={tr("summary.title")}>
    <header className="summary-header">
      <div><h1><Icon name="board" />{tr("summary.title")}</h1><p>{tr("summary.scope")}</p></div>
      <button className="btn-secondary" disabled={refreshing} onClick={() => refresh.current()}>{tr(refreshing ? "overview.refreshing" : "overview.refresh")}</button>
    </header>
    <div className="summary-content board-content">
      <div className="summary-meta"><span>{tr("summary.count", { workspaces: projects.length, tasks: tasks.length })}</span><span>{tr("summary.order")}</span></div>
      {error && <p className="error summary-error" role="alert">{tr(data ? "summary.stale" : "summary.failed")} {error} <button className="btn-secondary" disabled={refreshing} onClick={() => refresh.current()}>{tr("overview.retry")}</button></p>}
      {data === null && !error ? <div className="board-loading" role="status"><span className="spinner" />{tr("common.loading")}</div> : data !== null && <>
        {!projects.length && <p className="summary-empty" role="status">{tr("summary.empty_workspaces")}</p>}
        <div className="kanban-columns">
          {columns.map(({ status, icon }) => {
            const rows = tasks.filter(task => task.status === status);
            return <section key={status} className={`kanban-col col-${status}`} aria-label={tr(`board.${status}`)}>
              <div className="col-header"><h2 className="col-title"><Icon name={icon} />{tr(`board.${status}`)}</h2><span className="col-count">{rows.length}</span></div>
              {!rows.length ? <div className="col-empty"><Icon name={icon} size={22} /><span>{tr(`board.${status}_empty`)}</span></div> : <div className="col-cards">
                {rows.map(task => {
                  const project = byProject.get(task.project_id)!;
                  const machineId = project.machine_id ?? "local";
                  const machineName = machineId === "local" ? "local" : machines.find(m => m.id === machineId)?.name ?? machineId;
                  const machine = machineLabel(machineId, machineName);
                  return <button key={task.id} className={`task-card summary-card status-${status}`} onClick={() => onSelect(task.project_id, task.id)}
                    aria-label={tr("summary.open_task", { name: task.name, workspace: project.name, machine })}>
                    <span className="summary-card-heading"><span className="task-name">{task.name}</span><span className={`task-priority priority-${task.priority}`}>{tr(`priority.${task.priority}`)}</span></span>
                    <span className="summary-card-source"><span className="machine-source" title={machine}>{machineName}</span><span className="summary-workspace" title={project.name}>{project.name}</span></span>
                  </button>;
                })}
              </div>}
            </section>;
          })}
        </div>
      </>}
    </div>
  </section>;
}
