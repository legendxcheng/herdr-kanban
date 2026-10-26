// @vitest-environment jsdom
import { board, project, renderBoard } from "./fixtures";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { KanbanBoard } from "../KanbanBoard";
import type { Task } from "../types";

function deferredTasks() {
  let resolve!: (tasks: Task[]) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<Task[]>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function delayReads(request: ReturnType<typeof deferredTasks>) {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) =>
    command === "list_tasks" ? request.promise : base(command, args, options));
}

function boardElements() {
  return [".workspace-bar", ".board-toolbar", ".kanban-columns"].map(selector => {
    const element = document.querySelector(selector);
    expect(element).not.toBeNull();
    return element!;
  });
}

function expectBoardRetained(elements: Element[]) {
  expect(document.querySelector(".board-loading")).toBeNull();
  expect(boardElements()).toEqual(elements);
  elements.forEach(element => expect(element.isConnected).toBe(true));
}

it("空 Workspace 首次加载后，慢速轮询、焦点刷新和失败恢复均保留看板节点", async () => {
  vi.useFakeTimers();
  board.deleted = true;
  await act(async () => { render(<KanbanBoard project={project} />); });
  const elements = boardElements();
  const request = deferredTasks();
  delayReads(request);
  const before = vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "list_tasks").length;
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "list_tasks")).toHaveLength(before + 1);
  expectBoardRetained(elements);
  await act(async () => { await vi.advanceTimersByTimeAsync(4000); });
  expectBoardRetained(elements);
  await act(async () => { request.resolve([]); });
  expectBoardRetained(elements);

  const failed = deferredTasks();
  delayReads(failed);
  await act(async () => { fireEvent(window, new Event("focus")); });
  expectBoardRetained(elements);
  await act(async () => { failed.reject(new Error("temporary read failure")); });
  expectBoardRetained(elements);

  const recovered = deferredTasks();
  delayReads(recovered);
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expectBoardRetained(elements);
  await act(async () => { recovered.resolve([board.storedTask]); });
  expectBoardRetained(elements);
  expect(screen.getByText(board.storedTask.name, { selector: ".task-name" })).toBeTruthy();
});

it("删除最后一张卡片时，等待刷新也保留空看板和新建入口", async () => {
  await renderBoard();
  vi.spyOn(window, "confirm").mockReturnValue(true);
  const elements = boardElements();
  const request = deferredTasks();
  delayReads(request);
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "删除" })); });
  expect(board.deleted).toBe(true);
  expectBoardRetained(elements);
  expect(screen.getByRole("button", { name: /新建任务/ })).toBeTruthy();
  await act(async () => { request.resolve([]); });
  expectBoardRetained(elements);
});

it("按 Project key 切换时保留首次加载，并隔离旧项目的迟到结果", async () => {
  const oldRead = deferredTasks();
  delayReads(oldRead);
  const view = render(<KanbanBoard key={project.id} project={project} />);
  expect(document.querySelector(".board-loading")).not.toBeNull();
  const nextProject = { ...project, id: "next-project", name: "新项目" };
  const nextRead = deferredTasks();
  delayReads(nextRead);
  await act(async () => { view.rerender(<KanbanBoard key={nextProject.id} project={nextProject} />); });
  await act(async () => { oldRead.resolve([board.storedTask]); });
  expect(document.querySelector(".board-loading")).not.toBeNull();
  expect(screen.queryByText(board.storedTask.name)).toBeNull();
  await act(async () => { nextRead.resolve([]); });
  expect(document.querySelector(".board-loading")).toBeNull();
  expect(screen.getByRole("heading", { name: nextProject.name })).toBeTruthy();
  boardElements();
});

it("首次查询失败后结束加载，重试期间不反复隐藏看板", async () => {
  vi.useFakeTimers();
  const initial = deferredTasks();
  delayReads(initial);
  await act(async () => { render(<KanbanBoard project={project} />); });
  expect(document.querySelector(".board-loading")).not.toBeNull();
  await act(async () => { initial.reject(new Error("first read failed")); });
  const elements = boardElements();
  const retry = deferredTasks();
  delayReads(retry);
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expectBoardRetained(elements);
  await act(async () => { retry.resolve([]); });
  expectBoardRetained(elements);
});
