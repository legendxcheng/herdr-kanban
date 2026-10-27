// @vitest-environment jsdom
import { board, flowEvents, project, renderBoard } from "./fixtures";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

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
