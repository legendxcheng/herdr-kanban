// @vitest-environment jsdom
import { bindTask, board, column, dropOn, flowEvents, project, renderBoard, secondTask, sendPrompt } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "../App";

const panel = () => screen.getByRole("complementary", { name: "编辑任务" });
const title = () => within(panel()).getByRole("textbox", { name: "标题" }) as HTMLInputElement;
const choose = (name: string) => fireEvent.click(screen.getByRole("button", { name: `查看并编辑：${name}` }));
const changeTitle = (value: string) => fireEvent.change(title(), { target: { value } });
function twoTasks() { board.extraTasks = [{ ...board.storedTask, id: "todo-b", name: "待办 B" }]; }

it("Todo 点击和键盘选择使用右侧编辑，保存保留面板和选择，刷新不覆盖草稿", async () => {
  await renderBoard();
  fireEvent.keyDown(screen.getByRole("button", { name: `查看并编辑：${board.storedTask.name}` }), { key: "Enter" });
  const originalPanel = panel();
  expect(screen.queryByRole("dialog")).toBeNull();
  changeTitle("新的任务名称");
  fireEvent.paste(within(panel()).getByRole("textbox", { name: "提示词" }), {
    clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "新增正文" : "" },
  });
  await act(async () => { for (const callback of flowEvents) callback({ payload: { project_id: project.id, task_id: board.storedTask.id, operation_id: "refresh" } }); });
  expect(panel()).toBe(originalPanel);
  expect(title().value).toBe("新的任务名称");
  expect(within(panel()).getByRole("textbox", { name: "提示词" }).textContent).toContain("新增正文");
  fireEvent.click(within(panel()).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(within(panel()).getByRole("status").textContent).toBe("已保存"));
  expect(panel()).toBe(originalPanel);
  expect(screen.getByRole("button", { name: "查看并编辑：新的任务名称" }).getAttribute("aria-pressed")).toBe("true");
  expect(board.storedTask.prompt_json).toContain("新增正文");
  expect(sendPrompt).not.toHaveBeenCalled();
});

it("切换 Todo 可取消、放弃或保存后继续，保存失败保留原任务和草稿", async () => {
  twoTasks(); await renderBoard(); choose(board.storedTask.name); changeTitle("未保存标题");
  choose("待办 B");
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect(title().value).toBe("未保存标题");
  choose("待办 B");
  fireEvent.click(await screen.findByRole("button", { name: "放弃修改" }));
  await waitFor(() => expect(title().value).toBe("待办 B"));
  choose(board.storedTask.name);
  await waitFor(() => expect(title().value).toBe(board.storedTask.name));
  changeTitle("保存后切换");
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "update_task" ? Promise.reject("保存失败") : base(command, args, options));
  choose("待办 B");
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  await waitFor(() => expect(within(screen.getByRole("dialog")).getByRole("alert").textContent).toBe("保存失败"));
  expect(title().value).toBe("保存后切换");
  vi.mocked(invoke).mockImplementation(base);
  fireEvent.click(screen.getByRole("button", { name: "保存并继续" }));
  await waitFor(() => expect(title().value).toBe("待办 B"));
  expect(board.storedTask.name).toBe("保存后切换");
});

it("Todo 预览关闭原生查看连接，返回进行中任务重新连接且不派发或完成任务", async () => {
  bindTask(); board.extraTasks = [secondTask()]; await renderBoard();
  await screen.findByText("Herdr 已打开");
  const connection = vi.mocked(invoke).mock.calls.find(([name]) => name === "terminal_open")![1] as { connectionId: string };
  choose(board.storedTask.name);
  expect(panel()).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("terminal_close", { connectionId: connection.connectionId });
  fireEvent.click(screen.getByRole("button", { name: "查看终端：任务 B" }));
  await screen.findByText("Herdr 已打开");
  expect(screen.queryByRole("complementary", { name: "编辑任务" })).toBeNull();
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "terminal_open")).toHaveLength(2);
  expect(vi.mocked(invoke).mock.calls.some(([name]) => ["complete_task", "start_task", "dispatch_to_herdr"].includes(name))).toBe(false);
});

it("拖入开发中先保存编辑稿，取消不会启动，开始后右侧恢复终端", async () => {
  await renderBoard(); choose(board.storedTask.name); changeTitle("准备启动");
  dropOn(column("in_progress"));
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect(sendPrompt).not.toHaveBeenCalled();
  dropOn(column("in_progress"));
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  await waitFor(() => expect(sendPrompt).toHaveBeenCalledTimes(1));
  expect(board.storedTask.name).toBe("准备启动");
  await screen.findByText("Herdr 已打开");
  const commands = vi.mocked(invoke).mock.calls.map(([name]) => name);
  expect(commands.indexOf("update_task")).toBeLessThan(commands.indexOf("start_task"));
});

it("保存中不切走或重复保存，后续切换不误报未保存", async () => {
  twoTasks(); await renderBoard(); choose(board.storedTask.name); changeTitle("保存中的名称");
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: () => void;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "update_task"
    ? new Promise(resolve => { finish = () => { void Promise.resolve(base(command, args, options)).then(resolve); }; }) : base(command, args, options));
  fireEvent.click(within(panel()).getByRole("button", { name: "保存" }));
  choose("待办 B");
  await act(async () => {});
  expect(title().value).toBe("保存中的名称");
  expect(screen.queryByRole("dialog")).toBeNull();
  finish(); await waitFor(() => expect(within(panel()).getByRole("status").textContent).toBe("已保存"));
  choose("待办 B"); await waitFor(() => expect(title().value).toBe("待办 B"));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("切换 Workspace 前保护草稿，确认后释放面板且不遗留 inert", async () => {
  const other = { ...project, id: "project-2", name: "第二项目" };
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "list_projects") return Promise.resolve([project, other]);
    if (command === "list_tasks" && (args as { projectId: string }).projectId === other.id) return Promise.resolve([]);
    return base(command, args, options);
  });
  const root = document.createElement("div"); root.id = "root"; document.body.append(root);
  const app = render(<App />, { container: root });
  await screen.findByText(board.storedTask.name, { selector: ".task-name" });
  choose(board.storedTask.name); changeTitle("跨项目草稿");
  fireEvent.click(screen.getByRole("button", { name: /^第二项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect(title().value).toBe("跨项目草稿");
  fireEvent.click(screen.getByRole("button", { name: /^第二项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "保存并继续" }));
  await waitFor(() => expect(screen.getByRole("heading", { name: "第二项目" })).toBeTruthy());
  expect(board.storedTask.name).toBe("跨项目草稿");
  expect(root.inert).toBe(false);
  expect(screen.queryByRole("complementary", { name: "编辑任务" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /^测试项目/ }));
  await screen.findByRole("complementary", { name: "编辑任务" });
  expect(title().value).toBe("跨项目草稿");
  app.unmount(); root.remove();
});

it("删除选中的 Todo 释放面板并回到已有进行中任务", async () => {
  bindTask(); board.extraTasks = [secondTask()]; await renderBoard();
  await screen.findByText("Herdr 已打开"); choose(board.storedTask.name);
  vi.spyOn(window, "confirm").mockReturnValue(true);
  fireEvent.click(within(column("todo")).getByRole("button", { name: "删除" }));
  await screen.findByText("Herdr 已打开");
  expect(screen.queryByRole("complementary", { name: "编辑任务" })).toBeNull();
  expect(screen.getByRole("button", { name: "查看终端：任务 B" }).getAttribute("aria-pressed")).toBe("true");
});

it("图片导入期间保护当前面板，多次保存复用图片草稿，离开后释放", async () => {
  twoTasks();
  const base = vi.mocked(invoke).getMockImplementation()!;
  let finish!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "open_image_draft") return Promise.resolve("todo-image-draft");
    if (command === "import_prompt_image") return new Promise(resolve => { finish = resolve; });
    if (command === "preview_prompt_image") return Promise.resolve("data:image/png;base64,AA==");
    if (command === "close_image_draft") return Promise.resolve();
    return base(command, args, options);
  });
  await renderBoard(); choose(board.storedTask.name);
  fireEvent.paste(within(panel()).getByRole("textbox", { name: "提示词" }), {
    clipboardData: { files: [new File(["PNG"], "截图.png", { type: "image/png" })], items: [], getData: () => "" },
  });
  await waitFor(() => expect(finish).toBeTypeOf("function"));
  choose("待办 B"); await act(async () => {});
  expect(title().value).toBe(board.storedTask.name);
  finish({ id: "todo-image", name: "截图.png", mime: "image/png", bytes: 80, width: 2, height: 2 });
  await screen.findByRole("img", { name: "截图.png" });
  fireEvent.click(within(panel()).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(within(panel()).getByRole("status").textContent).toBe("已保存"));
  changeTitle("图文任务");
  fireEvent.click(within(panel()).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(within(panel()).getByRole("status").textContent).toBe("已保存"));
  const saves = vi.mocked(invoke).mock.calls.filter(([command]) => command === "update_task");
  expect(saves).toHaveLength(2);
  for (const [, args] of saves) expect(args).toMatchObject({ draftId: "todo-image-draft", promptJson: expect.stringContaining("todo-image") });
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "open_image_draft")).toHaveLength(1);
  expect(invoke).not.toHaveBeenCalledWith("close_image_draft", { draftId: "todo-image-draft" });
  choose("待办 B");
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("close_image_draft", { draftId: "todo-image-draft" }));
});

it("从任务概况切换项目先关闭概况，再确认草稿，取消后根节点仍可操作", async () => {
  const other = { ...project, id: "project-2", name: "第二项目" };
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "list_projects"
    ? Promise.resolve([project, other]) : base(command, args, options));
  const root = document.createElement("div"); root.id = "root"; document.body.append(root);
  const app = render(<App />, { container: root });
  await screen.findByText(board.storedTask.name, { selector: ".task-name" });
  choose(board.storedTask.name); changeTitle("保留概况草稿");
  fireEvent.click(screen.getByRole("button", { name: "任务概况" }));
  const overview = await screen.findByRole("dialog", { name: "任务概况" });
  fireEvent.click(await within(overview).findByRole("button", { name: "第二项目" }));
  expect(screen.queryByRole("dialog", { name: "任务概况" })).toBeNull();
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect(root.inert).toBe(false);
  expect(title().value).toBe("保留概况草稿");
  app.unmount(); root.remove();
});
