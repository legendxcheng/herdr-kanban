import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import type { Project, Task } from "./types";
import { tr, useLocalizedMessage } from "./i18n";
import { preparationLabel } from "./taskFlow";
import { terminalBounds } from "./nativeTerminalBounds";
import { Icon } from "./Icon";

export function TerminalPanel({ task, workspace, machineName, onRepair, maximized = false, onToggleMaximized }: {
  task: Task | null;
  workspace?: Project;
  machineName?: string;
  onRepair?: () => void;
  maximized?: boolean;
  onToggleMaximized?: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [state, setState] = useState("terminal.empty");
  const [error, setError] = useLocalizedMessage();
  const [attempt, setAttempt] = useState(0);
  const identity = JSON.stringify(task?.binding?.target);
  const bindingState = task?.binding?.state;
  const preparation = task ? preparationLabel(task) : null;
  useEffect(() => {
    setError("");
    if ((!task && !workspace) || !host.current) { setState("terminal.empty"); return; }
    if (!workspace && preparation) { setState(preparation); return; }
    if (!workspace && bindingState !== "valid") { setState("binding.repair_required"); return; }
    const connectionId = crypto.randomUUID();
    let active = true;
    let ended = false;
    let opened = false;
    let lastBounds = "";
    let syncing = false;
    let dirty = true;
    setState("terminal.connecting");
    function fail(e: unknown) {
      if (!active || ended) return;
      ended = true;
      setState("terminal.disconnected"); setError(String(e));
      void api.closeTerminal(connectionId).catch(() => {});
    }
    async function syncBounds() {
      dirty = true;
      if (!active || ended || !opened || syncing) return;
      syncing = true;
      try {
        while (active && !ended && dirty && host.current) {
          dirty = false;
          const bounds = terminalBounds(host.current);
          const key = JSON.stringify(bounds);
          if (key === lastBounds) continue;
          await api.setTerminalBounds(connectionId, bounds);
          lastBounds = key;
        }
      } catch (e) { fail(e); }
      finally { syncing = false; }
    }
    // Observe the entire layout: a title/status change or modal may move the
    // slot without changing its size. Native code handles main-window movement.
    const observer = new ResizeObserver(() => { void syncBounds(); });
    observer.observe(host.current);
    const mutations = new MutationObserver(() => { void syncBounds(); });
    mutations.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["class", "style", "inert", "role"] });
    const update = () => { void syncBounds(); };
    window.addEventListener("resize", update);
    document.addEventListener("visibilitychange", update);
    const initial = terminalBounds(host.current);
    const open = workspace ? api.openWorkspaceTerminal : api.openTerminal;
    void open(workspace?.id ?? task!.id, connectionId, initial, packet => {
      if (active && !ended && packet.kind === "closed") fail(packet.message);
    }).then(() => {
      if (!active || ended) { void api.closeTerminal(connectionId).catch(() => {}); return; }
      opened = true; setState("terminal.connected");
      void syncBounds();
    }).catch(fail);
    return () => {
      active = false;
      observer.disconnect(); mutations.disconnect();
      window.removeEventListener("resize", update);
      document.removeEventListener("visibilitychange", update);
      void api.closeTerminal(connectionId).catch(() => {});
    };
  // Reconnect only when the selected identity/state changes, not on list refreshes.
  }, [task?.id, identity, bindingState, attempt, preparation, workspace?.id, workspace?.machine_id, workspace?.herdr_session, workspace?.workspace_id]);

  const title = workspace ? tr("workspace_terminal.title", { workspace: workspace.name }) : task?.name ?? tr("terminal.label");

  return <aside className={`terminal-panel${maximized ? " terminal-panel-maximized" : ""}`} aria-label={tr("terminal.label")}>
    <header className="terminal-header">
      <strong title={task || workspace ? `${machineName ?? tr("machine.local")} · ${title}` : undefined}>{title}</strong>
      {(machineName || task || workspace) && <span className="terminal-machine" title={machineName ?? tr("machine.local")}>{machineName ?? tr("machine.local")}</span>}
      <span role="status">{tr(state)}</span>
      {onToggleMaximized && <button type="button" className="btn-icon terminal-size-toggle"
        aria-label={tr(maximized ? "terminal.restore" : "terminal.maximize")}
        title={tr(maximized ? "terminal.restore" : "terminal.maximize")} onClick={onToggleMaximized}>
        <Icon name={maximized ? "restore" : "maximize"} />
      </button>}
    </header>
    {!task && !workspace && <p className="terminal-hint">{tr("terminal.empty")}</p>}
    {(task || workspace) && state !== "terminal.connected" && <div className="terminal-feedback">
      {error && <p role="alert">{machineName ?? tr("machine.local")} · {error}</p>}
      {!preparation && state !== "terminal.connecting" && <div className="task-actions">
        {(bindingState === "valid" || workspace) && <button className="btn-secondary" onClick={() => setAttempt((n) => n + 1)}>{tr("terminal.reconnect")}</button>}
        {onRepair && <button className="btn-ghost" onClick={onRepair}>{tr("binding.repair")}</button>}
      </div>}
    </div>}
    <div className="terminal-body">
      <div ref={host} className="terminal-host" onKeyDown={(e) => e.stopPropagation()} />
    </div>
  </aside>;
}
