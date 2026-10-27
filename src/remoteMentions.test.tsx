// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TaskFormDialog } from "./TaskFormDialog";
import { resolveMentions } from "./dispatchUtils";
import type { Project, Task } from "./types";
import { setLocale } from "./i18n";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const project = { id: "remote-project", machine_id: "ssh-computer", name: "Remote", root_path: "D:/work" } as Project;
const file = { rel_path: "remote.txt", abs_path: "D:/work/remote.txt" };
const skill = { name: "remote-skill", description: "Remote skill", skill_md_path: "D:/work/.agents/skills/remote-skill/SKILL.md" };
const paste = (text: string) => fireEvent.paste(screen.getByRole("textbox", { name: "提示词" }), {
  clipboardData: { files: [], getData: (type: string) => type === "text/plain" ? text : "" },
});
function form(p = project, task?: Task) {
  return <TaskFormDialog project={p} task={task} onClose={() => {}} onSaved={() => {}} />;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  setLocale("zh-CN");
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
});
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it("shows remote wait and failure, retries without losing the draft, then saves and reopens both references", async () => {
  const pending = deferred<unknown[]>();
  let searches = 0;
  let saved = "";
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "search_files") return ++searches === 1 ? pending.promise : [file];
    if (command === "search_skills") return [skill];
    if (command === "create_task") { saved = (args as { promptJson: string }).promptJson; return {}; }
    if (command === "read_file_content") return "REMOTE-CONTENT";
  });
  const ui = render(form());
  paste("检查 @remote");
  expect(await screen.findByText("正在搜索远端…")).toBeTruthy();
  await waitFor(() => expect(searches).toBe(1));
  await act(async () => pending.reject("machine.setup_required"));
  expect((await screen.findByRole("alert")).textContent).toContain("搜索失败");
  expect(screen.getByRole("textbox", { name: "提示词" }).textContent).toBe("检查 @remote");
  fireEvent.click(screen.getByRole("button", { name: "重试搜索" }));
  await screen.findByText("remote.txt");
  fireEvent.keyDown(screen.getByRole("textbox", { name: "提示词" }), { key: "Enter" });
  paste("$remote");
  fireEvent.mouseDown(await screen.findByText("remote-skill"));
  fireEvent.click(screen.getByRole("button", { name: "添加任务" }));
  await waitFor(() => expect(saved).toContain('"kind":"skill"'));
  const nodes = JSON.parse(saved).content[0].content;
  expect(nodes.filter((n: any) => n.type === "mention").map((n: any) => n.attrs)).toEqual([
    { kind: "file", label: file.rel_path, ref: file.abs_path },
    { kind: "skill", label: skill.name, ref: skill.skill_md_path },
  ]);
  expect(invoke).toHaveBeenCalledWith("search_files", { rootPath: project.root_path, query: "remote", projectId: project.id });
  expect(invoke).toHaveBeenCalledWith("search_skills", { rootPath: project.root_path, query: "remote", projectId: project.id });
  ui.unmount();
  render(form(project, { id: "saved", name: "Saved", priority: "normal", prompt_json: saved } as Task));
  expect(document.querySelectorAll(".mention-chip")).toHaveLength(2);
  const prompt = resolveMentions(JSON.parse(saved), project.root_path);
  expect(prompt).toContain("remote.txt");
  expect(prompt).not.toContain("[文件:");
  expect(prompt).not.toContain("\n");
  expect(prompt).toContain("$remote-skill");
  expect(prompt).not.toContain("REMOTE-CONTENT");
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "read_file_content" || command === "read_skill_content")).toBe(false);
});

it.each(["@", "$"])("shows empty results for %s and Escape dismisses the popover without closing the form", async trigger => {
  vi.mocked(invoke).mockImplementation(async command => command.startsWith("search_") ? [] : undefined);
  render(form()); paste(trigger + "missing");
  expect(await screen.findByText(trigger === "@" ? "没有匹配的文件" : "没有匹配的技能")).toBeTruthy();
  fireEvent.keyDown(screen.getByRole("textbox", { name: "提示词" }), { key: "Escape" });
  expect(document.querySelector(".mention-popover")).toBeNull();
  expect(screen.getByRole("dialog")).toBeTruthy();
});

it("prewarms on opening and lets current input finish before a stale query", async () => {
  const first = deferred<unknown[]>();
  const queries: string[] = [];
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command !== "search_skills") return;
    queries.push((args as { query: string }).query);
    return queries.length === 1 ? first.promise : [skill];
  });
  render(form()); paste("$a");
  expect(invoke).toHaveBeenCalledWith("prepare_mention_search", { rootPath: project.root_path, projectId: project.id });
  await waitFor(() => expect(queries).toEqual(["a"]));
  paste("b");
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 160)); });
  paste("c");
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 160)); });
  expect(queries).toEqual(["a", "ab", "abc"]);
  await screen.findByText("remote-skill");
  await act(async () => first.resolve([{ ...skill, name: "stale" }]));
  await screen.findByText("remote-skill");
  expect(queries).toEqual(["a", "ab", "abc"]);
  expect(screen.queryByText("stale")).toBeNull();
});

it("isolates same-path projects and does not let a stalled remote search block local lookup", async () => {
  const old = deferred<unknown[]>();
  vi.mocked(invoke).mockImplementation(async (command, args) => command === "search_skills" ? ((args as { projectId?: string }).projectId === project.id ? old.promise : [skill]) : undefined);
  const ui = render(form()); paste("$remote");
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("search_skills", { rootPath: project.root_path, query: "remote", projectId: project.id }));
  ui.rerender(form({ ...project, id: "second-remote" }));
  await screen.findByText("remote-skill");
  ui.rerender(form({ ...project, id: "local-project", machine_id: "local" }));
  await screen.findByText("remote-skill");
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "search_skills").slice(-1)[0]?.slice(0, 2)).toEqual(["search_skills", { rootPath: project.root_path, query: "remote" }]);
  await act(async () => old.reject("old failure"));
  expect(screen.queryByRole("alert")).toBeNull();
});

it("explains a missing file-search directory and ignores results after closing", async () => {
  const pending = deferred<unknown[]>();
  vi.mocked(invoke).mockImplementation(async command => command === "search_files" ? pending.promise : undefined);
  const ui = render(form({ ...project, root_path: "" })); paste("@file");
  expect(await screen.findByText("请先设置项目工作目录")).toBeTruthy();
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "search_files")).toBe(false);
  ui.rerender(form());
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("search_files", { rootPath: project.root_path, query: "file", projectId: project.id }));
  fireEvent.keyDown(screen.getByRole("textbox", { name: "提示词" }), { key: "Escape" });
  await act(async () => pending.resolve([file]));
  expect(document.querySelector(".mention-popover")).toBeNull();
});

it("refreshes cached candidates without losing the prompt or reloading on subsequent typing", async () => {
  vi.mocked(invoke).mockImplementation(async command => command === "search_skills" ? [skill] : undefined);
  render(form()); paste("draft $remote");
  await screen.findByText("remote-skill");
  fireEvent.click(screen.getByRole("button", { name: "刷新候选列表" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("search_skills", {
    rootPath: project.root_path, query: "remote", projectId: project.id, refresh: true,
  }));
  expect(screen.getByRole("textbox", { name: "提示词" }).textContent).toBe("draft $remote");
  await screen.findByText("remote-skill");
  paste("x");
  await waitFor(() => expect(invoke).toHaveBeenLastCalledWith("search_skills", {
    rootPath: project.root_path, query: "remotex", projectId: project.id,
  }));
});
