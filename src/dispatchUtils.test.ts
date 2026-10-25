import { describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { resolveMentions } from "./dispatchUtils";

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

  it("still expands file content alongside a native skill invocation", async () => {
    const doc = { type: "doc", content: [{ type: "paragraph", content: [
      { type: "mention", attrs: { kind: "file", label: "plan.md", ref: "E:/fixture/plan.md" } },
      { type: "mention", attrs: { kind: "skill", label: "to-spec", ref: "C:/skills/to-spec/SKILL.md" } },
    ] }] };
    const prompt = await resolveMentions(doc, "E:/fixture");
    expect(prompt).toContain("file body");
    expect(prompt).toContain("$to-spec");
    expect(prompt).not.toContain("SKILL BODY");
  });
});
