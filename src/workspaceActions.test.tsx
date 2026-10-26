// @vitest-environment jsdom
import { project, board } from "./board-tests/fixtures";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ProjectSidebar } from "./ProjectSidebar";
import { ToastContainer } from "./Toast";
import { setLocale } from "./i18n";
import { terminalBounds } from "./nativeTerminalBounds";
import App from "./App";

const commands = () => vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "delete_project" || cmd === "hide_project");
const labels = { hide: "从侧栏隐藏", delete: "删除项目" };
const confirms = { hide: "确认隐藏", delete: "确认删除" };

for (const action of ["hide", "delete"] as const) {
  it(`${action}: cancel, Escape and backdrop leave the workspace intact; trap and restore focus`, () => {
    const hide = vi.fn(), changed = vi.fn();
    const root = document.createElement("div"); root.id = "root"; document.body.append(root);
    const view = render(<ProjectSidebar projects={[project]} selectedId={project.id} onSelect={() => {}} onChanged={changed} onHide={hide} />, { container: root });
    const opener = screen.getByRole("button", { name: `${labels[action]} ${project.name}` });
    for (const method of ["cancel", "escape", "backdrop"]) {
      fireEvent.click(opener);
      const dialog = screen.getByRole("dialog");
      const cancel = within(dialog).getByRole("button", { name: "取消" });
      const confirm = within(dialog).getByRole("button", { name: confirms[action] });
      expect(document.activeElement).toBe(cancel);
      expect(root.inert).toBe(true);
      expect(terminalBounds(root).visible).toBe(false);
      expect(dialog.textContent).toContain(project.name);
      fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
      expect(document.activeElement).toBe(confirm);
      fireEvent.keyDown(confirm, { key: "Tab" });
      expect(document.activeElement).toBe(cancel);
      if (method === "cancel") fireEvent.click(cancel);
      if (method === "escape") fireEvent.keyDown(cancel, { key: "Escape" });
      if (method === "backdrop") fireEvent.mouseDown(dialog.parentElement!);
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(root.inert).toBe(false);
      expect(document.activeElement).toBe(opener);
      expect(terminalBounds(root).visible).toBe(true);
    }
    expect(hide).not.toHaveBeenCalled(); expect(changed).not.toHaveBeenCalled(); expect(commands()).toEqual([]);
    view.unmount(); root.remove();
  });

  it(`${action}: only confirms the chosen workspace and prevents repeated operations while pending`, async () => {
    let finish!: () => void;
    const operation = new Promise<void>(resolve => { finish = resolve; });
    const hide = vi.fn(() => operation), changed = vi.fn();
    vi.mocked(invoke).mockImplementation(() => operation);
    const second = { ...project, id: "second", name: "Other" };
    render(<ProjectSidebar projects={[project, second]} selectedId={project.id} onSelect={() => {}} onChanged={changed} onHide={hide} />);
    fireEvent.click(screen.getByRole("button", { name: `${labels[action]} Other` }));
    expect(commands()).toEqual([]); expect(hide).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: confirms[action] }));
    expect(screen.queryByRole("dialog")).toBeNull();
    for (const label of Object.values(labels)) {
      const button = screen.getByRole("button", { name: `${label} Other` }) as HTMLButtonElement;
      expect(button.disabled).toBe(true); fireEvent.click(button);
    }
    if (action === "hide") { expect(hide).toHaveBeenCalledExactlyOnceWith("second"); expect(commands()).toEqual([]); }
    else { expect(invoke).toHaveBeenCalledExactlyOnceWith("delete_project", { id: "second" }); expect(hide).not.toHaveBeenCalled(); }
    await act(async () => finish());
    expect((screen.getByRole("button", { name: `${labels[action]} Other` }) as HTMLButtonElement).disabled).toBe(false);
    expect(changed).toHaveBeenCalledTimes(action === "delete" ? 1 : 0);
  });

  it(`${action}: failure keeps the workspace available and allows a fresh confirmation`, async () => {
    const hide = vi.fn().mockRejectedValue("fixture failure"), changed = vi.fn();
    vi.mocked(invoke).mockRejectedValue("fixture failure");
    render(<><ToastContainer /><ProjectSidebar projects={[project]} selectedId={null} onSelect={() => {}} onChanged={changed} onHide={hide} /></>);
    fireEvent.click(screen.getByRole("button", { name: `${labels[action]} ${project.name}` }));
    fireEvent.click(screen.getByRole("button", { name: confirms[action] }));
    await screen.findByText("fixture failure");
    expect(changed).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: `${labels[action]} ${project.name}` }));
    expect(screen.getByRole("dialog")).toBeTruthy();
  });
}

it("shows the remote computer and English consequences for same-named workspaces", () => {
  setLocale("en-US");
  try {
    render(<ProjectSidebar projects={[{ ...project, machine_id: "remote" }]} machines={[{ id: "remote", name: "Dev PC", kind: "ssh", session: "default", disabled: false, profile_id: "fixture", ssh_target: "devpc" }]}
      selectedId={null} onSelect={() => {}} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: `Delete project ${project.name}` }));
    const dialog = screen.getByRole("dialog", { name: "Delete local project?" });
    expect(dialog.textContent).toContain("Dev PC");
    expect(dialog.textContent).toContain("This cannot be undone. The Herdr Workspace will remain.");
  } finally { act(() => setLocale("zh-CN")); }
});

it("hidden workspace confirmation hands off to draft protection without overlapping modals or leaving inert behind", async () => {
  let hidden = false;
  const base = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => {
    if (command === "list_projects") return Promise.resolve(hidden ? [] : [project]);
    if (command === "navigation_state") return Promise.resolve({ configured: true, recovery_projects: [] });
    if (command === "hide_project") { hidden = true; return Promise.resolve(); }
    return base(command, args, options);
  });
  const root = document.createElement("div"); root.id = "root"; document.body.append(root);
  const view = render(<App />, { container: root });
  fireEvent.click(await screen.findByRole("button", { name: `查看并编辑：${board.storedTask.name}` }));
  fireEvent.change(screen.getByRole("textbox", { name: "标题" }), { target: { value: "草稿标题" } });
  for (const decision of ["继续编辑", "保存并继续"]) {
    fireEvent.click(screen.getByRole("button", { name: `从侧栏隐藏 ${project.name}` }));
    fireEvent.click(screen.getByRole("button", { name: "确认隐藏" }));
    const button = await screen.findByRole("button", { name: decision });
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(root.inert).toBe(true); expect(hidden).toBe(false);
    fireEvent.click(button);
    if (decision === "继续编辑") {
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(root.inert).toBe(false);
      expect((screen.getByRole("textbox", { name: "标题" }) as HTMLInputElement).value).toBe("草稿标题");
      expect(commands()).toEqual([]);
    }
  }
  await screen.findByText("当前没有显示的 Workspace，可随时重新添加。");
  expect(board.storedTask.name).toBe("草稿标题");
  expect(commands()).toHaveLength(1); expect(root.inert).toBe(false);
  view.unmount(); root.remove();
});
