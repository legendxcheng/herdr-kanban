import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import * as api from "./api";
import { Icon } from "./Icon";
import { errorText, tr, useLocale } from "./i18n";

export function IdleCleanupButton({ projectId, workspace, machine, onChanged }: {
  projectId: string; workspace: string; machine: string; onChanged: () => void;
}) {
  useLocale();
  const [open, setOpen] = useState(false);
  return <>
    <button className="btn-icon" title={tr("cleanup.title")} aria-label={tr("cleanup.title")} onClick={() => setOpen(true)}><Icon name="trash" /></button>
    {open && <CleanupDialog key={projectId} projectId={projectId} workspace={workspace} machine={machine}
      onChanged={onChanged} onClose={() => setOpen(false)} />}
  </>;
}

function CleanupDialog({ projectId, workspace, machine, onClose, onChanged }: {
  projectId: string; workspace: string; machine: string; onClose: () => void; onChanged: () => void;
}) {
  const id = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  const mounted = useRef(true);
  const inFlight = useRef(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<api.IdleCleanupPreview | null>(null);
  const [report, setReport] = useState<api.IdleCleanupReport | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    mounted.current = true;
    const root = document.getElementById("root");
    const wasInert = root?.inert ?? false;
    const opener = document.activeElement as HTMLElement | null;
    if (root) root.inert = true;
    cancel.current?.focus();
    api.previewIdleCleanup(projectId).then(value => { if (mounted.current) setPreview(value); })
      .catch(e => { if (mounted.current) setError(String(e)); })
      .finally(() => { if (mounted.current) setLoading(false); });
    return () => { mounted.current = false; if (root) root.inert = wasInert; if (opener?.isConnected) opener.focus(); };
  }, [projectId]);
  async function execute(verifyId?: string) {
    if (!preview || inFlight.current) return;
    inFlight.current = true; setBusy(true); setError("");
    try {
      const result = verifyId ? await api.idleCleanupResult(projectId, verifyId) : await api.executeIdleCleanup(projectId, preview.id);
      if (mounted.current) { setReport(result); onChanged(); }
    } catch (e) { if (mounted.current) setError(String(e)); }
    finally { inFlight.current = false; if (mounted.current) { setBusy(false); cancel.current?.focus(); } }
  }
  const unresolved = report?.items.some(item => ["unknown", "closing", "claimed"].includes(item.state));
  return createPortal(<div className="dialog-overlay" onMouseDown={e => {
    if (!busy && e.target === e.currentTarget) { e.preventDefault(); onClose(); }
  }}>
    <section className="dialog" role="dialog" aria-modal="true" aria-busy={busy || loading}
      aria-labelledby={`${id}-title`} aria-describedby={`${id}-message`} onKeyDown={e => {
        if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); if (!busy) onClose(); }
        if (e.key === "Tab") {
          const buttons = [...e.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
          if (!buttons.length) { e.preventDefault(); return; }
          if (e.shiftKey && document.activeElement === buttons[0]) { e.preventDefault(); buttons[buttons.length - 1]?.focus(); }
          else if (!e.shiftKey && document.activeElement === buttons[buttons.length - 1]) { e.preventDefault(); buttons[0].focus(); }
        }
      }}>
      <h3 id={`${id}-title`}>{tr("cleanup.title")}</h3>
      <p>{machine} · {workspace} · kanban</p>
      <p id={`${id}-message`}>{tr("cleanup.message")}</p>
      {loading && <p role="status">{tr("cleanup.loading")}</p>}
      {preview && !report && <p role="status">{tr(preview.count ? "cleanup.count" : "cleanup.empty", { count: preview.count })}</p>}
      {!report && preview?.unresolved?.map(previous => <p key={previous}>
        <button className="btn-secondary" disabled={busy} onClick={() => void execute(previous)}>{tr("cleanup.verify_previous")}</button>
      </p>)}
      {report && <div role="status">
        <p>{tr("cleanup.result", { closed: report.closed, skipped: report.skipped, failed: report.failed })}</p>
        <ul style={{ maxHeight: "220px", overflowY: "auto" }}>{report.items.filter(item => item.error).map(item =>
          <li key={item.pane_id}>{item.pane_id} — {errorText(item.error)}</li>)}</ul>
      </div>}
      {error && <p role="alert" className="error">{errorText(error)}</p>}
      <div className="dialog-actions">
        <button ref={cancel} className="btn-secondary" disabled={busy} onClick={onClose}>{tr(report ? "common.close" : "common.cancel")}</button>
        {!report && <button className="btn-primary" disabled={loading || busy || !preview?.count || !!error} onClick={() => void execute()}>{tr("cleanup.confirm")}</button>}
        {unresolved && <button className="btn-secondary" disabled={busy} onClick={() => void execute(report!.id)}>{tr("cleanup.verify")}</button>}
      </div>
    </section>
  </div>, document.body);
}
