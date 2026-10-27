import { useCallback, useEffect, useRef, useState } from "react";
import type { NavigationState, Project, WorkspaceOperation } from "./types";
import * as api from "./api";
import { useLocalizedMessage } from "./i18n";
import { notify } from "./Toast";
import { useProjectGitStatus } from "./useProjectGitStatus";
import { useRemoteMachines } from "./useRemoteMachines";

export function useProjectSync() {
  const remote = useRemoteMachines();
  const [projects, setProjects] = useState<Project[]>([]);
  const [navigation, setNavigation] = useState<NavigationState>({ configured: false, recovery_projects: [] });
  const navigationVersion = useRef(0);
  const [gitRefresh, setGitRefresh] = useState(0);
  const allProjects = [...projects, ...remote.projects];
  const gitStatuses = useProjectGitStatus(allProjects, gitRefresh);
  const [operations, setOperations] = useState<WorkspaceOperation[]>([]);
  // Operation IDs cannot be reused after dismissal. Filter all snapshots, including
  // requests already in flight on an offline/slow machine, for this app lifetime.
  const [dismissedOperations, setDismissedOperations] = useState<Set<string>>(() => new Set());
  const dismissOperation = async (id: string) => {
    await api.dismissWorkspaceOperation(id);
    setDismissedOperations(previous => new Set(previous).add(id));
  };
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useLocalizedMessage();
  const [session, setSession] = useState<string | null>(null);
  const [reordering, setReordering] = useState(false);
  const moving = useRef(false);
  const revision = useRef(0);
  const mounted = useRef(false);
  const running = useRef<Promise<void> | null>(null);
  const repeat = useRef(false);
  const initialized = useRef(false);
  const reloadLocal = useRef(false);
  const sync = useCallback((localChanged = false) => {
    if (localChanged) revision.current++;
    const navRequest = ++navigationVersion.current;
    void api.navigationState().then(value => { if (mounted.current && navRequest === navigationVersion.current && value) setNavigation(value); }).catch(() => {});
    setGitRefresh(value => value + 1);
    reloadLocal.current ||= localChanged;
    if (moving.current) { repeat.current = true; return Promise.resolve(); }
    if (running.current) { repeat.current = true; return running.current; }
    const run = async () => {
      do {
        repeat.current = false;
        const observedRevision = revision.current;
        const current = () => mounted.current && !moving.current && observedRevision === revision.current;
        if (mounted.current) setSyncing(true);
        // Seed from durable navigation once. Local CRUD can explicitly reload it even
        // while offline; routine focus/timer sync keeps the current view until success.
        const readLocal = !initialized.current || reloadLocal.current;
        reloadLocal.current = false;
        if (readLocal) {
          try {
            const cached = await api.listProjects();
            if (current() && !repeat.current) { setProjects(cached.filter(p => !p.machine_id || p.machine_id === "local")); initialized.current = true; }
          } catch (e) { if (current()) setError(String(e)); }
        }
        try {
          const result = await api.syncProjects();
          if (current() && !repeat.current) { setProjects(result.projects); setSession(result.session); setOperations(result.operations); setError(""); initialized.current = true; }
        } catch (e) {
          if (current()) setError({ prefix: "workspace.offline", error: String(e) });
          try { const pending = await api.workspaceOperations(); if (current()) setOperations(pending); } catch { /* Keep last durable operation view. */ }
        }
      } while (repeat.current && mounted.current && !moving.current);
      if (mounted.current) setSyncing(false);
    };
    running.current = run().finally(() => { running.current = null; });
    return running.current;
  }, []);
  const refresh = useCallback(() => { void remote.refresh(true); return sync(true); }, [sync, remote.refresh]);
  const move = useCallback(async (projectId: string, beforeProjectId: string | null) => {
    if (moving.current) return;
    moving.current = true;
    revision.current++;
    const moveRevision = revision.current;
    setReordering(true);
    try {
      const result = await api.moveProject(projectId, beforeProjectId);
      if (mounted.current && revision.current === moveRevision) {
        setProjects(result.projects); setSession(result.session); setOperations(result.operations);
        setError(""); initialized.current = true;
      }
    } catch (e) {
      if (mounted.current) notify({ prefix: "workspace.reorder_failed", error: String(e) }, "error");
    } finally {
      revision.current++;
      moving.current = false;
      if (mounted.current) { setReordering(false); void sync(true); }
    }
  }, [sync]);
  useEffect(() => {
    mounted.current = true;
    void sync();
    const focus = () => { void sync(); };
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "visible") return;
      if (running.current) setGitRefresh(value => value + 1);
      else void sync();
    }, 15000);
    window.addEventListener("focus", focus);
    return () => { mounted.current = false; window.clearInterval(timer); window.removeEventListener("focus", focus); };
  }, [sync]);
  const moveOnMachine = async (id: string, before: string | null) => {
    if (remote.projects.some(p => p.id === id)) {
      try { await remote.move(id, before); } catch(e) { notify({ prefix: "workspace.reorder_failed", error: String(e) }, "error"); }
    } else await move(id, before);
  };
  const hide = async (id: string) => {
    await api.hideProject(id);
    revision.current++;
    setProjects(previous => previous.filter(p => p.id !== id));
    remote.forget(id);
    void refresh();
  };
  return { projects: allProjects, navigation, hide, machines: remote.machines, machineErrors: remote.errors, machineBusy: remote.busy, gitStatuses, operations: [...operations, ...remote.operations].filter(op => !dismissedOperations.has(op.id)), dismissOperation, syncing, error, refresh, session, reordering, move: moveOnMachine };
}
