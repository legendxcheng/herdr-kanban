import { useState } from "react";
import type { Task } from "./types";
import { resolveDispatch } from "./api";
import { tr, formatDateTime, useLocalizedMessage } from "./i18n";

export function DispatchRecovery({ task, onClose, onSaved }: { task: Task; onClose: () => void; onSaved: () => void }) {
  const [evidence, setEvidence] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useLocalizedMessage();
  const op = task.binding?.operation;
  async function resolve(sent: boolean) {
    if (!op) return;
    setBusy(true);
    try { await resolveDispatch(task.id, op.id, sent, evidence); onSaved(); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  return <div className="dialog-overlay"><section className="dialog" role="dialog" aria-label={tr("dispatch.verify")}>
    <h3>{tr("dispatch.verify")}</h3>
    <p>{tr("dispatch.verify_help")}</p>
    <p>{op?.target.pane_id} · {op && formatDateTime(op.created_at)}</p>
    <pre className="prompt-preview">{op?.prompt}</pre>
    <label>{tr("dispatch.evidence")}<textarea aria-label={tr("dispatch.evidence")} value={evidence} onChange={(e) => setEvidence(e.target.value)} /></label>
    {error && <p role="alert" className="error">{error}</p>}
    <div className="dialog-actions">
      <button onClick={onClose} disabled={busy} className="btn-secondary">{tr("common.cancel")}</button>
      <button onClick={() => resolve(false)} disabled={busy || evidence.trim().length < 3} className="btn-secondary">{tr("dispatch.not_sent")}</button>
      <button onClick={() => resolve(true)} disabled={busy || evidence.trim().length < 3} className="btn-primary">{tr("dispatch.sent")}</button>
    </div>
  </section></div>;
}
