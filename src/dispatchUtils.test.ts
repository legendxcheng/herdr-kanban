import { describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { fileReference, resolveMentions, workspaceRelativePath } from "./dispatchUtils";
import pathCases from "./promptPathCases.json";

vi.mock("./api", () => ({
  readSkillContent: vi.fn().mockResolvedValue("SKILL BODY SHOULD NOT BE SENT"),
  readFileContent: vi.fn().mockResolvedValue("file body"),
}));

describe("dispatch prompt", () => {
  it("preserves a skill invocation instead of pasting SKILL.md", async () => {
    const doc = { type: "doc", content: [{ type: "paragraph", content: [
      { type: "text", text: "开始实现 " },
      { type: "mention", attrs: { kind: "skill", label: "to-spec", ref: "C:/skills/to-spec/SKILL.md" } },
    ] }] };
    expect(await resolveMentions(doc, "E:/fixture")).toBe("开始实现 $to-spec");
    expect(api.readSkillContent).not.toHaveBeenCalled();
  });

  it("sends relative file paths alongside native skills without reading either body", async () => {
    const doc = { type: "doc", content: [{ type: "paragraph", content: [
      { type: "mention", attrs: { kind: "file", label: "plan.md", ref: "E:/fixture/plan.md" } },
      { type: "text", text: " " },
      { type: "mention", attrs: { kind: "skill", label: "to-spec", ref: "C:/skills/to-spec/SKILL.md" } },
    ] }] };
    const prompt = await resolveMentions(doc, "E:/fixture");
    expect(prompt).toBe("plan.md $to-spec");
    expect(api.readFileContent).not.toHaveBeenCalled();
    expect(api.readSkillContent).not.toHaveBeenCalled();
  });

  it.each(pathCases)("formats $file relative to $root", ({ file, root, expected, formatted }) => {
    expect(workspaceRelativePath(file, root)).toBe(expected);
    expect(fileReference(file, root)).toBe(formatted);
  });

  it("preserves prose, adjacent references and authored paragraphs without adding file wrappers", () => {
    const doc = { type: "doc", content: [
      { type: "paragraph", content: [
        { type: "text", text: "对照 " },
        { type: "mention", attrs: { kind: "file", label: "README.md", ref: "E:/fixture/README.md" } },
        { type: "text", text: " " },
        { type: "mention", attrs: { kind: "file", label: "说明 文件.md", ref: "E:/fixture/docs/说明 文件.md" } },
        { type: "text", text: " 检查实现。" },
      ] },
      { type: "paragraph", content: [
        { type: "mention", attrs: { kind: "skill", label: "review", ref: "E:/skills/review/SKILL.md" } },
      ] },
    ] };
    expect(resolveMentions(doc, "E:/fixture")).toBe('对照 README.md "docs/说明 文件.md" 检查实现。\n$review');
  });
});
