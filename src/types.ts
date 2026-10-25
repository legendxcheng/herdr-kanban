export type TaskStatus = "todo" | "in_progress" | "done";
export type TaskPriority = "normal" | "high" | "urgent";

export interface Project {
  id: string;
  name: string;
  root_path: string;
  herdr_workspace: string;
  cs_task_id: string | null;
  created_at: string;
  herdr_session: string | null;
  workspace_id: string | null;
  sync_status: string;
  directory_source: string;
  herdr_directory: string | null;
}

export interface HerdrWorkspace { id: string; name: string; directory: string | null }
export interface ProjectGitStatus {
  project_id: string;
  root_path: string;
  state: "branch" | "detached" | "not_repository" | "unavailable";
  head: string | null;
}
export interface WorkspaceOperation { id: string; session: string; name: string; cwd: string; workspace_id: string | null; project_id: string; state: string; error: string }
export interface ProjectSnapshot { session: string; projects: Project[]; workspaces: HerdrWorkspace[]; operations: WorkspaceOperation[] }

export interface Task {
  priority: TaskPriority;
  archived_at: string | null;
  cleanup_required: boolean;
  flow?: { session_id: string | null; operation_id: string | null; kind: string | null; state: string | null; error: string; pane_id: string | null; stage?: string | null; requested_status?: TaskStatus | null } | null;
  binding?: TaskBinding | null;
  id: string;
  project_id: string;
  name: string;
  status: TaskStatus;
  prompt_json: string;
  prompt_snapshot: string | null;
  dispatched_at: string | null;
  dispatched_to: string | null;
  created_at: string;
  updated_at: string;
}

export interface HerdrTarget {
  session: string;
  workspace_id: string;
  pane_id: string;
  terminal_id: string;
  agent: string;
  process_id: string;
  codex_resume_locks?: string[];
  title: string;
  status: string;
}
export interface TaskBinding {
  state: "valid" | "reserved" | "pending" | "unknown" | "repair" | "unbound" | "dormant";
  target: HerdrTarget | null;
  operation: { id: string; prompt: string; created_at: string; target: HerdrTarget } | null;
}

export interface FileEntry {
  rel_path: string;
  abs_path: string;
}

export interface SkillEntry {
  name: string;
  description: string;
  skill_md_path: string;
}

export interface HerdrAgent {
  verified_task_id?: string | null;
  target?: HerdrTarget;
  occupied_by?: string | null;
  name: string;
  status: string;
  pane_id: string;
  title: string;
}

export interface DispatchRecord {
  task_id: string;
  target_workspace: string;
  target_agent: string;
  prompt_snapshot: string;
  dispatched_at: string;
}

// Slim view of a ContextSwitcher task for the project-link picker


// Tiptap mention node stored inside prompt_json
export interface MentionAttrs {
  kind: "file" | "skill";
  label: string;
  ref: string;
}
