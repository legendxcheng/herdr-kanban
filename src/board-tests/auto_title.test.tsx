// @vitest-environment jsdom
import { board, flowEvents, project, renderBoard } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TaskCard } from "../TaskCard";
import { setLocale } from "../i18n";

function titleCard() {
  const onChanged = vi.fn(), onSelect = vi.fn();
  render(<TaskCard task={board.storedTask} agentStatus="unknown" onChanged={onChanged} onSelect={onSelect}
    onRemoved={vi.fn()} onEditTask={vi.fn()} onDispatchTask={vi.fn()} onComplete={vi.fn()} onStart={vi.fn()} />);
  return { onChanged, onSelect };
}

it.each(["todo", "in_progress", "done"] as const)("%s 标题失败可重试，重复点击只提交一次且不触发任务派发", async status => {
  board.storedTask.status = status;
  board.storedTask.title_generation = { id: "old-request", state: "failed", attempts: 4, next_attempt_ms: 0, error: "machine.timeout" };
  let resolve!: () => void;
  vi.mocked(invoke).mockImplementation(() => new Promise<void>(done => { resolve = done; }));
  const { onChanged, onSelect } = titleCard();
  const button = screen.getByRole("button", { name: "重试标题" }) as HTMLButtonElement;
  expect(button.title).toContain("标题生成失败");
  fireEvent.click(button); fireEvent.click(button);
  expect(button.disabled).toBe(true);
  expect(vi.mocked(invoke).mock.calls).toEqual([["retry_task_title", { id: board.storedTask.id, requestId: "old-request" }]]);
  await act(async () => resolve());
  expect(onChanged).toHaveBeenCalledOnce();
  expect(onSelect).not.toHaveBeenCalled();
});

it("等待自动重试时保留摘要并显示次数，不提供重复请求入口", async () => {
  board.storedTask.title_generation = { id: "request", state: "retry_wait", attempts: 2, next_attempt_ms: Date.now() + 30_000, error: "title.timeout" };
  titleCard();
  expect(screen.getByText(board.storedTask.name)).toBeTruthy();
  expect(screen.getByRole("status", { name: /等待第 3\/4 次标题生成/ })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "重试标题" })).toBeNull();
  await act(async () => setLocale("en-US"));
  expect(screen.getByRole("status", { name: /Waiting for title attempt 3\/4/ })).toBeTruthy();
  setLocale("zh-CN");
});

it("标题事件刷新失败状态和新名称，不影响已打开的任务编辑草稿", async () => {
  board.storedTask.title_generation = { id: "old", state: "failed", attempts: 4, next_attempt_ms: 0, error: "title.timeout" };
  await renderBoard();
  fireEvent.click(screen.getByRole("button", { name: "编辑" }));
  const editor = screen.getByRole("complementary", { name: "编辑任务" });
  fireEvent.change(within(editor).getByRole("textbox", { name: "标题" }), { target: { value: "未保存的标题" } });
  board.storedTask.name = "重新生成的标题";
  board.storedTask.title_generation = { id: "new", state: "succeeded", attempts: 1, next_attempt_ms: 0, error: "" };
  await act(async () => { for (const callback of flowEvents) callback({ payload: { project_id: project.id, task_id: board.storedTask.id, operation_id: "title" } }); });
  expect(await screen.findByText("重新生成的标题")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "重试标题" })).toBeNull();
  expect((within(editor).getByRole("textbox", { name: "标题" }) as HTMLInputElement).value).toBe("未保存的标题");
});

it("新建只输入正文即可保存，空白不能创建且焦点在正文", async () => {
  await renderBoard();
  fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).queryByRole("textbox", { name: "标题" })).toBeNull();
  const prompt = within(dialog).getByRole("textbox", { name: "提示词" });
  await waitFor(() => expect(document.activeElement).toBe(prompt));
  const add = within(dialog).getByRole("button", { name: "添加任务" }) as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  fireEvent.paste(prompt, { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "修复登录失败" : "" } });
  expect(add.disabled).toBe(false);
  fireEvent.click(add);
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  const call = vi.mocked(invoke).mock.calls.find(([command]) => command === "create_task")!;
  expect(call[1]).not.toHaveProperty("name");
  expect(call[1]).toMatchObject({ priority: "normal" });
});

it("旧编辑表单只改优先级时保留后台新标题，主动改名则显式提交", async () => {
  await renderBoard();
  fireEvent.click(screen.getByRole("button", { name: "编辑" }));
  const dialog = screen.getByRole("complementary", { name: "编辑任务" });
  board.storedTask.name = "后台生成的新标题";
  await act(async () => { for (const callback of flowEvents) callback({ payload: {
    project_id: project.id, task_id: board.storedTask.id, operation_id: "title-result",
  } }); });
  fireEvent.change(within(dialog).getByRole("combobox", { name: "优先级" }), { target: { value: "urgent" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(within(dialog).getByRole("status").textContent).toBe("已保存"));
  expect(await screen.findByText("后台生成的新标题")).toBeTruthy();
  expect(vi.mocked(invoke).mock.calls.find(([command]) => command === "update_task")![1]).toMatchObject({ name: undefined });
  fireEvent.click(screen.getByRole("button", { name: "编辑" }));
  fireEvent.change(screen.getByRole("textbox", { name: "标题" }), { target: { value: "用户指定名称" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(within(dialog).getByRole("status").textContent).toBe("已保存"));
  expect(await screen.findByText("用户指定名称")).toBeTruthy();
});
