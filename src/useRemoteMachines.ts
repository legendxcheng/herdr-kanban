import { useCallback, useEffect, useRef, useState } from "react";
import type { Machine, Project, ProjectSnapshot, WorkspaceOperation } from "./types";
import * as api from "./api";

export function useRemoteMachines() {
  const [machines, setMachines] = useState<Machine[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [operations, setOperations] = useState<Record<string, WorkspaceOperation[]>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const alive = useRef(false);
  const known = useRef<Machine[]>([]);
  const seeded = useRef(false);
  const sequence = useRef(0);
  const revisions = useRef<Record<string, number>>({});
  const operationVersions = useRef<Record<string, number>>({});
  const running = useRef(new Map<string, Promise<void>>());
  const moving = useRef(new Set<string>());
  const reloading = useRef(new Set<string>());
  const repeat = useRef(new Set<string>());
  const apply = useCallback((id: string, snapshot: ProjectSnapshot) => {
    operationVersions.current[id] = (operationVersions.current[id] ?? 0) + 1;
    setProjects(previous => [...previous.filter(p => p.machine_id !== id), ...snapshot.projects]);
    setOperations(previous => ({ ...previous, [id]: snapshot.operations }));
    setErrors(previous => ({ ...previous, [id]: "" }));
  }, []);
  const syncOne = useCallback(function sync(id: string) {
    if (!alive.current || !known.current.some(m => m.id === id && !m.disabled)) return;
    if (running.current.has(id) || moving.current.has(id) || reloading.current.has(id)) {
      repeat.current.add(id);
      return;
    }
    repeat.current.delete(id);
    const revision = revisions.current[id] ?? 0;
    const current = () => alive.current && (revisions.current[id] ?? 0) === revision && known.current.some(m => m.id === id && !m.disabled);
    setBusy(previous => ({ ...previous, [id]: true }));
    const request = api.syncProjects(id).then(snapshot => {
      if (current()) apply(id, snapshot);
    }, error => {
      if (current()) setErrors(previous => ({ ...previous, [id]: String(error) }));
    }).finally(() => {
      running.current.delete(id);
      if (alive.current) setBusy(previous => ({ ...previous, [id]: false }));
      if (repeat.current.delete(id)) sync(id);
    });
    running.current.set(id, request);
  }, [apply]);
  const refresh = useCallback(async (reload = false) => {
    const request = ++sequence.current;
    try {
      const list = await api.listMachines();
      if (!alive.current || request !== sequence.current) return;
      const remote = (list ?? []).filter(m => m.kind === "ssh");
      for (const machine of remote) {
        if (reload || known.current.find(m => m.id === machine.id)?.disabled !== machine.disabled)
          revisions.current[machine.id] = (revisions.current[machine.id] ?? 0) + 1;
        if (!seeded.current || reload) reloading.current.add(machine.id);
      }
      known.current = remote; setMachines(remote);
      if (!seeded.current || reload) {
        const cached = await api.listProjects();
        if (!alive.current || request !== sequence.current) return;
        setProjects(cached.filter(p => p.machine_id && p.machine_id !== "local"));
        seeded.current = true;
        for (const machine of remote) {
          const version = operationVersions.current[machine.id] ?? 0;
          void api.workspaceOperations(machine.id).then(pending => {
            if(alive.current && (operationVersions.current[machine.id] ?? 0) === version)
              setOperations(previous => ({...previous,[machine.id]:pending ?? []}));
          }).catch(() => {});
        }
      }
      for (const machine of remote) reloading.current.delete(machine.id);
      for (const machine of remote) if (!machine.disabled) syncOne(machine.id);
    } catch {
      if (request === sequence.current) {
        reloading.current.clear();
        for (const machine of known.current) syncOne(machine.id);
      }
      // Keep the durable last-known remote groups until a successful refresh.
    }
  }, [syncOne]);
  const move = useCallback(async (projectId: string, before: string | null) => {
    const machine = projects.find(p => p.id === projectId)?.machine_id;
    if (!machine || moving.current.has(machine)) return;
    moving.current.add(machine);
    revisions.current[machine] = (revisions.current[machine] ?? 0) + 1;
    try { const snapshot = await api.moveProject(projectId, before); if (alive.current) apply(machine, snapshot); }
    finally { moving.current.delete(machine); revisions.current[machine]++; syncOne(machine); }
  }, [projects, apply, syncOne]);
  useEffect(() => {
    alive.current = true;
    void refresh();
    const focus = () => { void refresh(); };
    const timer = window.setInterval(() => { if (document.visibilityState === "visible") void refresh(); }, 15000);
    window.addEventListener("focus", focus);
    return () => { alive.current = false; sequence.current++; window.clearInterval(timer); window.removeEventListener("focus", focus); };
  }, [refresh]);
  const sorted = machines.flatMap(m => projects.filter(p => p.machine_id === m.id));
  return { machines, projects: sorted, operations: Object.values(operations).flat(), errors, busy, refresh, move };
}
