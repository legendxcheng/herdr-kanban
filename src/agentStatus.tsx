import type { HerdrAgent, HerdrTarget, Task } from "./types";
import { tr, useLocale } from "./i18n";
import type { AgentRead } from "./useAgentStatus";

export type AgentStatus = "working" | "blocked" | "done" | "idle" | "unknown" | "not_started" | "closed";
export function normalizeAgentStatus(status: string): AgentStatus {
  return ["working", "blocked", "done", "idle"].includes(status) ? status as AgentStatus : "unknown";
}
function hasUnresolvedOperation(task: Task, busy: boolean) {
  return busy || task.cleanup_required || !!task.binding?.operation || ["pending", "unknown"].includes(task.flow?.state ?? "");
}
function sameTarget(a: HerdrTarget, b: HerdrTarget) {
  return (a.machine_id ?? "local") === (b.machine_id ?? "local") && a.session === b.session && a.workspace_id === b.workspace_id && a.pane_id === b.pane_id
    && a.terminal_id === b.terminal_id && a.agent === b.agent && a.process_id === b.process_id;
}
export function taskAgentStatus(task: Task, agents: HerdrAgent[], busy = false): AgentStatus {
  if (hasUnresolvedOperation(task, busy)) return "unknown";
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

export interface AgentPresentation {
  status: AgentStatus;
  note?: "loading" | "updating" | "query_failed" | "stale" | "binding_mismatch" | "agent_missing" | "unrecognized" | "pending";
  previous?: AgentStatus;
}

export function taskAgentPresentation(task: Task, agents: HerdrAgent[], busy: boolean, read: AgentRead | undefined, now: number): AgentPresentation {
  const status = taskAgentStatus(task, agents, busy);
  if (status === "not_started" || status === "closed") return { status };
  if (hasUnresolvedOperation(task, busy)) {
    return { status: "unknown", note: "pending" };
  }
  if (task.binding?.state !== "valid" || !task.binding.target) return { status: "unknown", note: "binding_mismatch" };
  if (read?.failed || read?.paused || (read?.observedAt !== undefined && now - read.observedAt > 15_000)) {
    return { status: "unknown", note: read.failed ? "query_failed" : "stale", previous: status !== "unknown" ? status : undefined };
  }
  if (read?.observedAt === undefined) return { status: "unknown", note: "loading" };
  const failure = read.failures?.find(failure => failure.pane_id === task.binding!.target!.pane_id);
  if (failure) {
    if (failure.error === "binding.identity_changed") return { status: "unknown", note: "binding_mismatch" };
    return { status: "unknown", note: "query_failed", previous: status !== "unknown" ? status : undefined };
  }
  if (status === "unknown") {
    const live = agents.find(agent => agent.pane_id === task.binding!.target!.pane_id);
    return { status, note: !live ? "agent_missing" : !live.target || !sameTarget(task.binding.target, live.target) || live.verified_task_id !== task.id ? "binding_mismatch" : "unrecognized" };
  }
  return { status, note: read.updating ? "updating" : undefined };
}

export function AgentStatusDot({ status, presentation }: { status: AgentStatus; presentation?: AgentPresentation }) {
  useLocale();
  const note = presentation?.note;
  const label = [tr(`agent_status.${status}`), note && tr(`agent_observation.${note}`),
    presentation?.previous && tr("agent_observation.previous", { status: tr(`agent_status.${presentation.previous}`) })].filter(Boolean).join(" · ");
  return <span className="agent-status-indicator" title={label}>
    <span className="agent-status-dot" data-status={status} role="img" aria-label={label} title={label} />
    {note && note !== "updating" && <span className="agent-status-note" data-note={note}>{tr(`agent_observation.${note}`)}</span>}
  </span>;
}
