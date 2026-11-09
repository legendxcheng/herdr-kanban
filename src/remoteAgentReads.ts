import * as api from "./api";
import type { AgentSnapshot } from "./types";

type Read = { projectId: string; resolve: (snapshot: AgentSnapshot | null) => void; reject: (error: unknown) => void };
const pending = new Map<string, Read[]>();

// Collect this render/poll tick without adding a timer or delaying a round trip.
export function readRemoteAgents(machine: string, projectId: string): Promise<AgentSnapshot | null> {
  return new Promise((resolve, reject) => {
    const existing = pending.get(machine);
    const read = { projectId, resolve, reject };
    if (existing) { existing.push(read); return; }
    pending.set(machine, [read]);
    queueMicrotask(() => {
      const reads = pending.get(machine)!;
      pending.delete(machine);
      if (document.visibilityState === "hidden") { reads.forEach(r => r.resolve(null)); return; }
      void api.herdrAgentSnapshots([...new Set(reads.map(r => r.projectId))]).then(results => {
        for (const r of reads) {
          const result = results[r.projectId];
          if (result?.snapshot) r.resolve(result.snapshot);
          else r.reject(result?.error ?? "machine.invalid_response");
        }
      }).catch(error => reads.forEach(r => r.reject(error)));
    });
  });
}
