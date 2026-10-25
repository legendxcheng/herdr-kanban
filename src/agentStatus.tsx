import type { HerdrAgent, HerdrTarget, Task } from "./types";
import { tr } from "./i18n";

export type AgentStatus = "working" | "blocked" | "done" | "idle" | "unknown" | "not_started" | "closed";
export function normalizeAgentStatus(status: string): AgentStatus {
  return ["working", "blocked", "done", "idle"].includes(status) ? status as AgentStatus : "unknown";
}
function sameTarget(a: HerdrTarget, b: HerdrTarget) {
  return a.session === b.session && a.workspace_id === b.workspace_id && a.pane_id === b.pane_id
    && a.terminal_id === b.terminal_id && a.agent === b.agent && a.process_id === b.process_id;
}
export function taskAgentStatus(task: Task, agents: HerdrAgent[], busy = false): AgentStatus {
  if (busy || task.cleanup_required || task.binding?.operation
    || ["pending", "unknown"].includes(task.flow?.state ?? "")) return "unknown";
  const binding = task.binding;
  if (binding?.state === "valid" && binding.target) {
    const live = agents.find(agent => agent.verified_task_id === task.id && agent.target
      && sameTarget(binding.target!, agent.target));
    return live ? normalizeAgentStatus(live.status) : "unknown";
  }
  if (binding?.target || ["repair", "reserved", "pending", "unknown"].includes(binding?.state ?? "")) return "unknown";
  if (binding?.state === "dormant" || task.status === "done") return "closed";
  return task.status === "todo" && !task.dispatched_to ? "not_started" : "unknown";
}

export function AgentStatusDot({ status }: { status: AgentStatus }) {
  const label = tr(`agent_status.${status}`);
  return <span className="agent-status-dot" data-status={status} role="img" aria-label={label} title={label} />;
}
