import { useCallback, useEffect, useRef, useState } from "react";
import type { HerdrAgent, Project } from "./types";
import * as api from "./api";

// Share discovery across StrictMode/navigation and bound remote work independently of task reads.
const flights = new Map<string, Promise<HerdrAgent[]>>();
type Pool = "local" | "remote";
const queues: Record<Pool, (() => void)[]> = { local: [], remote: [] };
const activeReads: Record<Pool, number> = { local: 0, remote: 0 };
const isVisible = () => document.visibilityState !== "hidden";
function pump(pool: Pool) {
  while (activeReads[pool] < (pool === "local" ? 2 : 3) && queues[pool].length) queues[pool].shift()!();
}
function readAgents(projectId: string, pool: Pool) {
  const existing = flights.get(projectId);
  if (existing) return existing;
  const request = new Promise<HerdrAgent[]>((resolve, reject) => {
    queues[pool].push(() => {
      activeReads[pool]++;
      const read = document.visibilityState === "hidden" ? Promise.resolve([]) : api.herdrListAgents(projectId);
      void read.then(resolve, reject).finally(() => { activeReads[pool]--; pump(pool); });
    });
  }).finally(() => { if (flights.get(projectId) === request) flights.delete(projectId); });
  flights.set(projectId, request);
  pump(pool);
  return request;
}

export function useAgentStatus(projects: Project[]) {
  const scope = JSON.stringify(projects.map(p => [p.id, p.workspace_id, p.herdr_session, p.machine_id ?? "local"]).sort());
  const [snapshot, setSnapshot] = useState<{ scope: string; agents: Record<string, HerdrAgent[]> }>({ scope, agents: {} });
  const refreshRef = useRef<() => void>(() => {});
  const refreshAgents = useCallback(() => refreshRef.current(), []);
  useEffect(() => {
    let active = true;
    let generation = 0;
    const entries = (JSON.parse(scope) as string[][]).filter(p => p[1]);
    const pools = new Map(entries.map(p => [p[0], p[3] === "local" ? "local" as const : "remote" as const]));
    const ids = entries.map(p => p[0]);
    const running = new Set<string>();
    const repeat = new Set<string>();
    setSnapshot({ scope, agents: {} });
    async function poll(id: string) {
      if (!active || document.visibilityState === "hidden" || running.has(id)) return;
      running.add(id);
      do {
        repeat.delete(id);
        const current = generation;
        let agents: HerdrAgent[] = [];
        try { agents = await readAgents(id, pools.get(id)!); } catch { /* An unavailable target has unknown live state. */ }
        if (active && current === generation) setSnapshot(old => ({ scope, agents: { ...(old.scope === scope ? old.agents : {}), [id]: agents } }));
      } while (active && repeat.has(id) && isVisible());
      running.delete(id);
    }
    const pollAll = () => { ids.forEach(id => { void poll(id); }); };
    const invalidate = () => {
      generation++;
      setSnapshot({ scope, agents: {} });
      ids.forEach(id => { if (running.has(id)) repeat.add(id); else void poll(id); });
    };
    refreshRef.current = invalidate;
    pollAll();
    const timer = window.setInterval(pollAll, 3000);
    window.addEventListener("focus", invalidate);
    document.addEventListener("visibilitychange", invalidate);
    return () => {
      active = false; refreshRef.current = () => {};
      window.clearInterval(timer);
      window.removeEventListener("focus", invalidate);
      document.removeEventListener("visibilitychange", invalidate);
    };
  }, [scope]);
  return { agents: snapshot.scope === scope ? snapshot.agents : {}, refreshAgents };
}
