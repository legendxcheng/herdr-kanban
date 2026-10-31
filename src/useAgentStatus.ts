import { useCallback, useEffect, useRef, useState } from "react";
import type { AgentSnapshot, HerdrAgent, Project } from "./types";
import * as api from "./api";

export interface AgentRead {
  updating: boolean;
  failed: boolean;
  observedAt?: number;
  paused?: boolean;
  failures?: AgentSnapshot["failures"];
}

// Share discovery across StrictMode/navigation and bound remote work independently of task reads.
const flights = new Map<string, Promise<{ snapshot: AgentSnapshot; observedAt: number } | null>>();
type Pool = "local" | "remote";
const queues: Record<Pool, (() => void)[]> = { local: [], remote: [] };
const activeReads: Record<Pool, number> = { local: 0, remote: 0 };
const isVisible = () => document.visibilityState !== "hidden";
function pump(pool: Pool) {
  while (activeReads[pool] < (pool === "local" ? 2 : 3) && queues[pool].length) queues[pool].shift()!();
}
function readAgents(projectId: string, key: string, pool: Pool) {
  const existing = flights.get(key);
  if (existing) return existing;
  const request = new Promise<{ snapshot: AgentSnapshot; observedAt: number } | null>((resolve, reject) => {
    queues[pool].push(() => {
      activeReads[pool]++;
      const observedAt = Date.now();
      const read = !isVisible() ? Promise.resolve(null) : api.herdrAgentSnapshot(projectId).then(snapshot => ({ snapshot,
        // Never hide SSH response or IPC delivery delays behind a small remote
        // sampling age. Keep a conservative local upper bound on freshness.
        observedAt: snapshot.age_ms === undefined ? observedAt : Math.min(observedAt, Date.now() - Math.max(0, snapshot.age_ms)),
      }));
      void read.then(resolve, reject).finally(() => { activeReads[pool]--; pump(pool); });
    });
  }).finally(() => { if (flights.get(key) === request) flights.delete(key); });
  flights.set(key, request);
  pump(pool);
  return request;
}

export function useAgentStatus(projects: Project[]) {
  const scope = JSON.stringify(projects.map(p => [p.id, p.workspace_id, p.herdr_session, p.machine_id ?? "local"]).sort());
  const [snapshot, setSnapshot] = useState<{ scope: string; agents: Record<string, HerdrAgent[]>; reads: Record<string, AgentRead> }>({ scope, agents: {}, reads: {} });
  const [now, setNow] = useState(Date.now);
  const refreshRef = useRef<() => void>(() => {});
  const refreshAgents = useCallback(() => refreshRef.current(), []);
  useEffect(() => {
    let active = true;
    let generation = 0;
    const entries = (JSON.parse(scope) as string[][]).filter(p => p[1]);
    const pools = new Map(entries.map(p => [p[0], p[3] === "local" ? "local" as const : "remote" as const]));
    const keys = new Map(entries.map(p => [p[0], JSON.stringify(p)]));
    const ids = entries.map(p => p[0]);
    const running = new Set<string>();
    const repeat = new Set<string>();
    setSnapshot({ scope, agents: {}, reads: {} });
    function updateRead(id: string, read: Partial<AgentRead>, agents?: HerdrAgent[]) {
      if (!active) return;
      setSnapshot(old => {
        const same = old.scope === scope;
        const failedPanes = new Set(read.failures?.map(failure => failure.pane_id));
        const retained = same && agents ? (old.agents[id] ?? []).filter(agent => failedPanes.has(agent.pane_id)
          && !agents.some(current => current.pane_id === agent.pane_id)) : [];
        return { scope,
          agents: { ...(same ? old.agents : {}), ...(agents ? { [id]: [...agents, ...retained] } : {}) },
          reads: { ...(same ? old.reads : {}), [id]: { updating: false, failed: false, ...(same ? old.reads[id] : {}), ...read } },
        };
      });
    }
    async function poll(id: string) {
      if (!active || document.visibilityState === "hidden" || running.has(id)) return;
      running.add(id);
      do {
        repeat.delete(id);
        const current = generation;
        updateRead(id, { updating: true, paused: false });
        try {
          const result = await readAgents(id, keys.get(id)!, pools.get(id)!);
          if (active && current === generation && result) updateRead(id,
            { updating: false, failed: false, observedAt: result.observedAt, failures: result.snapshot.failures }, result.snapshot.agents);
        } catch {
          // Keep the last observation as history, never present it as a fresh success.
          if (active && current === generation) updateRead(id, { updating: false, failed: true });
        }
        if (active) setNow(Date.now());
      } while (active && repeat.has(id) && isVisible());
      running.delete(id);
    }
    const pollAll = () => { setNow(Date.now()); ids.forEach(id => { void poll(id); }); };
    const invalidate = () => {
      generation++;
      setNow(Date.now());
      if (!isVisible()) ids.forEach(id => updateRead(id, { paused: true, updating: false }));
      ids.forEach(id => { if (running.has(id)) repeat.add(id); else void poll(id); });
    };
    const focus = () => { if (isVisible()) pollAll(); };
    refreshRef.current = invalidate;
    pollAll();
    const timer = window.setInterval(pollAll, 3000);
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", invalidate);
    return () => {
      active = false; refreshRef.current = () => {};
      window.clearInterval(timer);
      window.removeEventListener("focus", focus);
      document.removeEventListener("visibilitychange", invalidate);
    };
  }, [scope]);
  return { agents: snapshot.scope === scope ? snapshot.agents : {}, reads: snapshot.scope === scope ? snapshot.reads : {}, now, refreshAgents };
}
