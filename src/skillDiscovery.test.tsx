// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { Editor, EditorContext } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { MentionNode } from "./MentionNode";
import { MentionPopover } from "./MentionPopover";
import { resolveMentions } from "./dispatchUtils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

afterEach(() => { cleanup(); vi.resetAllMocks(); });

it("searches with project context and saves a selectable native skill reference", async () => {
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    expect(command).toBe("search_skills");
    const input = args as { rootPath: string; query: string };
    return input.rootPath === "E:/ContextSwitcher" && input.query === "kanban-dev" ? [{
      name: "kanban-dev", description: "项目技能",
      skill_md_path: "E:/ContextSwitcher/.agents/skills/kanban-dev/SKILL.md",
    }] : [];
  });
  const editor = new Editor({
    extensions: [StarterKit, MentionNode],
    content: "<p></p>",
    // jsdom has no layout; selection and transactions still use real Tiptap.
    editorProps: { handleScrollToSelection: () => true },
  });
  vi.spyOn(editor.view, "coordsAtPos").mockReturnValue({ top: 0, bottom: 10, left: 0, right: 1 });
  try {
    const { rerender } = render(<EditorContext.Provider value={{ editor }}>
      <MentionPopover rootPath="E:/ContextSwitcher" />
    </EditorContext.Provider>);
    act(() => { editor.commands.insertContent("$kanban-dev"); });
    const candidate = await screen.findByText("kanban-dev");
    expect(invoke).toHaveBeenCalledWith("search_skills", {
      rootPath: "E:/ContextSwitcher", query: "kanban-dev",
    });
    fireEvent.mouseDown(candidate);
    const saved = JSON.parse(JSON.stringify(editor.getJSON()));
    expect(saved.content[0].content[0]).toEqual({
      type: "mention", attrs: {
        kind: "skill", label: "kanban-dev",
        ref: "E:/ContextSwitcher/.agents/skills/kanban-dev/SKILL.md",
      },
    });
    expect(await resolveMentions(saved, "E:/ContextSwitcher")).toBe("$kanban-dev");

    // A project change must query the new context rather than keep old candidates.
    act(() => { editor.commands.setContent("<p>$kanban-dev</p>"); editor.commands.setTextSelection(12); });
    await screen.findByText("kanban-dev");
    rerender(<EditorContext.Provider value={{ editor }}>
      <MentionPopover rootPath="E:/another-project" />
    </EditorContext.Provider>);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("search_skills", {
      rootPath: "E:/another-project", query: "kanban-dev",
    }));
    await waitFor(() => expect(screen.queryByText("kanban-dev")).toBeNull());
  } finally { cleanup(); editor.destroy(); }
});
