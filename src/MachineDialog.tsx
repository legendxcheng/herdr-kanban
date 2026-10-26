import { useEffect, useRef, useState } from "react";
import type { Machine, SavedMachine } from "./types";
import * as api from "./api";
import { tr, useLocalizedMessage } from "./i18n";

export function MachineDialog({ machines, onChanged, onClose }: { machines: Machine[]; onChanged: () => void; onClose: () => void }) {
  const [profiles, setProfiles] = useState<SavedMachine[]>([]);
  const [profile, setProfile] = useState("");
  const [target, setTarget] = useState("devpc");
  const [session, setSession] = useState("default");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useLocalizedMessage();
  const [message, setMessage] = useState(false);
  const dialog = useRef<HTMLElement>(null);
  const reload = () => api.savedMachines().then(items => { setProfiles(items); setProfile(current => items.some(p => p.id === current) ? current : items[0]?.id ?? ""); }).catch(e => setError(String(e)));
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.focus(); void reload();
    return () => previous?.focus();
  }, []);
  async function run(action: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true); setError("");
    try { await action(); onChanged(); await reload(); } catch(e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  return <div className="dialog-overlay" onMouseDown={() => { if (!busy) onClose(); }}>
    <section ref={dialog} tabIndex={-1} className="dialog workspace-dialog" role="dialog" aria-modal="true" aria-label={tr("machine.manage")} onMouseDown={e => e.stopPropagation()} onKeyDown={e => {
      if(e.key === "Escape" && !busy) onClose();
      if(e.key === "Tab") {
        const controls = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled)') ?? []);
        if(e.shiftKey && (document.activeElement === controls[0] || document.activeElement === dialog.current)) { e.preventDefault(); controls[controls.length - 1]?.focus(); }
        else if(!e.shiftKey && document.activeElement === controls[controls.length - 1]) { e.preventDefault(); controls[0]?.focus(); }
      }
    }}>
      <h3>{tr("machine.manage")}</h3>
      <p className="hint">{tr("machine.help")}</p>
      {machines.map(machine => <div className="machine-entry" key={machine.id}>
        <strong>{machine.name}</strong><span>{machine.ssh_target} · {machine.session}</span>
        <button disabled={busy} className="btn-secondary" onClick={() => { const name = prompt(tr("machine.rename"), machine.name); if(name?.trim()) void run(() => api.renameMachine(machine.id, name.trim())); }}>{tr("machine.rename")}</button>
        {machine.disabled ? <button disabled={busy || !machine.profile_id} className="btn-secondary" onClick={() => void run(() => api.connectMachine(machine.profile_id!))}>{tr("machine.connect")}</button> : <button disabled={busy} className="btn-secondary" onClick={() => { if(confirm(tr("machine.remove_confirm"))) void run(() => api.removeMachine(machine.id)); }}>{tr("machine.remove")}</button>}
      </div>)}
      <div className="dialog-section"><label htmlFor="machine-profile">{tr("machine.saved")}</label>
        <select id="machine-profile" disabled={busy} value={profile} onChange={e => setProfile(e.target.value)}><option value="">{tr("machine.choose")}</option>{profiles.filter(p => p.enabled).map(p => <option key={p.id} value={p.id}>{p.label} · {p.target} · {p.session}</option>)}</select>
        <button className="btn-primary" disabled={busy || !profile} onClick={() => void run(() => api.connectMachine(profile))}>{tr(busy ? "machine.connecting" : "machine.connect")}</button>
        <button className="btn-secondary" disabled={busy} onClick={() => void reload()}>{tr("workspace.refresh")}</button>
      </div>
      <div className="dialog-section"><label htmlFor="machine-target">{tr("machine.ssh_alias")}</label><input id="machine-target" value={target} disabled={busy} onChange={e => setTarget(e.target.value)} />
        <label htmlFor="machine-session">{tr("machine.session")}</label><input id="machine-session" value={session} disabled={busy} onChange={e => setSession(e.target.value)} />
        <button className="btn-secondary" disabled={busy || !target.trim() || !session.trim()} onClick={() => void run(async () => { await api.enrollMachine(target.trim(), session.trim()); setMessage(true); })}>{tr("machine.enroll")}</button>
        {message && <p role="status">{tr("machine.enroll_help")}</p>}
      </div>
      {error && <p className="error" role="alert">{error}</p>}
      <div className="dialog-actions"><button className="btn-secondary" disabled={busy} onClick={onClose}>{tr("common.close")}</button></div>
    </section>
  </div>;
}
