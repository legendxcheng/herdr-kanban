import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import * as api from "./api";
import type { Project } from "./types";
import { tr, getLocale, useLocalizedMessage } from "./i18n";

/** Calendar boundaries, not now minus 24h; preserves local DST transitions. */
export function localDay(now = new Date()) {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const end = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  return { start: start.toISOString(), end: end.toISOString(), label: start.toLocaleDateString(document.documentElement.lang || undefined) };
}

export function TaskOverviewDialog({ projects, onClose, onSelect }: {
  projects: Project[]; onClose: () => void; onSelect: (id: string) => void;
}) {
  const scope = JSON.stringify(projects.map(project => project.id));
  const [snapshot, setSnapshot] = useState<{ scope: string; day: ReturnType<typeof localDay>; rows: api.ProjectCounts[] } | null>(null);
  const [error, setError] = useLocalizedMessage();
  const [refreshing, setRefreshing] = useState(false);
  const refresh = useRef<() => void>(() => {});
  const closeButton = useRef<HTMLButtonElement>(null);
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  const id = useId();

  useEffect(() => {
    const root = document.getElementById("root");
    const wasInert = root?.inert ?? false;
    if (root) root.inert = true;
    closeButton.current?.focus();
    return () => {
      if (root) root.inert = wasInert;
      if (opener?.isConnected) opener.focus();
    };
  }, [opener]);

  useEffect(() => {
    let active = true, running = false, queued = false;
    let unlisten: (() => void) | undefined;
    let midnight: number | undefined;
    const ids: string[] = JSON.parse(scope);
    setSnapshot(null); setError("");
    async function reload() {
      if (!active || document.visibilityState === "hidden") return;
      if (running) { queued = true; return; }
      running = true; setRefreshing(true);
      const day = localDay();
      try {
        const rows = await api.taskOverview(ids, day.start, day.end);
        if (!active) return;
        if (localDay().start !== day.start) { setSnapshot(null); queued = true; return; }
        setSnapshot({ scope, day, rows }); setError("");
      } catch (reason) {
        if (active) { setSnapshot(null); setError(String(reason)); }
      } finally {
        running = false;
        if (active) {
          setRefreshing(false);
          if (queued) { queued = false; void reload(); }
        }
      }
    }
    function scheduleMidnight() {
      midnight = window.setTimeout(() => {
        setSnapshot(null); void reload(); scheduleMidnight();
      }, Math.max(1, new Date(localDay().end).getTime() - Date.now()));
    }
    refresh.current = () => { void reload(); };
    void reload(); scheduleMidnight();
    void api.onStartChanged(event => { if (ids.includes(event.project_id)) void reload(); })
      .then(stop => { if (active) { unlisten = stop; void reload(); } else stop(); })
      .catch(() => { /* Polling/focus reconcile if event transport is unavailable. */ });
    const timer = window.setInterval(() => void reload(), 2000);
    window.addEventListener("focus", reload);
    document.addEventListener("visibilitychange", reload);
    return () => {
      active = false; unlisten?.(); window.clearInterval(timer); window.clearTimeout(midnight);
      window.removeEventListener("focus", reload); document.removeEventListener("visibilitychange", reload);
    };
  }, [scope]);

  const data = snapshot?.scope === scope && snapshot.day.start === localDay().start ? snapshot : null;
  const totals = data?.rows.reduce((sum, row) => ({ todo: sum.todo + row.todo, in_progress: sum.in_progress + row.in_progress, today_done: sum.today_done + row.today_done }), { todo: 0, in_progress: 0, today_done: 0 });
  const byId = new Map(data?.rows.map(row => [row.project_id, row]));

  return createPortal(<div className="dialog-overlay" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <section className="dialog overview-dialog" role="dialog" aria-modal="true" aria-labelledby={`${id}-title`} aria-describedby={`${id}-scope`}
      onKeyDown={event => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); }
        if (event.key === "Tab") {
          const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
          const first = buttons[0], last = buttons[buttons.length - 1];
          if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
          else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
        }
      }}>
      <header className="overview-header"><div><h3 id={`${id}-title`}>{tr("overview.title")}</h3><p id={`${id}-scope`}>{tr("overview.scope")}</p></div>
        <button ref={closeButton} className="btn-icon" aria-label={tr("common.close")} onClick={onClose}>✕</button>
      </header>
      <div className="overview-body">
        {error ? <p className="error" role="alert">{tr("overview.failed")} {error}</p> : !data ? <p role="status">{tr("overview.loading")}</p> : <>
          <div className="overview-totals">{(["todo", "in_progress", "today_done"] as const).map(key => <div key={key}><span>{tr(`overview.${key}`)}</span><strong>{totals![key]}</strong></div>)}</div>
          {projects.length === 0 ? <p className="overview-empty">{tr("overview.empty")}</p> : <div className="overview-table-scroll"><table className="overview-table">
            <thead><tr><th scope="col">{tr("common.workspace")}</th>{["todo", "in_progress", "done", "today_done"].map(key => <th scope="col" key={key}>{tr(`overview.${key}`)}</th>)}</tr></thead>
            <tbody>{projects.map(project => {
              const row = byId.get(project.id);
              return <tr key={project.id}><th scope="row"><button title={project.name} onClick={() => onSelect(project.id)}>{project.name}</button></th>
                {(["todo", "in_progress", "done", "today_done"] as const).map(key => <td key={key}>{row?.[key] ?? "—"}</td>)}
              </tr>;
            })}</tbody>
          </table></div>}
          <p className="overview-note">{tr("overview.rules")}</p>
        </>}
      </div>
      <footer className="overview-footer"><span>{new Date(data?.day.start ?? localDay().start).toLocaleDateString(getLocale())} · {tr("overview.local_day")}</span>
        <button className="btn-secondary" disabled={refreshing} onClick={() => refresh.current()}>{tr(error ? "overview.retry" : refreshing ? "overview.refreshing" : "overview.refresh")}</button>
      </footer>
    </section>
  </div>, document.body);
}
