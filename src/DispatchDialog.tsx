import { useState, useEffect, useRef } from "react";
import type { Task, Project, HerdrAgent } from "./types";
import * as api from "./api";
import { hasPromptImages, resolveMentions } from "./dispatchUtils";
import { tr, errorText } from "./i18n";

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
  const [error, setError] = useState<string>("");
  const [preview, setPreview] = useState<string>("");
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
      .catch((e) => { if (active) { setError(errorText(e)); setLoading(false); } });
    return () => { active = false; };
  }, [project.id, project.workspace_id, refresh]);

  function buildPreview() {
    // Preview file placeholders and native skill invocations.
    try {
      const doc = JSON.parse(task.prompt_json);
      const text = extractTextWithMentions(doc);
      setPreview(text);
    } catch {
      setPreview("");
    }
  }

  async function handleDispatch() {
    if (!selected) { setError("请先选择 Agent"); return; }
    if (agents.some((agent) => agent.pane_id === selected && isOccupied(agent))) { setError(tr("binding.occupied")); return; }
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
      setError(errorText(e));
      if (!String(e).includes("verify_required")) operation.current = crypto.randomUUID();
      onRefresh?.();
      setLoading(false);
    }
  }

  return (
    <div className="dialog-overlay" onMouseDown={onClose}>
      <div className="dialog" onMouseDown={(e) => e.stopPropagation()}>
        <h3>派发任务: {task.name}</h3>
        <div className="dialog-section">
          <label>目标 Workspace: <strong>{project.herdr_workspace}</strong></label>
        </div>
        <div className="dialog-section">
          <label>选择 Agent</label>
          {loading && agents.length === 0 ? (
            <p>加载中…</p>
          ) : agents.length === 0 ? (
            <p className="hint">未找到 Agent。确认 Herdr 正在运行且 workspace 内有 agent。</p>
          ) : (
            <select aria-label={tr("binding.agent")} value={selected} onChange={(e) => setSelected(e.target.value)} disabled={loading || !!boundPane}>
              <option value="">-- 选择 --</option>
              {agents.map((a) => (
                <option key={a.pane_id} value={a.pane_id} disabled={isOccupied(a)}>
                  {a.title || a.name} ({a.status})
                  {isOccupied(a) ? ` · ${tr("binding.occupied")}` : ""}
                </option>
              ))}
            </select>
          )}
          <button className="btn-ghost" disabled={loading} onClick={() => setRefresh((value) => value + 1)}>{tr("binding.refresh")}</button>
          <p className="hint">{tr("binding.prepare")}</p>
        </div>
        <div className="dialog-section">
          <button className="btn-secondary" onClick={buildPreview}>预览提示词</button>
          {preview && (
            <pre className="prompt-preview">{preview}</pre>
          )}
        </div>
        {error && <p className="error">{error}</p>}
        <div className="dialog-actions">
          <button onClick={onClose} className="btn-secondary">取消</button>
          <button onClick={handleDispatch} disabled={loading || !selected || !!task.binding?.operation} className="btn-primary">
            {loading ? "派发中…" : "派发"}
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
        ? `\n[@文件: ${node.attrs.label}]\n`
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

