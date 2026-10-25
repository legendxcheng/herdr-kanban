import { useState, useEffect, useRef } from "react";
import type { Project, ProjectSnapshot } from "./types";
import * as api from "./api";
import { tr, errorText } from "./i18n";

interface ProjectFormDialogProps {
  existing?: Project | null;
  onClose: () => void;
  onSaved: () => void;
}
export function ProjectFormDialog({ existing, onClose, onSaved }: ProjectFormDialogProps) {
  const [name, setName] = useState(existing?.name ?? "");
  const [root, setRoot] = useState(existing?.root_path ?? "");
  const [mode, setMode] = useState(existing ? (existing.workspace_id && existing.sync_status !== "missing" ? "edit" : "link") : "create");
  const [selected, setSelected] = useState("");
  const [snapshot, setSnapshot] = useState<ProjectSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [uncertain, setUncertain] = useState(false);
  const [useHerdrDirectory, setUseHerdrDirectory] = useState(false);
  const busy = useRef(false);
  const operation = useRef(crypto.randomUUID());
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    if (mode !== "link") return;
    let active = true;
    setLoading(true);
    api.syncProjects().then((s) => { if (active) { setSnapshot(s); setError(""); } })
      .catch((e) => { if (active) setError(errorText(e)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [mode, refresh]);
  async function save() {
    if (busy.current) return;
    if (mode === "create" && (!name.trim() || !root.trim())) { setError(tr("workspace.invalid_input")); return; }
    busy.current = true; setSaving(true); setError("");
    try {
      if (uncertain) await api.recoverWorkspace(operation.current);
      else if (mode === "create") await api.createProject(name.trim(), root.trim(), operation.current);
      else if (mode === "link") await api.linkWorkspace(existing?.id ?? null, selected, root.trim() || null);
      else if (existing) await api.updateProject(existing.id, name.trim(), useHerdrDirectory ? "" : root !== existing.root_path ? root.trim() : null);
      onSaved(); onClose();
    } catch (e) {
      setError(errorText(e));
      if (mode === "create") {
        // Durable operation state, not the error text, determines whether retry may create.
        try {
          const ops = await api.workspaceOperations();
          const pending = ops.find((o) => o.id === operation.current);
          setUncertain(!!pending && pending.state !== "failed");
        } catch { setUncertain(true); }
      }
    } finally { busy.current = false; setSaving(false); }
  }
  const candidates = snapshot?.workspaces.filter((w) => !snapshot.projects.some((p) => p.id !== existing?.id && p.herdr_session === snapshot.session && p.workspace_id === w.id)) ?? [];
  const directoryReadonly = mode === "edit" && existing?.directory_source === "herdr";
  const close = () => { if (!busy.current) onClose(); };
  return <div className="dialog-overlay" onMouseDown={close}>
    <section className="dialog workspace-dialog" role="dialog" aria-label={tr(existing ? "workspace.edit" : "workspace.new")} onMouseDown={(e) => e.stopPropagation()}>
      <h3>{tr(existing ? "workspace.edit" : "workspace.new")}</h3>
      <p className="hint">{tr("workspace.source_help")}</p>
      {!existing && <div className="workspace-mode">
        <button disabled={saving || uncertain} className={mode === "create" ? "btn-primary" : "btn-secondary"} onClick={() => setMode("create")}>{tr("workspace.new")}</button>
        <button disabled={saving || uncertain} className={mode === "link" ? "btn-primary" : "btn-secondary"} onClick={() => setMode("link")}>{tr("workspace.add_existing")}</button>
      </div>}
      {mode === "link" ? <div className="dialog-section">
        <label htmlFor="workspace-choice">{tr("workspace.choose")}</label>
        <select id="workspace-choice" disabled={saving || loading} value={selected} onChange={(e) => { setSelected(e.target.value); const w = candidates.find((w) => w.id === e.target.value); setRoot(w?.directory ?? ""); }}>
          <option value="">{tr("workspace.choose")}</option>
          {candidates.map((w) => <option key={w.id} value={w.id}>{w.name} · {w.directory || tr("workspace.directory_missing")} · {w.id}</option>)}
        </select>
        <button className="btn-ghost" disabled={saving || loading} onClick={() => setRefresh((n) => n + 1)}>{tr("workspace.refresh")}</button>
        {!loading && snapshot && !candidates.length && <p className="hint">{tr("workspace.no_candidates")}</p>}
      </div> : <div className="dialog-section">
        <label htmlFor="workspace-name">{tr("workspace.name")}</label>
        <input id="workspace-name" value={name} disabled={saving || uncertain} onChange={(e) => setName(e.target.value)} autoFocus />
      </div>}
      <div className="dialog-section">
        <label htmlFor="workspace-directory">{tr("workspace.directory")}</label>
        <div className="workspace-path">
          <input id="workspace-directory" value={useHerdrDirectory ? existing?.herdr_directory ?? "" : root} readOnly={!!directoryReadonly || useHerdrDirectory || (mode === "link" && !!candidates.find((w) => w.id === selected)?.directory)} disabled={saving || uncertain} onChange={(e) => setRoot(e.target.value)} />
          {!directoryReadonly && !useHerdrDirectory && <button type="button" className="btn-secondary" disabled={saving || uncertain} onClick={async () => { try { const path = await api.pickFolder(); if (path) setRoot(path); } catch (e) { setError(errorText(e)); } }}>{tr("workspace.browse")}</button>}
        </div>
        {existing && <p className="hint">{tr(`workspace.directory_${existing.directory_source}`)}</p>}
        {existing?.herdr_directory && existing.directory_source !== "herdr" && <label className="workspace-directory-option"><input type="checkbox" checked={useHerdrDirectory} disabled={saving} onChange={(e) => setUseHerdrDirectory(e.target.checked)} />{tr("workspace.use_directory")} · {existing.herdr_directory}</label>}
      </div>
      {existing?.workspace_id && mode === "edit" && <button className="btn-ghost" disabled={saving} onClick={() => setMode("link")}>{tr("workspace.relink")}</button>}
      {error && <p className="error" role="alert">{error}</p>}
      {uncertain && <p className="hint">{tr("workspace.verify_help")}</p>}
      <div className="dialog-actions">
        <button className="btn-secondary" disabled={saving} onClick={close}>{tr("common.cancel")}</button>
        <button className="btn-primary" disabled={saving || loading || (mode === "link" && !selected)} onClick={save}>{tr(saving ? "workspace.saving" : uncertain ? "workspace.recover" : mode === "create" ? "workspace.create" : "workspace.save")}</button>
      </div>
    </section>
  </div>;
}
