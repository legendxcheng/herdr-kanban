import { useCallback, useEffect, useRef, useState } from "react";
import type { HerdrAgent, Project } from "./types";
import * as api from "./api";

// Coalesce in-flight reads, including React StrictMode's setup/cleanup/setup cycle.
const flights = new Map<string, Promise<HerdrAgent[]>>();
const isVisible = () => document.visibilityState !== "hidden";
function readAgents(projectId: string) {
  const existing = flights.get(projectId);
  if (existing) return existing;
  const request = api.herdrListAgents(projectId).finally(() => {
    if (flights.get(projectId) === request) flights.delete(projectId);
  });
  flights.set(projectId, request);
  return request;
}

export function useAgentStatus(project: Project) {
  const [agents, setAgents] = useState<HerdrAgent[]>([]);
  const refreshRef = useRef<() => void>(() => {});
  const refresh = useCallback(() => refreshRef.current(), []);
  useEffect(() => {
    let active = true;
    let running = false;
    let repeat = false;
    let generation = 0;
    setAgents([]);
    async function poll() {
      if (!active || running || !isVisible() || !project.workspace_id) return;
      running = true;
      do {
        repeat = false;
        const current = generation;
        try {
          const result = await readAgents(project.id);
          if (active && current === generation) setAgents(result);
        } catch {
          if (active && current === generation) setAgents([]);
        }
      } while (active && repeat && isVisible());
      running = false;
    }
    // A mutation discards the pre-mutation observation and schedules a fresh read.
    const invalidate = () => {
      generation++;
      setAgents([]);
      if (running) repeat = true;
      else void poll();
    };
    refreshRef.current = invalidate;
    const onVisible = () => { if (document.visibilityState !== "hidden") invalidate(); };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") { generation++; setAgents([]); }
      else onVisible();
    };
    void poll();
    const timer = window.setInterval(() => { void poll(); }, 3000);
    window.addEventListener("focus", onVisible);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      active = false;
      refreshRef.current = () => {};
      window.clearInterval(timer);
      window.removeEventListener("focus", onVisible);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [project.id, project.workspace_id, project.herdr_session]);
  return { agents, refreshAgents: refresh };
}
