// @vitest-environment jsdom
import { project } from "./board-tests/fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { WorkspaceRecovery } from "./WorkspaceRecovery";
import { setLocale } from "./i18n";
import App from "./App";

const operation = { id: "attempt", session: "default", name: "Failed workspace", cwd: "G:/fixture", workspace_id: "w1", project_id: "attempt", state: "unknown", error: "workspace.occupied" };

it("requires confirmation, identifies the machine, and supports cancel/Escape/backdrop with focus restoration", () => {
  render(<WorkspaceRecovery operation={operation} machine="Remote PC" onChanged={vi.fn()} />);
  const opener = screen.getByRole("button", { name: "移除记录" });
  for (const method of ["cancel", "escape", "backdrop"]) {
    fireEvent.click(opener);
    const dialog = screen.getByRole("dialog");
    const cancel = within(dialog).getByRole("button", { name: "取消" });
    expect(document.activeElement).toBe(cancel);
    expect(dialog.textContent).toContain("Remote PC · Failed workspace");
    expect(dialog.textContent).toContain("已有工作区、任务和 Herdr 进程会保留");
    if (method === "cancel") fireEvent.click(cancel);
    if (method === "escape") fireEvent.keyDown(cancel, { key: "Escape" });
    if (method === "backdrop") fireEvent.mouseDown(dialog.parentElement!);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
  }
  expect(invoke).not.toHaveBeenCalled();
});

it("blocks repeat dismissal and recovery until completion; preserves the row and permits retry after failure", async () => {
  let reject!: (error: string) => void;
  vi.mocked(invoke).mockReturnValue(new Promise((_, fail) => { reject = fail; }));
  const changed = vi.fn();
  render(<WorkspaceRecovery operation={{ ...operation, state: "failed" }} machine="Local" onChanged={changed} />);
  fireEvent.click(screen.getByRole("button", { name: "移除记录" }));
  fireEvent.click(screen.getByRole("button", { name: "确认移除" }));
  for (const name of ["移除记录", "重新创建"]) {
    const button = screen.getByRole("button", { name }) as HTMLButtonElement;
    expect(button.disabled).toBe(true); fireEvent.click(button);
  }
  expect(invoke).toHaveBeenCalledExactlyOnceWith("dismiss_workspace_operation", { operationId: "attempt" });
  await act(async () => reject("fixture failure"));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "fixture failure");
  expect(changed).not.toHaveBeenCalled();
  vi.mocked(invoke).mockResolvedValue(undefined);
  fireEvent.click(screen.getByRole("button", { name: "移除记录" }));
  fireEvent.click(screen.getByRole("button", { name: "确认移除" }));
  await waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
});

it("does not offer dismissal while creation is pending", () => {
  render(<WorkspaceRecovery operation={{ ...operation, state: "pending" }} machine="Local" onChanged={vi.fn()} />);
  expect(screen.queryByRole("button", { name: "移除记录" })).toBeNull();
});

it("provides English removal text", () => {
  setLocale("en-US");
  try {
    render(<WorkspaceRecovery operation={operation} machine="Remote PC" onChanged={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Remove record" }));
    expect(screen.getByRole("dialog", { name: "Remove this creation record?" }).textContent).toContain("Existing workspaces, tasks and Herdr processes will remain.");
  } finally { act(() => setLocale("zh-CN")); }
});

it("removes immediately while offline and ignores stale operation snapshots without deleting the existing project", async () => {
  const base = vi.mocked(invoke).getMockImplementation()!;
  let completeSync!: (result: unknown) => void;
  let slow = false;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "ensure_local_herdr") return Promise.resolve();
    if (command === "list_projects") return Promise.resolve([project]);
    if (command === "sync_projects") {
      if (slow) return new Promise(resolve => { completeSync = resolve; });
      return Promise.reject("herdr.unavailable");
    }
    if (command === "workspace_operations") return Promise.resolve([operation]);
    if (command === "dismiss_workspace_operation") return Promise.resolve();
    return base(command, args, options);
  });
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "移除记录" }));
  slow = true;
  fireEvent.click(screen.getByRole("button", { name: "确认移除" }));
  await waitFor(() => expect(screen.queryByText("Failed workspace")).toBeNull());
  await waitFor(() => expect(completeSync).toBeTypeOf("function"));
  await act(async () => completeSync({ projects: [project], operations: [operation], workspaces: [], session: "default" }));
  expect(screen.queryByText("Failed workspace")).toBeNull();
  expect(screen.getByRole("button", { name: new RegExp(`^${project.name}`) })).toBeTruthy();
  expect(vi.mocked(invoke).mock.calls.some(([cmd]) => ["delete_project", "hide_project", "create_project", "recover_workspace", "delete_task"].includes(cmd))).toBe(false);
});
