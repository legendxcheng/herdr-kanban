import { useEffect, useRef, useState } from "react";
import type { Machine, WorkspaceChoices } from "./types";
import * as api from "./api";
import { tr, useLocalizedMessage } from "./i18n";
import { MachineDialog } from "./MachineDialog";
import { ProjectFormDialog } from "./ProjectFormDialog";

export const machineLabel = (id: string, name?: string) => id === "local" ? `Local · ${tr("machine.this_computer")}` : `Remote · ${name ?? id}`;

export function WorkspaceAddDialog({ machines, onClose, onSaved }: { machines: Machine[]; onClose: () => void; onSaved: () => void }) {
  const [machineId, setMachineId] = useState("local");
  const [connected, setConnected] = useState<Machine | null>(null);
  const [addingMachine, setAddingMachine] = useState(false);
  const [creating, setCreating] = useState(false);
  const [choices, setChoices] = useState<WorkspaceChoices | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [search, setSearch] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useLocalizedMessage();
  const dialog = useRef<HTMLElement>(null);
  const busy = useRef(false);
  const alive = useRef(true);
  const allMachines = connected ? [...machines.filter(m => m.id !== connected.id), connected] : machines;
  const available = machineId === "local" || allMachines.some(m => m.id === machineId && !m.disabled && !!m.helper_name);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => {
    if (addingMachine || creating) return;
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.focus();
    return () => previous?.focus();
  }, [addingMachine, creating]);
  useEffect(() => {
    if (addingMachine || creating) return;
    let active = true;
    setLoading(true); setChoices(null); setError("");
    if (!available) { setLoading(false); setError("machine.setup_required"); return; }
    api.workspaceChoices(machineId).then(value => {
      if (active && value.machine_id === machineId) {
        setChoices(value);
        setSelected(current => current.filter(id => value.workspaces.some(w => w.id === id && !w.shown && !w.blocked)));
      }
    }).catch(e => { if (active) setError(String(e)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [machineId, refresh, addingMachine, creating, available]);
  const close = () => { if (!busy.current) onClose(); };
  async function save() {
    if (busy.current || !choices || !selected.length) return;
    busy.current = true; setSaving(true); setError("");
    try {
      await api.addExistingWorkspaces(machineId, choices.session, selected);
      if (alive.current) { onSaved(); onClose(); }
    } catch(e) { if (alive.current) setError(String(e)); }
    finally { busy.current = false; if (alive.current) setSaving(false); }
  }
  if (addingMachine) return <MachineDialog machines={allMachines} onChanged={onSaved} onClose={() => setAddingMachine(false)} onConnected={machine => {
    setConnected(machine); setMachineId(machine.id); setSelected([]); setSearch(""); setAddingMachine(false); onSaved();
  }} />;
  if (creating) return <ProjectFormDialog machines={allMachines} initialMachineId={machineId} onAddExisting={() => setCreating(false)} onClose={onClose} onSaved={onSaved} />;
  const matching = choices?.workspaces.filter(w => `${w.name} ${w.directory ?? ""} ${w.id}`.toLocaleLowerCase().includes(search.toLocaleLowerCase())) ?? [];
  return <div className="dialog-overlay" onMouseDown={close}>
    <section ref={dialog} tabIndex={-1} className="dialog workspace-dialog" role="dialog" aria-modal="true" aria-label={tr("workspace.add")} onMouseDown={e => e.stopPropagation()} onKeyDown={e => {
      if (e.key === "Escape") close();
      if (e.key === "Tab") {
        const items = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled)') ?? []);
        if (e.shiftKey && (document.activeElement === items[0] || document.activeElement === dialog.current)) { e.preventDefault(); items[items.length-1]?.focus(); }
        else if (!e.shiftKey && document.activeElement === items[items.length-1]) { e.preventDefault(); items[0]?.focus(); }
      }
    }}>
      <h3>{tr("workspace.add")}</h3><p className="hint">{tr("workspace.selection_help")}</p>
      <div className="dialog-section">
        <label htmlFor="add-workspace-machine">{tr("machine.computer")}</label>
        <select id="add-workspace-machine" value={machineId} disabled={saving} onChange={e => { setMachineId(e.target.value); setChoices(null); setSelected([]); setSearch(""); }}>
          <option value="local">{machineLabel("local")}</option>
          {allMachines.map(m => <option key={m.id} value={m.id} disabled={m.disabled || !m.helper_name}>{machineLabel(m.id,m.name)}{m.disabled ? ` · ${tr("machine.disabled_short")}` : !m.helper_name ? ` · ${tr("machine.setup_required")}` : ""}</option>)}
        </select>
        <button className="btn-secondary" disabled={saving} onClick={() => setAddingMachine(true)}>{tr("machine.add_remote")}</button>
      </div>
      <div className="workspace-mode"><button className="btn-primary" disabled={saving}>{tr("workspace.add_existing")}</button><button className="btn-secondary" disabled={saving || !available} onClick={() => setCreating(true)}>{tr("workspace.new")}</button></div>
      <div className="dialog-section">
        <label htmlFor="workspace-search">{tr("workspace.search")}</label><input id="workspace-search" value={search} disabled={saving} onChange={e => setSearch(e.target.value)} />
        <div className="workspace-choices" aria-busy={loading}>
          {loading && <p role="status">{tr("workspace.syncing")}</p>}
          {!loading && choices && !matching.length && <p className="hint">{tr("workspace.no_matches")}</p>}
          {!loading && matching.map(w => <label className="workspace-choice" key={w.id}>
            <input type="checkbox" checked={w.shown || selected.includes(w.id)} disabled={saving || w.shown || w.blocked} onChange={e => setSelected(current => e.target.checked ? [...current,w.id] : current.filter(id => id !== w.id))} />
            <span><strong>{w.name}</strong><small title={w.directory ?? undefined}>{w.directory || tr("workspace.directory_missing")}</small></span>
            {w.shown ? <small>{tr("workspace.added")}</small> : w.blocked ? <small>{tr("workspace.recover")}</small> : null}
          </label>)}
        </div>
        <button className="btn-ghost" disabled={saving || loading} onClick={() => setRefresh(n => n+1)}>{tr("workspace.refresh")}</button>
      </div>
      {error && <p className="error" role="alert">{error}</p>}
      <div className="dialog-actions"><button className="btn-secondary" disabled={saving} onClick={close}>{tr("common.cancel")}</button><button className="btn-primary" disabled={saving || loading || !choices || !selected.length} onClick={save}>{saving ? tr("workspace.saving") : tr("workspace.add_selected", { count: selected.length })}</button></div>
    </section>
  </div>;
}
