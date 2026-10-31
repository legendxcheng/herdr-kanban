/** Local UI preferences keyed by the stable Project ID, never a Workspace name. */
const keyPrefix = "kanban.workspaceTaskSelection.v1:";
const sessionSelections = new Map<string, string | null>();
const unsavedSelections = new Set<string>();

export function readWorkspaceTaskSelection(projectId: string): string | null {
  // A failed write must not let an older stored value replace this session's choice.
  if (unsavedSelections.has(projectId)) return sessionSelections.get(projectId) ?? null;
  try {
    const value: unknown = JSON.parse(localStorage.getItem(keyPrefix + projectId) ?? "null");
    const taskId = typeof value === "string" && value.trim() ? value : null;
    sessionSelections.set(projectId, taskId);
    return taskId;
  } catch {
    return sessionSelections.get(projectId) ?? null;
  }
}

export function rememberWorkspaceTaskSelection(projectId: string, taskId: string | null): void {
  sessionSelections.set(projectId, taskId);
  try {
    localStorage.setItem(keyPrefix + projectId, JSON.stringify(taskId));
    unsavedSelections.delete(projectId);
  } catch {
    // Switching Workspaces still works when WebView preferences cannot be written.
    unsavedSelections.add(projectId);
  }
}
