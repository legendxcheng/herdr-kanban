import { useCallback, useEffect, useRef, useState } from "react";
import type { Project, WorkspaceOperation } from "./types";
import * as api from "./api";
import { useLocalizedMessage } from "./i18n";
import { notify } from "./Toast";
import { useProjectGitStatus } from "./useProjectGitStatus";

export function useProjectSync() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [gitRefresh, setGitRefresh] = useState(0);
  const gitStatuses = useProjectGitStatus(projects, gitRefresh);
  const [operations, setOperations] = useState<WorkspaceOperation[]>([]);
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
            if (current() && !repeat.current) { setProjects(cached); initialized.current = true; }
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
  const refresh = useCallback(() => sync(true), [sync]);
  const move = useCallback(async (projectId: string, beforeProjectId: string | null) => {
    if (moving.current) return;
    moving.current = true;
    revision.current++;
    setReordering(true);
    try {
      const result = await api.moveProject(projectId, beforeProjectId);
      if (mounted.current) {
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
  return { projects, gitStatuses, operations, syncing, error, refresh, session, reordering, move };
}
