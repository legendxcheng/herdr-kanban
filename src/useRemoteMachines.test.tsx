// @vitest-environment jsdom
import { act, renderHook, waitFor, cleanup } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import * as api from "./api";
import { useRemoteMachines } from "./useRemoteMachines";
import type { Machine, Project, ProjectSnapshot, WorkspaceOperation } from "./types";

vi.mock("./api", () => ({ listMachines: vi.fn(), listProjects: vi.fn(), syncProjects: vi.fn(), workspaceOperations: vi.fn(), moveProject: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it("discards an old remote response after explicit CRUD refresh and queues a new sync", async () => {
  const machine = { id: "remote", kind: "ssh", disabled: false } as Machine;
  const old = { id: "removed", machine_id: "remote" } as Project;
  const fresh = { id: "created", machine_id: "remote" } as Project;
  const snapshot = (projects: Project[]) => ({ projects, operations: [], workspaces: [], session: "default" }) as ProjectSnapshot;
  vi.mocked(api.listMachines).mockResolvedValue([machine]);
  vi.mocked(api.workspaceOperations).mockResolvedValue([]);
  vi.mocked(api.listProjects).mockResolvedValue([old]);
  let finish!: (value: ProjectSnapshot) => void;
  vi.mocked(api.syncProjects).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValue(snapshot([fresh]));
  const { result } = renderHook(() => useRemoteMachines());
  await waitFor(() => expect(result.current.projects).toEqual([old]));
  vi.mocked(api.listProjects).mockResolvedValue([fresh]);
  await act(() => result.current.refresh(true));
  expect(result.current.projects).toEqual([fresh]);
  expect(api.syncProjects).toHaveBeenCalledTimes(1);
  await act(async () => { finish(snapshot([old])); });
  await waitFor(() => expect(api.syncProjects).toHaveBeenCalledTimes(2));
  expect(result.current.projects).toEqual([fresh]);
});

it("keeps offline operations visible and accepts a slow sync despite periodic refreshes", async () => {
  const machine = { id: "remote", kind: "ssh", disabled: false } as Machine;
  const pending: WorkspaceOperation = { id: "unknown-create", session: "default", name: "Pending", cwd: "C:\\remote", workspace_id: null, project_id: "pending-project", state: "unknown", error: "" };
  vi.mocked(api.listMachines).mockResolvedValue([machine]);
  vi.mocked(api.listProjects).mockResolvedValue([]);
  vi.mocked(api.workspaceOperations).mockResolvedValue([pending]);
  let finish!: (value: ProjectSnapshot) => void;
  vi.mocked(api.syncProjects).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
    .mockImplementation(() => new Promise(() => {}));
  const { result } = renderHook(() => useRemoteMachines());
  await waitFor(() => expect(result.current.operations).toEqual([pending]));
  await act(() => result.current.refresh());
  await act(() => result.current.refresh());
  expect(api.syncProjects).toHaveBeenCalledTimes(1);
  const project = { id: "slow-result", machine_id: "remote" } as Project;
  await act(async () => finish({ projects: [project], operations: [], workspaces: [], session: "default" } as ProjectSnapshot));
  expect(result.current.projects).toEqual([project]);
  expect(result.current.operations).toEqual([]);
});
