import type { AgentDiagnostic, Task } from "./types";
import { errorText, formatDateTime, tr } from "./i18n";
import { openAgentStatusLogs } from "./api";
import { notify } from "./Toast";

export function AgentQueryDetails({ diagnostic, task }: { diagnostic: AgentDiagnostic; task: Task }) {
  const details = { ...diagnostic, selected_task_id: task.id, selected_pane_id: task.binding?.target?.pane_id,
    scope: diagnostic.pane_id ? "pane" : "workspace" };
  async function copy() {
    try {
      await navigator.clipboard.writeText(JSON.stringify(details, null, 2));
      notify({ key: "agent_diagnostic.copied" }, "success");
    } catch { notify({ key: "agent_diagnostic.copy_failed" }, "error"); }
  }
  async function openLogs() {
    try { await openAgentStatusLogs(); }
    catch (error) { notify({ error: String(error) }, "error"); }
  }
  return <section className="agent-query-details" aria-label={tr("agent_diagnostic.title")}
    onClick={event => event.stopPropagation()} onKeyDown={event => event.stopPropagation()}>
    <p className="task-notice error">{tr(`agent_diagnostic.stage.${diagnostic.stage}`)}：{errorText(diagnostic.code)}</p>
    <p className="task-notice"><code>{diagnostic.code}</code> · {tr(diagnostic.pane_id ? "agent_diagnostic.pane" : "agent_diagnostic.workspace")}</p>
    <p className="task-notice">{formatDateTime(diagnostic.at)} · {diagnostic.elapsed_ms} ms</p>
    <div className="task-recovery">
      <button className="btn-secondary" onClick={() => void copy()}>{tr("agent_diagnostic.copy")}</button>
      <button className="btn-secondary" onClick={() => void openLogs()}>{tr("agent_diagnostic.open_logs")}</button>
    </div>
    <p className="task-notice">{tr("agent_diagnostic.log_hint")}</p>
  </section>;
}
