import { useEffect, useRef } from "react";
import { onStartChanged } from "./api";

/** Events prompt authoritative reads; polling/focus also recover missed notifications. */
export function useTaskFlowUpdates(projectId: string, reload: () => Promise<void>) {
  const latest = useRef(reload);
  latest.current = reload;
  useEffect(() => {
    let active = true;
    let running = false;
    let queued = false;
    let unlisten: (() => void) | undefined;
    async function reconcile() {
      if (!active || document.visibilityState === "hidden") return;
      if (running) { queued = true; return; }
      running = true;
      try { await latest.current(); }
      finally {
        running = false;
        if (queued && active) { queued = false; void reconcile(); }
      }
    }
    void onStartChanged(event => {
      if (event.project_id === projectId) void reconcile();
    }).then(stop => {
      if (!active) { stop(); return; }
      unlisten = stop;
      // Close the gap between initial load and listener registration.
      void reconcile();
    }).catch(() => { /* Polling remains available if the event transport fails. */ });
    const timer = window.setInterval(() => void reconcile(), 2000);
    window.addEventListener("focus", reconcile);
    document.addEventListener("visibilitychange", reconcile);
    return () => {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener("focus", reconcile);
      document.removeEventListener("visibilitychange", reconcile);
      unlisten?.();
    };
  }, [projectId]);
}
