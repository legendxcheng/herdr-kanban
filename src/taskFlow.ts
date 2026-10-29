import type { Task, TaskStatus } from "./types";

export function flowPending(task: Task) {
  return task.flow?.state === "pending";
}

export function flowUnresolved(task: Task) {
  return flowPending(task) || task.flow?.state === "unknown" || !!task.binding?.operation;
}

export function canResetTask(task: Task) {
  return !task.archived_at && !flowPending(task) && task.binding?.state !== "pending"
    && (["failed", "unknown"].includes(task.flow?.state ?? "") || task.binding?.state === "unknown");
}

export function taskColumn(task: Task): TaskStatus {
  return task.flow?.kind === "start" && flowUnresolved(task)
    ? task.flow.requested_status ?? task.status : task.status;
}

export function preparationLabel(task: Task): string | null {
  if (task.flow?.kind !== "start") return null;
  if (task.flow.state === "unknown") return "flow.verify_required";
  if (!flowPending(task)) return null;
  switch (task.flow.stage) {
    case "reading_prompt": return "flow.reading_prompt";
    case "starting_agent": return "flow.starting_agent";
    case "checking_model": return "flow.checking_model";
    case "dispatching": return "flow.dispatching";
    default: return "flow.preparing";
  }
}
