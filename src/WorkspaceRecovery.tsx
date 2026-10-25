import { useRef, useState } from "react";
import type { HerdrWorkspace, WorkspaceOperation } from "./types";
import * as api from "./api";
import { tr, useLocalizedMessage } from "./i18n";

export function WorkspaceRecovery({ operation, onChanged }: { operation: WorkspaceOperation; onChanged: () => void }) {
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const [error, setError] = useLocalizedMessage();
  const [choices, setChoices] = useState<HerdrWorkspace[]>([]);
  const [selected, setSelected] = useState("");
  async function recover() {
    if (locked.current) return;
    locked.current = true; setBusy(true); setError("");
    try {
      if (operation.state === "failed") await api.createProject(operation.name, operation.cwd, operation.id);
      else await api.recoverWorkspace(operation.id, selected || null);
      onChanged();
    } catch (e) {
      setError(String(e));
      try {
        const snapshot = await api.syncProjects();
        setChoices(snapshot.workspaces.filter((w) => !snapshot.operations.some((o) => o.id !== operation.id && (o.workspace_id === w.id || w.name === `kanban-create-${o.id}`) && o.state !== "failed")));
      } catch { /* Show original error; cached tasks remain available. */ }
    } finally { locked.current = false; setBusy(false); }
  }
  return <section className="workspace-recovery">
    <strong>{operation.name}</strong>
    <p className="hint">{operation.cwd}</p>
    {!operation.workspace_id && choices.length > 0 && <select aria-label={tr("workspace.choose_created")} disabled={busy} value={selected} onChange={(e) => setSelected(e.target.value)}>
      <option value="">{tr("workspace.choose_created")}</option>
      {choices.map((w) => <option value={w.id} key={w.id}>{w.name} · {w.id}</option>)}
    </select>}
    <button className="btn-secondary" disabled={busy} onClick={recover}>{tr(operation.state === "failed" ? "workspace.retry_failed" : "workspace.recover")}</button>
    {error && <p className="error" role="alert">{error}</p>}
  </section>;
}
