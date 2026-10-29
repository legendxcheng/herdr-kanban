import { useEffect, useRef, useState } from "react";
import type { EnrollmentStatus, Machine, SavedMachine } from "./types";
import * as api from "./api";
import { tr, useLocalizedMessage } from "./i18n";

export function MachineDialog({ machines, onChanged, onClose, onConnected }: {
  machines: Machine[]; onChanged: () => void; onClose: () => void; onConnected?: (machine: Machine) => void;
}) {
  const [profiles, setProfiles] = useState<SavedMachine[]>([]);
  const [profile, setProfile] = useState("");
  const [target, setTarget] = useState("");
  const [session, setSession] = useState("default");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useLocalizedMessage();
  const [operation, setOperation] = useState<EnrollmentStatus | null>(null);
  const [verified, setVerified] = useState(false);
  const dialog = useRef<HTMLElement>(null);
  const alive = useRef(false);
  const locked = useRef(false);
  const generation = useRef(0);
  const callbacks = useRef({onChanged,onConnected}); callbacks.current = {onChanged,onConnected};
  const waiting = operation?.state === "waiting";
  async function reload() {
    const items = await api.savedMachines();
    if (alive.current) { setProfiles(items); setProfile(current => items.some(p => p.id === current && p.enabled) ? current : items.find(p => p.enabled)?.id ?? ""); }
    return items;
  }
  useEffect(() => {
    alive.current = true;
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.focus();
    void reload().catch(e => { if (alive.current) setError(String(e)); });
    return () => { alive.current = false; generation.current++; previous?.focus(); };
  }, []);
  async function run(action: () => Promise<void>) {
    if (locked.current) return;
    locked.current = true; setBusy(true); setError(""); setVerified(false);
    try { await action(); }
    catch(e) { if (alive.current) setError(String(e)); }
    finally { locked.current = false; if (alive.current) setBusy(false); }
  }
  async function connect(id: string) {
    const request = ++generation.current;
    try {
      const machine = await api.connectMachine(id);
      if (alive.current && request === generation.current) {
        setVerified(true); callbacks.current.onChanged();
        if (callbacks.current.onConnected) callbacks.current.onConnected(machine);
        else await reload();
      }
    } catch(e) {
      if (alive.current && request === generation.current) callbacks.current.onChanged();
      throw e;
    }
  }
  function enroll(configureKey: boolean) {
    void run(async () => {
      const result = await api.enrollMachine(target.trim(), session.trim(), configureKey);
      if (alive.current) setOperation(result);
    });
  }
  useEffect(() => {
    if (!operation || operation.state !== "waiting") return;
    let active = true;
    let timer: number | undefined;
    const id = operation.id;
    const expectedTarget = target.trim(), expectedSession = session.trim();
    const poll = async () => {
      try {
        const status = await api.enrollmentStatus(id);
        if (!active || !alive.current) return;
        if (status.state === "waiting") { timer = window.setTimeout(poll, 750); return; }
        setOperation(status);
        if (status.state !== "succeeded") { setError(status.error || "machine.enrollment_failed"); return; }
        void run(async () => {
          const items = await reload();
          if (!alive.current) return;
          const matches = items.filter(p => p.enabled && p.target === expectedTarget && p.session === expectedSession);
          if (matches.length !== 1) throw new Error("machine.profile_selection_required");
          setProfile(matches[0].id);
          await connect(matches[0].id);
        });
      } catch(e) {
        if (active && alive.current) { setError(String(e)); timer = window.setTimeout(poll, 2000); }
      }
    };
    void poll();
    return () => { active = false; window.clearTimeout(timer); };
  }, [operation?.id, operation?.state]);
  const close = () => { if (!locked.current) onClose(); };
  return <div className="dialog-overlay" onMouseDown={close}>
    <section ref={dialog} tabIndex={-1} className="dialog workspace-dialog" role="dialog" aria-modal="true" aria-label={tr(onConnected ? "machine.add_remote" : "machine.manage")} onMouseDown={e => e.stopPropagation()} onKeyDown={e => {
      if(e.key === "Escape") close();
      if(e.key === "Tab") {
        const controls = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled)') ?? []);
        if(e.shiftKey && (document.activeElement === controls[0] || document.activeElement === dialog.current)) { e.preventDefault(); controls[controls.length-1]?.focus(); }
        else if(!e.shiftKey && document.activeElement === controls[controls.length-1]) { e.preventDefault(); controls[0]?.focus(); }
      }
    }}>
      <h3>{tr(onConnected ? "machine.add_remote" : "machine.manage")}</h3>
      <p className="hint">{tr("machine.steps")}</p><p className="hint">{tr("machine.help")}</p>
      {!onConnected && machines.map(machine => <div className="machine-entry" key={machine.id}>
        <strong>Remote · {machine.name}</strong><span>{machine.ssh_target} · {machine.session}</span>
        {(machine.disabled || !machine.helper_name) && <span>{tr(!machine.helper_name ? "machine.setup_required" : "machine.disabled_short")}</span>}
        <button disabled={busy || waiting} className="btn-secondary" onClick={() => { const name = prompt(tr("machine.rename"), machine.name); if(name?.trim()) void run(async () => { await api.renameMachine(machine.id,name.trim()); callbacks.current.onChanged(); }); }}>{tr("machine.rename")}</button>
        <button disabled={busy || waiting || !machine.profile_id} className="btn-secondary" onClick={() => void run(() => connect(machine.profile_id!))}>{tr("machine.check_connect")}</button>
        {!machine.disabled && <button disabled={busy || waiting} className="btn-secondary" onClick={() => { if(confirm(tr("machine.remove_confirm"))) void run(async () => { await api.removeMachine(machine.id); callbacks.current.onChanged(); }); }}>{tr("machine.remove")}</button>}
      </div>)}
      <div className="dialog-section"><label htmlFor="machine-profile">{tr("machine.saved")}</label>
        <select id="machine-profile" disabled={busy || waiting} value={profile} onChange={e => { setProfile(e.target.value); setVerified(false); }}><option value="">{tr("machine.choose")}</option>{profiles.filter(p => p.enabled).map(p => <option key={p.id} value={p.id}>{p.label} · {p.target} · {p.session}</option>)}</select>
        <button className="btn-primary" disabled={busy || waiting || !profile} onClick={() => void run(() => connect(profile))}>{tr(busy ? "machine.connecting" : "machine.connect")}</button>
        <button className="btn-secondary" disabled={busy || waiting} onClick={() => void run(async () => { await reload(); })}>{tr("workspace.refresh")}</button>
      </div>
      <div className="dialog-section"><label htmlFor="machine-target">{tr("machine.ssh_alias")}</label><input id="machine-target" placeholder="devpc" value={target} disabled={busy || waiting} onChange={e => setTarget(e.target.value)} />
        <label htmlFor="machine-session">{tr("machine.session")}</label><input id="machine-session" value={session} disabled={busy || waiting} onChange={e => setSession(e.target.value)} />
        <p className="hint">{tr("machine.enroll_help")}</p>
        <button className="btn-primary" disabled={busy || waiting || !target.trim() || !session.trim()} onClick={() => enroll(false)}>{tr("machine.connect_existing")}</button>
        <button className="btn-secondary" disabled={busy || waiting || !target.trim() || !session.trim()} onClick={() => enroll(true)}>{tr("machine.enroll")}</button>
      </div>
      {waiting && <p role="status">{tr("machine.enroll_waiting")}</p>}
      {busy && <p role="status">{tr("machine.connecting")}</p>}
      {verified && <p role="status">{tr("machine.verified")}</p>}
      {error && <p className="error" role="alert">{error}</p>}
      <div className="dialog-actions"><button className="btn-secondary" disabled={busy} onClick={close}>{tr("common.close")}</button></div>
    </section>
  </div>;
}
