// @vitest-environment jsdom
import { project, flowEvents } from "./fixtures";
import { act, fireEvent, render, renderHook } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ProjectSidebar } from "../ProjectSidebar";
import { useProjectTaskCounts } from "../useProjectTaskCounts";
import type { ProjectCounts } from "../api";

const remote = { ...project, id: "remote-project", machine_id: "remote" };
const row = (id: string, count: number): ProjectCounts => ({ project_id: id, in_progress: count, todo: 0, done: 0, today_done: 0 });
const reads = () => vi.mocked(invoke).mock.calls.filter(([command]) => command === "task_overview");
const emit = (project_id: string) => flowEvents.forEach(callback => callback({ payload: { project_id, task_id: "t", operation_id: "o" } }));

it("shows independent local/remote counts and refreshes changes, failures and real zero without adding a third info row", async () => {
  vi.useFakeTimers();
  let count = 2, failed = false;
  vi.mocked(invoke).mockImplementation(async command => {
    if (command !== "task_overview") throw new Error(`Unexpected ${command}`);
    if (failed) throw new Error("database busy");
    return [row(project.id, count), row(remote.id, 4)];
  });
  function Harness() {
    const counts = useProjectTaskCounts([project, remote]);
    return <ProjectSidebar projects={[project, remote]} inProgressCounts={counts} selectedId={null} onSelect={() => {}} onChanged={() => {}}
      machines={[{ id: "remote", name: "devpc", kind: "ssh", session: "default", disabled: false, profile_id: null, ssh_target: null }]} />;
  }
  const view = render(<Harness />);
  const countFor = (id: string) => document.querySelector(`[data-project-id="${id}"] .project-meta .project-in-progress`)?.textContent;
  expect(countFor(project.id)).toBe("进行中 —");
  await act(async () => {});
  expect(countFor(project.id)).toBe("进行中 2");
  expect(countFor(remote.id)).toBe("进行中 4");
  expect(reads()[0][1]).toMatchObject({ projectIds: expect.arrayContaining([project.id, remote.id]) });
  count = 3;
  await act(async () => emit(project.id));
  expect(countFor(project.id)).toBe("进行中 3");
  count = 1;
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(countFor(project.id)).toBe("进行中 1");
  failed = true;
  await act(async () => { fireEvent(window, new Event("focus")); });
  expect(countFor(project.id)).toBe("进行中 —");
  failed = false; count = 0;
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(countFor(project.id)).toBe("进行中 0");
  view.unmount();
  const stopped = reads().length;
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  expect(reads()).toHaveLength(stopped);
  expect(flowEvents.size).toBe(0);
});

it("coalesces slow reads and refresh events, skips hidden polling and resumes with current counts", async () => {
  vi.useFakeTimers();
  let resolve!: (rows: ProjectCounts[]) => void;
  let count = 7;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(done => { resolve = done; }))
    .mockImplementation(async () => [row(project.id, count)]);
  const view = renderHook(() => useProjectTaskCounts([project]));
  await act(async () => { emit(project.id); fireEvent(window, new Event("focus")); await vi.advanceTimersByTimeAsync(6000); });
  expect(reads()).toHaveLength(1);
  await act(async () => resolve([row(project.id, 1)]));
  expect(reads()).toHaveLength(2);
  expect(view.result.current[project.id]).toBe(7);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  expect(reads()).toHaveLength(2);
  count = 0;
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  await act(async () => { fireEvent(document, new Event("visibilitychange")); });
  expect(view.result.current[project.id]).toBe(0);
});

it("ignores late counts after the visible project scope changes, and queries nothing for an empty list", async () => {
  let resolve!: (rows: ProjectCounts[]) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(done => { resolve = done; }))
    .mockImplementation(async () => [row(remote.id, 8)]);
  const view = renderHook(({ projects }) => useProjectTaskCounts(projects), { initialProps: { projects: [project] } });
  await act(async () => {});
  view.rerender({ projects: [remote] });
  await act(async () => {});
  expect(view.result.current).toEqual({ [remote.id]: 8 });
  await act(async () => resolve([row(project.id, 99)]));
  expect(view.result.current).toEqual({ [remote.id]: 8 });
  view.rerender({ projects: [] });
  const stopped = reads().length;
  await act(async () => { fireEvent(window, new Event("focus")); });
  expect(view.result.current).toEqual({});
  expect(reads()).toHaveLength(stopped);
});
