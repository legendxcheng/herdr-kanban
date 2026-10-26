import { useEffect, useRef, useState } from "react";
import type { Project, ProjectGitStatus } from "./types";
import * as api from "./api";

export function useProjectGitStatus(projects: Project[], refresh: number) {
  const [statuses, setStatuses] = useState<Record<string, ProjectGitStatus>>({});
  const running = useRef(new Map<string, Promise<ProjectGitStatus[] | null>>());
  // Renames and reordering do not change the directories being observed.
  const key = JSON.stringify(projects.map(p => [p.id, p.root_path, p.machine_id ?? "local"]).sort(([a], [b]) => a.localeCompare(b)));
  useEffect(() => {
    let active = true;
    const groups = new Map<string, [string,string][]>();
    for (const [id,root,machine] of JSON.parse(key) as [string,string,string][]) groups.set(machine,[...(groups.get(machine) ?? []),[id,root]]);
    for (const [machine,requested] of groups) {
    const unavailable = () => requested.map(([project_id, root_path]) => ({ project_id, root_path, state: "unavailable" as const, head: null }));
    // Coalesce focus/refresh bursts behind the in-flight request. Obsolete queued
    // effects skip their query, and their responses cannot replace the latest paths.
    const request = (running.current.get(machine) ?? Promise.resolve(null)).catch(() => null)
      .then(() => active ? api.projectGitStatuses(requested.map(([id]) => id)) : null);
    running.current.set(machine, request);
    void request.then(result => {
      if (active && result) setStatuses(previous => ({ ...previous, ...Object.fromEntries(result.map(status => [status.project_id, status])) }));
    }, () => {
      if (active) setStatuses(previous => ({ ...previous, ...Object.fromEntries(unavailable().map(status => [status.project_id, status])) }));
    }).then(() => {
      if (running.current.get(machine) === request) running.current.delete(machine);
    });
    }
    return () => { active = false; };
  }, [key, refresh]);
  return Object.fromEntries(projects.flatMap(p => {
    const status = statuses[p.id];
    return status?.root_path === p.root_path ? [[p.id, status]] : [];
  })) as Record<string, ProjectGitStatus>;
}
