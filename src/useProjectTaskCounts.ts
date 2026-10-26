import { useEffect, useState } from "react";
import type { Project } from "./types";
import * as api from "./api";

/** Read all visible workspaces in one local query, including remote projects. */
export function useProjectTaskCounts(projects: Project[]) {
  const scope = JSON.stringify(projects.map(project => project.id).sort());
  const [snapshot, setSnapshot] = useState<{ scope: string; counts: Record<string, number> } | null>(null);

  useEffect(() => {
    const ids: string[] = JSON.parse(scope);
    if (!ids.length) return;
    let active = true, running = false, queued = false;
    let unlisten: (() => void) | undefined;
    async function reload(queue = false) {
      if (!active || document.visibilityState === "hidden") return;
      if (running) { queued ||= queue; return; }
      running = true;
      // The existing overview IPC also accepts a local calendar-day range.
      const start = new Date(); start.setHours(0, 0, 0, 0);
      const end = new Date(start); end.setDate(end.getDate() + 1);
      try {
        const rows = await api.taskOverview(ids, start.toISOString(), end.toISOString());
        if (active && !queued) setSnapshot({ scope, counts: Object.fromEntries(rows.map(row => [row.project_id, row.in_progress])) });
      } catch {
        // Unknown is not zero, and an old count must not look current after failure.
        if (active) setSnapshot(null);
      } finally {
        running = false;
        if (active && queued) { queued = false; void reload(); }
      }
    }
    const refresh = () => { void reload(true); };
    void reload();
    void api.onStartChanged(event => { if (ids.includes(event.project_id)) refresh(); })
      .then(stop => {
        if (!active) { stop(); return; }
        unlisten = stop; refresh();
      }).catch(() => { /* Polling/focus also recover missed events. */ });
    const timer = window.setInterval(() => void reload(), 2000);
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      active = false; unlisten?.(); window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [scope]);

  return snapshot?.scope === scope ? snapshot.counts : {};
}
