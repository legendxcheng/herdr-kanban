import { useState, useEffect, useRef } from "react";
import type { Task, Project, HerdrAgent } from "./types";
import * as api from "./api";
import { normalizeAgentStatus } from "./agentStatus";
import { hasPromptImages, resolveMentions } from "./dispatchUtils";
import { tr, useLocalizedMessage } from "./i18n";

interface DispatchDialogProps {
  task: Task;
  project: Project;
  onClose: () => void;
  onDispatched: () => void;
  preselectedPaneId?: string;
  onRefresh?: () => void;
}

export function DispatchDialog({ task, project, onClose, onDispatched, preselectedPaneId, onRefresh }: DispatchDialogProps) {
  const [agents, setAgents] = useState<HerdrAgent[]>([]);
  const boundPane = task.binding?.target?.pane_id;
  const [selected, setSelected] = useState<string>(boundPane ?? preselectedPaneId ?? "");
  const operation = useRef(crypto.randomUUID());
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useLocalizedMessage();
  const [previewOpen, setPreviewOpen] = useState(false);
  let preview = "";
  if (previewOpen) {
    try { preview = extractTextWithMentions(JSON.parse(task.prompt_json)); } catch { /* Invalid content has no preview. */ }
  }
  const [refresh, setRefresh] = useState(0);
  const isOccupied = (agent: HerdrAgent) => !!agent.occupied_by && agent.occupied_by !== task.id;

  // Load agents; once loaded, honour preselectedPaneId
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    api.herdrListAgents(project.id)
      .then((a) => {
        if (!active) return;
        setAgents(a);
        if (!boundPane && preselectedPaneId && a.some((ag) => ag.pane_id === preselectedPaneId && !isOccupied(ag))) {
          setSelected(preselectedPaneId);
        }
        setLoading(false);
      })
      .catch((e) => { if (active) { setError(String(e)); setLoading(false); } });
    return () => { active = false; };
  }, [project.id, project.workspace_id, refresh]);

  async function handleDispatch() {
    if (!selected) { setError("dispatch.choose_required"); return; }
    if (agents.some((agent) => agent.pane_id === selected && isOccupied(agent))) { setError("binding.occupied"); return; }
    setError("");
    setLoading(true);
    try {
      // Expand files, retaining skill names for the target agent to resolve.
      const doc = JSON.parse(task.prompt_json);
      const resolved = hasPromptImages(doc) ? "" : await resolveMentions(doc, project.root_path);
      if (!mounted.current) return;
      await api.dispatchToHerdr(task.id, selected, project.herdr_workspace, resolved, operation.current, hasPromptImages(doc) ? task.prompt_json : undefined);
      // A closed dialog may refresh persisted results, but cannot select a task
      // or close another dialog that the user has opened in the meantime.
      if (!mounted.current) { onRefresh?.(); return; }
      setLoading(false);
      onDispatched();
      onClose();
    } catch (e) {
      if (!mounted.current) { onRefresh?.(); return; }
      setError(String(e));
      if (!String(e).includes("verify_required")) operation.current = crypto.randomUUID();
      onRefresh?.();
      setLoading(false);
    }
  }

  return (
    <div className="dialog-overlay" onMouseDown={onClose}>
      <div className="dialog" onMouseDown={(e) => e.stopPropagation()}>
        <h3>{tr("dispatch.title", { name: task.name })}</h3>
        <div className="dialog-section">
          <label>{tr("dispatch.workspace")} <strong>{project.herdr_workspace}</strong></label>
        </div>
        <div className="dialog-section">
          <label>{tr("dispatch.choose_agent")}</label>
          {loading && agents.length === 0 ? (
            <p>{tr("common.loading")}</p>
          ) : agents.length === 0 ? (
            <p className="hint">{tr("dispatch.no_agents")}</p>
          ) : (
            <select aria-label={tr("binding.agent")} value={selected} onChange={(e) => setSelected(e.target.value)} disabled={loading || !!boundPane}>
              <option value="">{tr("dispatch.choose")}</option>
              {agents.map((a) => (
                <option key={a.pane_id} value={a.pane_id} disabled={isOccupied(a)}>
                  {a.title || a.name} ({tr(`agent_status.${normalizeAgentStatus(a.status)}`)})
                  {isOccupied(a) ? ` · ${tr("binding.occupied")}` : ""}
                </option>
              ))}
            </select>
          )}
          <button className="btn-ghost" disabled={loading} onClick={() => setRefresh((value) => value + 1)}>{tr("binding.refresh")}</button>
          <p className="hint">{tr("binding.prepare")}</p>
        </div>
        <div className="dialog-section">
          <button className="btn-secondary" onClick={() => setPreviewOpen(true)}>{tr("dispatch.preview")}</button>
          {preview && (
            <pre className="prompt-preview">{preview}</pre>
          )}
        </div>
        {error && <p className="error">{error}</p>}
        <div className="dialog-actions">
          <button onClick={onClose} className="btn-secondary">{tr("common.cancel")}</button>
          <button onClick={handleDispatch} disabled={loading || !selected || !!task.binding?.operation} className="btn-primary">
            {tr(loading ? "dispatch.sending" : "dispatch.action")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Extract text from a Tiptap doc, keeping mention placeholders for preview. */
function extractTextWithMentions(doc: any): string {
  if (!doc || !doc.content) return "";
  let text = "";
  function walk(node: any) {
    if (node.type === "text") {
      text += node.text;
    } else if (node.type === "promptImage") {
      text += `\n[${tr("image.reference")}: ${node.attrs.name}]\n`;
    } else if (node.type === "mention") {
      text += node.attrs.kind === "file"
        ? `\n[${tr("dispatch.file")}: ${node.attrs.label}]\n`
        : `$${node.attrs.label}`;
    } else if (node.content) {
      node.content.forEach(walk);
    }
    if (node.type === "paragraph" || node.type === "heading") {
      text += "\n";
    }
  }
  walk(doc);
  return text.trim();
}

