import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import * as api from "./api";
import type { Task } from "./types";
import { tr, errorText } from "./i18n";
import { preparationLabel } from "./taskFlow";
import { terminalAppearance } from "./terminalAppearance";
import { changeTerminalFontSize, readTerminalFontSize, terminalFontSizeDelta } from "./terminalFontSize";
import { attachTerminalScroll } from "./terminalScroll";
import { Icon } from "./Icon";

export function TerminalPanel({ task, onRepair, maximized = false, onToggleMaximized }: {
  task: Task | null;
  onRepair?: () => void;
  maximized?: boolean;
  onToggleMaximized?: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [state, setState] = useState("terminal.empty");
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const identity = JSON.stringify(task?.binding?.target);
  const bindingState = task?.binding?.state;
  const preparation = task ? preparationLabel(task) : null;
  useEffect(() => {
    setError("");
    if (!task || !host.current) { setState("terminal.empty"); return; }
    if (preparation) { setState(preparation); return; }
    if (bindingState !== "valid") { setState("binding.repair_required"); return; }
    const connectionId = crypto.randomUUID();
    let active = true;
    let connected = false;
    let ended = false;
    let pendingInput = "";
    let writing = false;
    let inputTimer: ReturnType<typeof setTimeout> | undefined;
    let sizeGeneration = 0;
    let sizeSync = Promise.resolve();
    let sizeAcknowledged = false;
    let renderedSize = { cols: 0, rows: 0 };
    const terminal = new Terminal({ ...terminalAppearance, fontSize: readTerminalFontSize(), scrollback: 3000, convertEol: false });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host.current);
    const scroll = attachTerminalScroll(host.current, terminal, connectionId, fail);
    terminal.attachCustomKeyEventHandler((event) => {
      const delta = terminalFontSizeDelta(event);
      if (!delta) return true;
      // Consume every phase, but adjust only on keydown (including key repeat).
      event.preventDefault();
      event.stopPropagation();
      if (active && event.type === "keydown") {
        const size = changeTerminalFontSize(terminal.options.fontSize!, delta);
        if (size !== terminal.options.fontSize) {
          terminal.options.fontSize = size;
          fit.fit();
        }
      }
      return false;
    });
    fit.fit();
    terminal.options.disableStdin = true;
    setState("terminal.connecting");
    function fail(e: unknown) {
      if (!active) return;
      ended = true; connected = false; pendingInput = "";
      scroll.pause();
      terminal.options.disableStdin = true;
      setState("terminal.disconnected"); setError(errorText(e));
      void api.closeTerminal(connectionId).catch(() => {});
    }
    async function flushInput() {
      if (!active || !connected || writing || !pendingInput) return;
      const data = pendingInput; pendingInput = ""; writing = true;
      try { await api.inputTerminal(connectionId, data); }
      catch (e) { fail(e); }
      finally { writing = false; if (active && connected && pendingInput) void flushInput(); }
    }
    const dataListener = terminal.onData((data) => {
      if (!active || !connected) return;
      if (new TextEncoder().encode(pendingInput + data).length > 65536) { fail("terminal.input_limit"); return; }
      pendingInput += data;
      clearTimeout(inputTimer); inputTimer = setTimeout(() => void flushInput(), 10);
    });
    function syncSize() {
      scroll.pause();
      sizeAcknowledged = false;
      const generation = ++sizeGeneration;
      const { cols, rows } = terminal;
      sizeSync = sizeSync.then(async () => {
        if (!active || !connected || generation !== sizeGeneration) return;
        await api.resizeTerminal(connectionId, cols, rows);
        if (active && connected && generation === sizeGeneration) { sizeAcknowledged = true; enableScroll(); }
      }).catch(fail);
    }
    function enableScroll() {
      if (active && connected && sizeAcknowledged && renderedSize.cols === terminal.cols && renderedSize.rows === terminal.rows) scroll.enable();
    }
    const resizeListener = terminal.onResize(syncSize);
    const observer = new ResizeObserver(() => { if (active) fit.fit(); });
    observer.observe(host.current);
    void api.openTerminal(task.id, connectionId, terminal.cols, terminal.rows, (packet) => {
      if (!active || ended) return;
      if (packet.kind === "closed") { fail(packet.message); return; }
      // Output can arrive before the invoke result. It belongs to this connection.
      terminal.write(new Uint8Array(packet.data), () => {
        if (packet.cols && packet.rows) renderedSize = { cols: packet.cols, rows: packet.rows };
        enableScroll();
        if (active && !ended) void api.ackTerminal(connectionId, packet.data.length).catch(fail);
      });
    }).then(() => {
      if (!active || ended) { void api.closeTerminal(connectionId).catch(() => {}); return; }
      connected = true; terminal.options.disableStdin = false; setState("terminal.connected");
      fit.fit();
      // fit may already have run during connect, without an active resize channel.
      syncSize();
    }).catch(fail);
    return () => {
      active = false; connected = false; clearTimeout(inputTimer); pendingInput = "";
      scroll.dispose();
      observer.disconnect(); dataListener.dispose(); resizeListener.dispose(); terminal.dispose();
      void api.closeTerminal(connectionId).catch(() => {});
    };
  // Reconnect only when the selected identity/state changes, not on list refreshes.
  }, [task?.id, identity, bindingState, attempt, preparation]);

  return <aside className={`terminal-panel${maximized ? " terminal-panel-maximized" : ""}`} aria-label={tr("terminal.label")}>
    <header className="terminal-header">
      <strong>{task?.name ?? tr("terminal.label")}</strong><span role="status">{tr(state)}</span>
      {onToggleMaximized && <button type="button" className="btn-icon terminal-size-toggle"
        aria-label={tr(maximized ? "terminal.restore" : "terminal.maximize")}
        title={tr(maximized ? "terminal.restore" : "terminal.maximize")} onClick={onToggleMaximized}>
        <Icon name={maximized ? "restore" : "maximize"} />
      </button>}
    </header>
    {!task && <p className="terminal-hint">{tr("terminal.empty")}</p>}
    {task && state !== "terminal.connected" && <div className="terminal-feedback">
      {error && <p role="alert">{error}</p>}
      {!preparation && state !== "terminal.connecting" && <div className="task-actions">
        {bindingState === "valid" && <button className="btn-secondary" onClick={() => setAttempt((n) => n + 1)}>{tr("terminal.reconnect")}</button>}
        {onRepair && <button className="btn-ghost" onClick={onRepair}>{tr("binding.repair")}</button>}
      </div>}
    </div>}
    <div className="terminal-body">
      <div ref={host} className="terminal-host" onKeyDown={(e) => e.stopPropagation()} />
    </div>
  </aside>;
}
