import { useEffect, useState } from "react";
import type { HerdrAgent, Project, Task } from "./types";
import * as api from "./api";
import { tr, errorText } from "./i18n";

export function BindingDialog({ task, project, onClose, onSaved }: { task: Task; project: Project; onClose: () => void; onSaved: () => void }) {
  const [agents, setAgents] = useState<HerdrAgent[]>([]);
  const [selected, setSelected] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    let active = true;
    setLoading(true); setError("");
    api.herdrListAgents(project.id).then((items) => { if (active) setAgents(items); })
      .catch((e) => { if (active) setError(errorText(e)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [project.id, project.workspace_id, refresh]);
  async function save() {
    setBusy(true); setError("");
    try { await api.repairBinding(task.id, selected); onSaved(); }
    catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  return <div className="dialog-overlay"><section className="dialog" role="dialog" aria-label={tr("binding.repair")}>
    <h3>{tr("binding.repair")} · {task.name}</h3>
    <p>{tr("binding.repair_help")}</p>
    <label>{tr("binding.agent")}<select aria-label={tr("binding.agent")} value={selected} disabled={busy || loading} onChange={(e) => setSelected(e.target.value)}>
      <option value="">{tr("binding.choose")}</option>
      {agents.map((agent) => {
        const occupied = !!agent.occupied_by && agent.occupied_by !== task.id;
        return <option key={agent.pane_id} value={agent.pane_id} disabled={occupied}>{agent.title || agent.name} · {agent.pane_id}{occupied ? ` · ${tr("binding.occupied")}` : ""}</option>;
      })}
    </select></label>
    <p className="hint">{tr("binding.prepare")}</p>
    <button className="btn-ghost" disabled={busy || loading} onClick={() => setRefresh((n) => n + 1)}>{tr("binding.refresh")}</button>
    {error && <p role="alert" className="error">{error}</p>}
    <div className="dialog-actions">
      <button className="btn-secondary" disabled={busy} onClick={onClose}>{tr("common.cancel")}</button>
      <button className="btn-primary" disabled={busy || loading || !selected || !!task.binding?.operation} onClick={save}>{tr("binding.save")}</button>
    </div>
  </section></div>;
}
