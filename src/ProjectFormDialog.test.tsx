// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ProjectFormDialog } from "./ProjectFormDialog";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); invoke.mockReset(); });

it("creates a Herdr workspace with a directory and never reads ContextSwitcher", async () => {
  invoke.mockResolvedValue({ projects: [], workspaces: [], operations: [] });
  const saved = vi.fn();
  render(<ProjectFormDialog onClose={() => {}} onSaved={saved} />);
  fireEvent.change(screen.getByLabelText("工作空间名称"), { target: { value: "Alpha" } });
  fireEvent.change(screen.getByLabelText("工作目录"), { target: { value: "E:/Alpha" } });
  fireEvent.click(screen.getByRole("button", { name: "创建工作空间" }));
  await waitFor(() => expect(saved).toHaveBeenCalled());
  expect(invoke).toHaveBeenCalledWith("create_project", { name: "Alpha", rootPath: "E:/Alpha", operationId: expect.any(String) });
  expect(invoke.mock.calls.some(([command]) => command === "read_cs_tasks")).toBe(false);
});

it("checks an uncertain creation without sending another create and retains the draft", async () => {
  let operationId = "";
  invoke.mockImplementation(async (command, args) => {
    if (command === "create_project") { operationId = args.operationId; throw new Error("herdr.timeout"); }
    if (command === "workspace_operations") return [{ id: operationId, state: "unknown" }];
    if (command === "recover_workspace") return {};
    throw new Error(`Unexpected ${command}`);
  });
  const saved = vi.fn(); render(<ProjectFormDialog onClose={() => {}} onSaved={saved} />);
  fireEvent.change(screen.getByLabelText("工作空间名称"), { target: { value: "Alpha" } });
  fireEvent.change(screen.getByLabelText("工作目录"), { target: { value: "E:/Alpha" } });
  fireEvent.click(screen.getByRole("button", { name: "创建工作空间" }));
  fireEvent.click(await screen.findByRole("button", { name: "核实创建结果" }));
  await waitFor(() => expect(saved).toHaveBeenCalled());
  expect(invoke.mock.calls.filter(([command]) => command === "create_project")).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith("recover_workspace", { operationId, workspaceId: null });
});

it("adds an existing workspace by ID using its Herdr directory", async () => {
  invoke.mockImplementation(async (command) => command === "sync_projects" ? { session: "default", projects: [], workspaces: [{ id: "w2", name: "Existing", directory: "E:/Existing" }], operations: [] } : {});
  const saved = vi.fn(); render(<ProjectFormDialog onClose={() => {}} onSaved={saved} />);
  fireEvent.click(screen.getByRole("button", { name: "添加已有工作空间" }));
  await screen.findByRole("option", { name: /Existing/ });
  fireEvent.change(screen.getByLabelText("选择 Herdr 工作空间"), { target: { value: "w2" } });
  expect((screen.getByLabelText("工作目录") as HTMLInputElement).value).toBe("E:/Existing");
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(saved).toHaveBeenCalled());
  expect(invoke).toHaveBeenCalledWith("link_workspace", { projectId: null, workspaceId: "w2", fallbackPath: "E:/Existing" });
  expect(invoke.mock.calls.some(([command]) => command === "create_project")).toBe(false);
});

it("reuses the failed operation ID when retrying a creation", async () => {
  let operationId = ""; let attempt = 0;
  invoke.mockImplementation(async (command, args) => {
    if (command === "create_project") { attempt++; if (attempt === 1) { operationId = args.operationId; throw new Error("cannot spawn"); } return {}; }
    if (command === "workspace_operations") return [{ id: operationId, state: "failed" }];
    throw new Error(`Unexpected ${command}`);
  });
  const saved = vi.fn(); render(<ProjectFormDialog onClose={() => {}} onSaved={saved} />);
  fireEvent.change(screen.getByLabelText("工作空间名称"), { target: { value: "Alpha" } });
  fireEvent.change(screen.getByLabelText("工作目录"), { target: { value: "E:/Alpha" } });
  fireEvent.click(screen.getByRole("button", { name: "创建工作空间" }));
  await screen.findByRole("alert");
  await waitFor(() => expect((screen.getByRole("button", { name: "创建工作空间" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "创建工作空间" }));
  await waitFor(() => expect(saved).toHaveBeenCalled());
  const calls = invoke.mock.calls.filter(([command]) => command === "create_project");
  expect(calls.map(([, args]) => args.operationId)).toEqual([operationId, operationId]);
});
