// @vitest-environment jsdom
import { board, project, renderBoard } from "./fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TaskFormDialog } from "../TaskFormDialog";

const catalog = { models: [{ id: "test-model", name: "Test Model", efforts: ["low", "high"] }], source: "cli_catalog", cached: false, error: null };

it.each([false, true])("SSH 目录查询失败仍可保存已有配置（缓存：%s）", async cached => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => {
    if (cmd === "last_codex_settings") return Promise.resolve({ model: "test-model", effort: "high" });
    if (cmd === "codex_model_catalog") return cached
      ? Promise.resolve({ ...catalog, cached: true, error: "model.catalog_unavailable" })
      : Promise.reject("model.catalog_unavailable");
    return base(cmd, args, options);
  });
  render(<TaskFormDialog project={{ ...project, machine_id: "remote" }} onClose={() => {}} onSaved={() => {}} />);
  await screen.findByText(/可保留当前选择并启动/);
  expect((screen.getByRole("combobox", { name: "AI 模型" }) as HTMLSelectElement).value).toBe("test-model");
  expect((screen.getByRole("combobox", { name: "思考强度" }) as HTMLSelectElement).value).toBe("high");
  fireEvent.paste(screen.getByRole("textbox", { name: "提示词" }), { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "remote task" : "" } });
  fireEvent.click(screen.getByRole("button", { name: "添加任务" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("create_task", expect.objectContaining({ codexSettings: { model: "test-model", effort: "high" } })));
});

it("切换 Workspace 后读取各自默认设置，并忽略旧目录迟到结果", async () => {
  let resolve!: (value: typeof catalog) => void;
  const old = new Promise<typeof catalog>(yes => { resolve = yes; });
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => {
    if (cmd === "last_codex_settings") return Promise.resolve((args as { projectId: string }).projectId === project.id
      ? { model: "test-model", effort: "high" } : { model: "other-model", effort: "low" });
    if (cmd === "codex_model_catalog") return (args as { projectId: string }).projectId === project.id ? old : Promise.resolve({ ...catalog, models: [{ id: "other-model", name: "Other Model", efforts: ["low"] }] });
    return base(cmd, args, options);
  });
  const form = (remote = false) => <TaskFormDialog project={remote ? { ...project, id: "remote-project", machine_id: "remote" } : project} onClose={() => {}} onSaved={() => {}} />;
  const ui = render(form());
  await waitFor(() => expect((screen.getByRole("combobox", { name: "AI 模型" }) as HTMLSelectElement).value).toBe("test-model"));
  ui.rerender(form(true));
  await screen.findByRole("option", { name: "Other Model" });
  await act(async () => resolve(catalog));
  expect(screen.queryByRole("option", { name: "Test Model" })).toBeNull();
  expect((screen.getByRole("combobox", { name: "AI 模型" }) as HTMLSelectElement).value).toBe("other-model");
  expect((screen.getByRole("combobox", { name: "思考强度" }) as HTMLSelectElement).value).toBe("low");
  expect(invoke).toHaveBeenCalledWith("last_codex_settings", { projectId: project.id });
  expect(invoke).toHaveBeenCalledWith("last_codex_settings", { projectId: "remote-project" });
  expect(screen.queryByRole("alert")).toBeNull();
});

it("旧 Workspace 默认值迟到时不覆盖当前设置，等待当前值期间禁止创建", async () => {
  let resolveOld!: (value: { model: string; effort: string }) => void;
  let resolveCurrent!: (value: { model: string; effort: string }) => void;
  const old = new Promise<{ model: string; effort: string }>(yes => { resolveOld = yes; });
  const current = new Promise<{ model: string; effort: string }>(yes => { resolveCurrent = yes; });
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => {
    if (cmd === "last_codex_settings") return (args as { projectId: string }).projectId === project.id ? old : current;
    if (cmd === "codex_model_catalog") return Promise.resolve(catalog);
    return base(cmd, args, options);
  });
  const form = (id: string) => <TaskFormDialog project={{ ...project, id }} onClose={() => {}} onSaved={() => {}} />;
  const ui = render(form(project.id));
  fireEvent.paste(screen.getByRole("textbox", { name: "提示词" }), { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "workspace draft" : "" } });
  ui.rerender(form("other-project"));
  await act(async () => resolveOld({ model: "test-model", effort: "high" }));
  expect((screen.getByRole("button", { name: "添加任务" }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.submit(screen.getByRole("dialog", { name: "新建任务" }));
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "create_task")).toBe(false);
  await act(async () => resolveCurrent({ model: "test-model", effort: "low" }));
  expect((screen.getByRole("combobox", { name: "思考强度" }) as HTMLSelectElement).value).toBe("low");
  fireEvent.click(screen.getByRole("button", { name: "添加任务" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("create_task", expect.objectContaining({ projectId: "other-project", codexSettings: { model: "test-model", effort: "low" } })));
});

it("Workspace 默认值读取失败时保留正文并阻止用临时默认值创建", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => {
    if (cmd === "last_codex_settings") return Promise.reject("workspace.project_missing");
    return base(cmd, args, options);
  });
  render(<TaskFormDialog project={project} onClose={() => {}} onSaved={() => {}} />);
  await screen.findByText("找不到项目。");
  fireEvent.paste(screen.getByRole("textbox", { name: "提示词" }), { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "keep my draft" : "" } });
  expect(screen.getByRole("textbox", { name: "提示词" }).textContent).toContain("keep my draft");
  expect((screen.getByRole("button", { name: "添加任务" }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.submit(screen.getByRole("dialog", { name: "新建任务" }));
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "create_task")).toBe(false);
});

it("修改 Todo 配置进入草稿保护并保存整组配置", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => cmd === "codex_model_catalog" ? Promise.resolve(catalog) : base(cmd, args, options));
  await renderBoard();
  fireEvent.click(screen.getByRole("button", { name: `查看并编辑：${board.storedTask.name}` }));
  await screen.findByRole("option", { name: "Test Model" });
  fireEvent.change(screen.getByRole("combobox", { name: "AI 模型" }), { target: { value: "test-model" } });
  fireEvent.change(screen.getByRole("combobox", { name: "思考强度" }), { target: { value: "high" } });
  expect(screen.getByRole("status").textContent).toBe("有未保存的修改");
  fireEvent.click(screen.getAllByRole("button", { name: "关闭" })[0]);
  fireEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
  expect((screen.getByRole("combobox", { name: "思考强度" }) as HTMLSelectElement).value).toBe("high");
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("update_task", expect.objectContaining({ codexSettings: { model: "test-model", effort: "high" } })));
});

it("继承不支持的强度时提示并阻止保存，不静默替换", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => {
    if (cmd === "last_codex_settings") return Promise.resolve({ model: "test-model", effort: "ultra" });
    if (cmd === "codex_model_catalog") return Promise.resolve(catalog);
    return base(cmd, args, options);
  });
  await renderBoard();
  fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
  await screen.findByText("当前模型或思考强度不受支持，请重新选择。");
  expect((screen.getByRole("combobox", { name: "思考强度" }) as HTMLSelectElement).value).toBe("ultra");
  expect((screen.getByRole("button", { name: "添加任务" }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(screen.getByRole("combobox", { name: "思考强度" }), { target: { value: "low" } });
  await waitFor(() => expect(screen.queryByText("当前模型或思考强度不受支持，请重新选择。")).toBeNull());
});
it("新建继承最近模型与强度并将整组配置保存，编辑载入自身配置", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((cmd, args, options) => {
    if (cmd === "last_codex_settings") return Promise.resolve({ model: "test-model", effort: "high" });
    if (cmd === "codex_model_catalog") return Promise.resolve(catalog);
    return base(cmd, args, options);
  });
  await renderBoard();
  fireEvent.click(screen.getByRole("button", { name: /新建任务/ }));
  const form = screen.getByRole("dialog", { name: "新建任务" });
  await waitFor(() => expect((within(form).getByRole("combobox", { name: "AI 模型" }) as HTMLSelectElement).value).toBe("test-model"));
  expect((within(form).getByRole("combobox", { name: "思考强度" }) as HTMLSelectElement).value).toBe("high");
  fireEvent.paste(within(form).getByRole("textbox", { name: "提示词" }), { clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? "Use the selected model" : "" } });
  fireEvent.click(within(form).getByRole("button", { name: "添加任务" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("create_task", expect.objectContaining({ codexSettings: { model: "test-model", effort: "high" } })));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  fireEvent.click(screen.getByRole("button", { name: `查看并编辑：${board.storedTask.name}` }));
  expect((screen.getByRole("combobox", { name: "AI 模型" }) as HTMLSelectElement).value).toBe("");
});
