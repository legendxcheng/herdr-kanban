import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import type { HerdrTarget } from "./types";
import { tr, useLocale, errorText } from "./i18n";
import { Icon } from "./Icon";
import "./sessionCleanup.css";
interface Item {
    id: string;
    target: HerdrTarget;
    machine: string;
    workspace: string;
    state: string;
    error: string;
    warning?: string;
}
interface Batch {
    id: string;
    state: string;
    items: Item[];
    scopes: {
        machine: string;
        workspace: string;
        error: string;
    }[];
}
interface Resource {
    id: number;
    task_id: string;
    target: HerdrTarget;
    reason: string;
    state: string;
    error: string;
}
export function SessionCleanupButton({ onChanged }: {
    onChanged?: () => void;
}) {
    useLocale();
    const [open, setOpen] = useState(false);
    return <><button className="btn-icon" title={tr("session_cleanup.title")} aria-label={tr("session_cleanup.title")} onClick={() => setOpen(true)}><Icon name="trash"/></button>
    {open && <CleanupDialog onClose={() => setOpen(false)} onChanged={onChanged}/>}</>;
}
function CleanupDialog({ onClose, onChanged }: {
    onClose: () => void;
    onChanged?: () => void;
}) {
    const [batch, setBatch] = useState<Batch | null>(null);
    const [history, setHistory] = useState<Batch[]>([]);
    const [resources, setResources] = useState<Resource[]>([]);
    const [busy, setBusy] = useState(false);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState("");
    const alive = useRef(true);
    const flight = useRef(false);
    const cancel = useRef<HTMLButtonElement>(null);
    async function records() {
        const [h, r] = await Promise.all([invoke<Batch[]>("session_cleanup_history"), invoke<Resource[]>("session_resource_list")]);
        if (alive.current) {
            setHistory(h);
            setResources(r);
        }
    }
    async function preview(resourceId?: number) {
        if (flight.current)
            return;
        flight.current = true;
        setLoading(true);
        setError("");
        try {
            const result = await invoke<Batch>(resourceId === undefined ? "preview_session_cleanup" : "preview_session_resource", resourceId === undefined ? {} : {resourceId});
            if (alive.current)
                setBatch(result);
            await records();
        }
        catch (e) {
            if (alive.current)
                setError(String(e));
        }
        finally {
            flight.current = false;
            if (alive.current)
                setLoading(false);
        }
    }
    useEffect(() => {
        alive.current = true;
        const root = document.getElementById("root");
        const previous = root?.inert ?? false;
        const opener = document.activeElement as HTMLElement | null;
        if (root)
            root.inert = true;
        cancel.current?.focus();
        void preview();
        return () => { alive.current = false; if (root)
            root.inert = previous; if (opener?.isConnected)
            opener.focus(); };
    }, []);
    async function operate(id: string, verify: boolean, retry = false) {
        if (flight.current)
            return;
        flight.current = true;
        setBusy(true);
        setError("");
        let polling = true;
        const timer = verify ? undefined : window.setInterval(() => { void invoke<Batch>("session_cleanup_result", { batchId: id }).then(b => { if (alive.current && polling)
            setBatch(b); }).catch(() => { }); }, 1000);
        try {
            const result = await invoke<Batch>(retry ? "retry_session_cleanup" : verify ? "session_cleanup_result" : "execute_session_cleanup", { batchId: id });
            polling = false;
            if (alive.current) {
                setBatch(result);
                onChanged?.();
                window.dispatchEvent(new Event("focus"));
            }
            await records();
        }
        catch (e) {
            if (alive.current)
                setError(String(e));
        }
        finally {
            polling = false;
            if (timer !== undefined)
                clearInterval(timer);
            flight.current = false;
            if (alive.current) {
                setBusy(false);
                cancel.current?.focus();
            }
        }
    }
    const candidates = batch?.items.filter(i => i.state === "ready") ?? [];
    const working = candidates.filter(i => ["working", "blocked", "unknown"].includes(i.target.status)).length;
    const counts = batch?.items.reduce<Record<string, number>>((all, i) => { all[i.state] = (all[i.state] ?? 0) + 1; return all; }, {}) ?? {};
    return createPortal(<div className="dialog-overlay" onMouseDown={e => { if (!busy && e.target === e.currentTarget)
        onClose(); }}>
    <section className="dialog session-cleanup-dialog" role="dialog" aria-modal="true" aria-labelledby="session-cleanup-title" aria-busy={busy || loading} onKeyDown={e => {
            if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                if (!busy)
                    onClose();
            }
            if (e.key === "Tab") {
                const buttons = [...e.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
                if (e.shiftKey && document.activeElement === buttons[0]) {
                    e.preventDefault();
                    buttons[buttons.length - 1]?.focus();
                }
                else if (!e.shiftKey && document.activeElement === buttons[buttons.length - 1]) {
                    e.preventDefault();
                    buttons[0]?.focus();
                }
            }
        }}>
      <h3 id="session-cleanup-title">{tr("session_cleanup.title")}</h3>
      <p>{tr("session_cleanup.message")}</p>
      {loading && <p role="status">{tr("cleanup.loading")}</p>}
      {batch && <>
        <p role="status">{batch.state === "preview" ? tr("session_cleanup.count", { count: candidates.length, working }) : tr("session_cleanup.result", { closed: counts.closed ?? 0, skipped: (counts.skipped ?? 0) + (counts.protected ?? 0), failed: counts.failed ?? 0, unknown: (counts.unknown ?? 0) + (counts.closing ?? 0) + (counts.claimed ?? 0) })}</p>
        {!!batch.scopes.length && <div className="session-cleanup-warning"><strong>{tr("session_cleanup.unscanned")}</strong><ul>{batch.scopes.map((s, i) => <li key={i}>{s.machine} · {s.workspace} — {errorText(s.error)}</li>)}</ul></div>}
        <div className="session-cleanup-list"><table><thead><tr><th>{tr("session_cleanup.location")}</th><th>{tr("session_cleanup.agent")}</th><th>{tr("session_cleanup.state")}</th></tr></thead>
          <tbody>{batch.items.map(item => <tr key={item.id}><td>{item.machine} · {item.workspace}</td><td>{item.target.title || item.target.pane_id}<br /><small>{item.target.agent} · {item.target.status}</small></td><td>{tr(`session_cleanup.state.${item.state}`)}{item.error && <small>{errorText(item.error)}</small>}{item.warning && <small>{errorText(item.warning)}</small>}</td></tr>)}</tbody></table></div>
      </>}
      {error && <p role="alert" className="error">{errorText(error)}</p>}
      <details><summary>{tr("session_cleanup.history")}</summary>{history.map((h, i) => <button key={h.id} disabled={busy || loading} className="btn-secondary" onClick={() => void operate(h.id, true)}>{tr("session_cleanup.history_item", { index: i + 1, count: h.items.length })}</button>)}</details>
      <details><summary>{tr("session_cleanup.resources", { count: resources.length })}</summary><ul>{resources.map(r => <li key={r.id}>{r.target.machine_id} · {r.target.workspace_id} · {r.target.title || r.target.pane_id} — {tr(`session_cleanup.reason.${r.reason}`)}{r.error && ` · ${errorText(r.error)}`} <button className="btn-secondary" disabled={busy || loading} onClick={() => void preview(r.id)}>{tr("session_cleanup.inspect_resource")}</button></li>)}</ul></details>
      <div className="dialog-actions">
        <button ref={cancel} className="btn-secondary" disabled={busy} onClick={onClose}>{tr(batch?.state === "done" ? "common.close" : "common.cancel")}</button>
        <button className="btn-secondary" disabled={busy || loading} onClick={() => void preview()}>{tr("session_cleanup.refresh")}</button>
        {batch?.state === "preview" && <button className="btn-primary" disabled={busy || loading || !candidates.length || !!error} onClick={() => void operate(batch.id, false)}>{tr("session_cleanup.confirm")}</button>}
        {batch?.state === "done" && batch.items.some(i => i.state === "unknown") && <button className="btn-secondary" disabled={busy || loading} onClick={() => void operate(batch.id, false, true)}>{tr("session_cleanup.retry")}</button>}
        {batch && batch.state !== "preview" && <button className="btn-primary" disabled={busy || loading} onClick={() => void operate(batch.id, true)}>{tr("cleanup.verify")}</button>}
      </div>
    </section>
  </div>, document.body);
}
