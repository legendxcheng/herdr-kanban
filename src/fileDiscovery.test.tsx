// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { Editor, EditorContext } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { MentionNode } from "./MentionNode";
import { MentionPopover } from "./MentionPopover";
import type { FileEntry } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const editors: Editor[] = [];

afterEach(() => {
  cleanup();
  editors.splice(0).forEach(editor => editor.destroy());
  vi.useRealTimers();
  vi.resetAllMocks();
});

function setup() {
  const editor = new Editor({
    extensions: [StarterKit, MentionNode], content: "<p></p>",
    editorProps: { handleScrollToSelection: () => true },
  });
  editors.push(editor);
  vi.spyOn(editor.view, "coordsAtPos").mockReturnValue({ top: 0, bottom: 10, left: 0, right: 1 });
  const view = (rootPath: string) => <EditorContext.Provider value={{ editor }}>
    <MentionPopover rootPath={rootPath} />
  </EditorContext.Provider>;
  const rendered = render(view("E:/project"));
  return {
    editor,
    type: (text: string) => act(() => {
      editor.commands.setContent("<p></p>");
      editor.commands.insertContent(text);
    }),
    project: (root: string) => rendered.rerender(view(root)),
  };
}

const file = (rel_path: string): FileEntry => ({ rel_path, abs_path: `E:/project/${rel_path}` });

it("combines rapid typing into one file search and cancels pending searches on close", async () => {
  vi.mocked(invoke).mockResolvedValue([]);
  const ui = setup();
  vi.useFakeTimers();
  ui.type("@M");
  await act(async () => { await vi.advanceTimersByTimeAsync(60); });
  ui.type("@Mention");
  await act(async () => { await vi.advanceTimersByTimeAsync(60); });
  expect(invoke).not.toHaveBeenCalled();
  await act(async () => { await vi.advanceTimersByTimeAsync(120); });
  expect(invoke).toHaveBeenCalledExactlyOnceWith("search_files", { rootPath: "E:/project", query: "Mention" });
  ui.type("@App");
  ui.type("plain text");
  await act(async () => { await vi.advanceTimersByTimeAsync(200); });
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("clears stale candidates while typing and ignores late responses and previous projects", async () => {
  const pending: Array<{ query: string; root: string; resolve: (value: FileEntry[]) => void }> = [];
  vi.mocked(invoke).mockImplementation((_command, args) => new Promise(resolve => {
    const input = args as { query: string; rootPath: string };
    pending.push({ query: input.query, root: input.rootPath, resolve });
  }));
  const ui = setup();
  ui.type("@App");
  await waitFor(() => expect(pending).toHaveLength(1));
  ui.type("@Mention");
  await waitFor(() => expect(pending).toHaveLength(2));
  await act(async () => { pending[1].resolve([file("src/MentionPopover.tsx")]); });
  await screen.findByText("src/MentionPopover.tsx");
  await act(async () => { pending[0].resolve([file("src/App.tsx")]); });
  expect(screen.queryByText("src/App.tsx")).toBeNull();
  ui.type("@Missing");
  expect(screen.queryByText("src/MentionPopover.tsx")).toBeNull();
  await waitFor(() => expect(pending).toHaveLength(3));
  ui.project("E:/other");
  await act(async () => { pending[2].resolve([file("wrong-project.ts")]); });
  expect(screen.queryByText("wrong-project.ts")).toBeNull();
  await waitFor(() => expect(pending).toHaveLength(4));
  expect(pending[3].root).toBe("E:/other");
});

it("keeps backend ranking and saves a keyboard-selected relative file reference", async () => {
  const selected = file(".scratch/probe/check.ts");
  vi.mocked(invoke).mockResolvedValue([selected, file(".scratch/probe/other.ts")]);
  const ui = setup();
  ui.type("@.scratch/probe/chk");
  await screen.findByText(selected.rel_path);
  expect([...document.querySelectorAll(".mention-label")].map(node => node.textContent)).toEqual([
    selected.rel_path, ".scratch/probe/other.ts",
  ]);
  fireEvent.keyDown(ui.editor.view.dom, { key: "Enter" });
  expect(ui.editor.getJSON().content?.[0].content?.[0]).toEqual({
    type: "mention", attrs: { kind: "file", label: selected.rel_path, ref: selected.abs_path },
  });
  expect(document.querySelector(".mention-popover")).toBeNull();
});

it.each(["kanban\\src\\MentionPopover", "文档/说明"])("passes the relative query %s to file discovery", async query => {
  vi.mocked(invoke).mockResolvedValue([]);
  const ui = setup();
  ui.type(`@${query}`);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("search_files", { rootPath: "E:/project", query }));
});
