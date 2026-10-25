// @vitest-environment jsdom
import { project, flowEvents } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";
import { TaskOverviewDialog, localDay } from "../TaskOverviewDialog";
import type { ProjectCounts } from "../api";

const other = { ...project, id: "p2", name: "第二项目" };
const rows: ProjectCounts[] = [
  { project_id: project.id, todo: 8, in_progress: 2, done: 12, today_done: 3 },
  { project_id: other.id, todo: 4, in_progress: 1, done: 5, today_done: 1 },
];
function mockOverview(read: () => Promise<ProjectCounts[]> = async () => rows) {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "task_overview") return read() as ReturnType<typeof invoke>;
    if (command === "list_projects") return Promise.resolve([project, other]);
    if (command === "sync_projects") return Promise.resolve({ session: "default", projects: [project, other], operations: [], workspaces: [] });
    return base(command, args, options);
  });
}
const calls = () => vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "task_overview");

it("overview is opt-in, summarizes all visible projects and selecting a row switches boards", async () => {
  mockOverview(); render(<App />);
  await screen.findByRole("button", { name: /^第二项目/ });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(calls()).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "任务概况" }));
  const dialog = await screen.findByRole("dialog", { name: "任务概况" });
  const table = await within(dialog).findByRole("table");
  expect(within(table).getByRole("row", { name: /测试项目/ }).textContent).toBe("测试项目82123");
  expect(Array.from(dialog.querySelectorAll(".overview-totals strong")).map(el => el.textContent)).toEqual(["12", "3", "4"]);
  expect(calls()[0][1]).toMatchObject({ projectIds: [project.id, other.id] });
  fireEvent.click(within(dialog).getByRole("button", { name: other.name }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(document.querySelector('.project-row[aria-current="page"]')?.textContent).toContain(other.name);
});

it("read failure offers retry rather than displaying zero counts", async () => {
  let failed = true;
  mockOverview(async () => { if (failed) throw new Error("database busy"); return rows; });
  render(<TaskOverviewDialog projects={[project, other]} onClose={() => {}} onSelect={() => {}} />);
  await screen.findByRole("alert");
  expect(screen.queryByRole("table")).toBeNull();
  expect(document.querySelector(".overview-totals")).toBeNull();
  failed = false; fireEvent.click(screen.getByRole("button", { name: "重试" }));
  await screen.findByRole("table");
  expect(screen.queryByRole("alert")).toBeNull();
});

it("late response from a previous project scope cannot replace the current snapshot", async () => {
  let resolve!: (rows: ProjectCounts[]) => void;
  mockOverview(() => new Promise(done => { resolve = done; }));
  const view = render(<TaskOverviewDialog projects={[project]} onClose={() => {}} onSelect={() => {}} />);
  const oldResolve = resolve;
  view.rerender(<TaskOverviewDialog projects={[other]} onClose={() => {}} onSelect={() => {}} />);
  await act(async () => { resolve([rows[1]]); });
  await screen.findByRole("table");
  await act(async () => { oldResolve([rows[0]]); });
  expect(screen.queryByRole("button", { name: project.name })).toBeNull();
  expect(screen.getByRole("button", { name: other.name })).toBeTruthy();
  expect(document.querySelector(".overview-totals strong")?.textContent).toBe("4");
});

it("midnight recomputes local boundaries and foreground focus reconciles background changes", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 25, 23, 59, 59));
  mockOverview(async () => [{ ...rows[0], today_done: new Date().getDate() === 25 ? 3 : 0 }]);
  const view = render(<TaskOverviewDialog projects={[project]} onClose={() => {}} onSelect={() => {}} />);
  await act(async () => { await Promise.resolve(); });
  expect(document.querySelector(".overview-totals > div:last-child strong")?.textContent).toBe("3");
  await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
  expect(calls()[calls().length - 1]?.[1]).toMatchObject({ dayStart: new Date(2026, 9, 26).toISOString(), dayEnd: new Date(2026, 9, 27).toISOString() });
  expect(document.querySelector(".overview-totals > div:last-child strong")?.textContent).toBe("0");
  const before = calls().length;
  await act(async () => { fireEvent(window, new Event("focus")); });
  expect(calls().length).toBeGreaterThan(before);
  view.unmount();
  const closedCalls = calls().length;
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(calls()).toHaveLength(closedCalls);
  expect(flowEvents.size).toBe(0);
});

it("Escape, backdrop and close button dismiss; keyboard focus is contained and restored", async () => {
  mockOverview(); render(<App />);
  const opener = screen.getByRole("button", { name: "任务概况" });
  opener.focus(); fireEvent.click(opener);
  let dialog = screen.getByRole("dialog");
  await within(dialog).findByRole("table");
  const close = within(dialog).getByRole("button", { name: "关闭" });
  expect(document.activeElement).toBe(close);
  fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
  expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "刷新" }));
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull(); expect(document.activeElement).toBe(opener);
  fireEvent.click(opener); dialog = screen.getByRole("dialog");
  fireEvent.mouseDown(dialog.parentElement!);
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(opener);
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "关闭" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("empty scope has an explicit empty state and localDay follows calendar midnight", async () => {
  mockOverview(async () => []);
  render(<TaskOverviewDialog projects={[]} onClose={() => {}} onSelect={() => {}} />);
  await screen.findByText(/暂无 Workspace/);
  expect(screen.queryByRole("table")).toBeNull();
  const day = localDay(new Date(2026, 9, 25, 15, 42));
  expect(day.start).toBe(new Date(2026, 9, 25).toISOString());
  expect(day.end).toBe(new Date(2026, 9, 26).toISOString());
});
