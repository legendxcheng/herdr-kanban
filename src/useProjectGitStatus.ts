import { useEffect, useRef, useState } from "react";
import type { Project, ProjectGitStatus } from "./types";
import * as api from "./api";

export function useProjectGitStatus(projects: Project[], refresh: number) {
  const [statuses, setStatuses] = useState<Record<string, ProjectGitStatus>>({});
  const running = useRef<Promise<ProjectGitStatus[] | null> | null>(null);
  // Renames and reordering do not change the directories being observed.
  const key = JSON.stringify(projects.map(p => [p.id, p.root_path]).sort(([a], [b]) => a.localeCompare(b)));
  useEffect(() => {
    let active = true;
    const requested = JSON.parse(key) as [string, string][];
    if (!requested.length) return;
    const unavailable = () => requested.map(([project_id, root_path]) => ({ project_id, root_path, state: "unavailable" as const, head: null }));
    // Coalesce focus/refresh bursts behind the in-flight request. Obsolete queued
    // effects skip their query, and their responses cannot replace the latest paths.
    const request = (running.current ?? Promise.resolve(null)).catch(() => null)
      .then(() => active ? api.projectGitStatuses(requested.map(([id]) => id)) : null);
    running.current = request;
    void request.then(result => {
      if (active && result) setStatuses(Object.fromEntries(result.map(status => [status.project_id, status])));
    }, () => {
      if (active) setStatuses(Object.fromEntries(unavailable().map(status => [status.project_id, status])));
    }).then(() => {
      if (running.current === request) running.current = null;
    });
    return () => { active = false; };
  }, [key, refresh]);
  return Object.fromEntries(projects.flatMap(p => {
    const status = statuses[p.id];
    return status?.root_path === p.root_path ? [[p.id, status]] : [];
  })) as Record<string, ProjectGitStatus>;
}
