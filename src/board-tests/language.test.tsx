// @vitest-environment jsdom
import { bindTask, board, project, renderBoard } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { LanguageSelect } from "../LanguageSelect";
import { setLocale, localeStorageKey, tr } from "../i18n";
import { notify, ToastContainer } from "../Toast";
import { TaskOverviewDialog } from "../TaskOverviewDialog";

beforeEach(() => { document.documentElement.lang = "zh-CN"; localStorage.clear(); });
afterEach(() => { localStorage.clear(); document.documentElement.lang = "zh-CN"; });

it("switches the board using the two-option selector and persists the choice", async () => {
  render(<LanguageSelect />);
  await renderBoard();
  expect(screen.getAllByRole("option").map(option => option.textContent)).toEqual(["简体中文", "English"]);
  fireEvent.change(screen.getByRole("combobox", { name: "语言" }), { target: { value: "en-US" } });
  expect(screen.getByText("Task board")).toBeTruthy();
  expect(screen.getByText("To do")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Dispatch" })).toBeTruthy();
  expect(screen.getByText(board.storedTask.name)).toBeTruthy();
  expect(document.documentElement.lang).toBe("en-US");
  expect(localStorage.getItem(localeStorageKey)).toBe("en-US");
});

it("updates an open editor and existing error while retaining the draft, priority and undo history", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "create_task"
    ? Promise.reject(new Error("image.storage: diagnostic")) : base(command, args, options));
  await renderBoard();
  fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
  const dialog = screen.getByRole("dialog");
  const prompt = within(dialog).getByRole("textbox", { name: "提示词" });
  expect(prompt.querySelector("p")?.getAttribute("data-placeholder")).toBe(tr("task.prompt_placeholder"));
  act(() => setLocale("en-US"));
  expect(prompt.querySelector("p")?.getAttribute("data-placeholder")).toBe(tr("task.prompt_placeholder"));
  fireEvent.paste(prompt, { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "中文 draft $literal" : "" } });
  fireEvent.change(within(dialog).getByRole("combobox", { name: "Priority" }), { target: { value: "urgent" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Add task" }));
  await screen.findByRole("alert");
  expect(screen.getByRole("alert").textContent).toContain("Unable to read or write");
  act(() => setLocale("zh-CN"));
  expect(screen.getByRole("dialog", { name: "新建任务" })).toBe(dialog);
  expect(within(dialog).getByRole("textbox", { name: "提示词" })).toBe(prompt);
  expect(prompt.textContent).toBe("中文 draft $literal");
  expect((within(dialog).getByRole("combobox") as HTMLSelectElement).value).toBe("urgent");
  expect(screen.getByRole("alert").textContent).toBe("图片读写失败，请重试: diagnostic");
  fireEvent.keyDown(prompt, { key: "z", code: "KeyZ", ctrlKey: true });
  await waitFor(() => expect(prompt.textContent).toBe(""));
  const calls = vi.mocked(invoke).mock.calls.filter(([command]) => command === "create_task");
  expect(calls).toHaveLength(1);
  expect(calls[0][1]).toMatchObject({ locale: "en-US" });
});

it("keeps the selected native terminal connection across language changes", async () => {
  bindTask();
  await renderBoard("in_progress");
  await screen.findByText("Herdr 已打开");
  const host = document.querySelector(".terminal-host");
  const opened = vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_open");
  act(() => setLocale("en-US"));
  expect(screen.getByText("Herdr open")).toBeTruthy();
  expect(document.querySelector(".terminal-host")).toBe(host);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_open")).toEqual(opened);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "terminal_close")).toHaveLength(0);
  expect(document.querySelector(".task-card.selected .task-name")?.textContent).toBe(board.storedTask.name);
});

it("translates an already visible notification while retaining diagnostic details", () => {
  render(<ToastContainer />);
  act(() => notify({ prefix: "workspace.offline", error: "herdr.timeout: external detail" }, "error"));
  expect(screen.getByRole("status").textContent).toContain("同步失败");
  act(() => setLocale("en-US"));
  expect(screen.getByRole("status").textContent).toContain("Sync failed; local projects and tasks are retained. Herdr timed out: external detail");
});

it("reformats the open overview date without changing its local-day query or remounting the dialog", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "task_overview"
    ? Promise.resolve([{ project_id: project.id, todo: 1, in_progress: 0, done: 0, today_done: 0 }]) : base(command, args, options));
  render(<TaskOverviewDialog projects={[project]} onClose={() => {}} onSelect={() => {}} />);
  await screen.findByRole("table");
  const dialog = screen.getByRole("dialog");
  const calls = vi.mocked(invoke).mock.calls.filter(([command]) => command === "task_overview");
  act(() => setLocale("en-US"));
  expect(screen.getByRole("dialog", { name: "Task overview" })).toBe(dialog);
  expect(document.querySelector(".overview-footer")?.textContent).toContain(new Date().toLocaleDateString("en-US"));
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "task_overview")).toEqual(calls);
});
