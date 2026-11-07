import { useEffect, useState } from "react";
import { taskRecoveryStatus, type RecoveryStatus } from "./api";

/** Display only: the application-owned coordinator runs without this component. */
export function useTaskRecovery() {
  const [statuses, setStatuses] = useState<Record<string, RecoveryStatus>>({});
  useEffect(() => {
    let active = true;
    let running = false;
    async function read() {
      if (!active || running || document.visibilityState === "hidden") return;
      running = true;
      try {
        const result = await taskRecoveryStatus();
        if (active && Array.isArray(result)) setStatuses(Object.fromEntries(result.map(status => [status.task_id, status])));
      } catch { /* Task and terminal errors remain available if the status IPC fails. */ }
      finally { running = false; }
    }
    void read();
    const timer = window.setInterval(() => void read(), 2000);
    window.addEventListener("focus", read);
    document.addEventListener("visibilitychange", read);
    return () => { active = false; clearInterval(timer); window.removeEventListener("focus", read); document.removeEventListener("visibilitychange", read); };
  }, []);
  return statuses;
}
