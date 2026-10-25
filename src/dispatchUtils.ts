import * as api from "./api";

export function hasPromptImages(doc: any): boolean {
  return doc?.type === "promptImage" || (Array.isArray(doc?.content) && doc.content.some(hasPromptImages));
}

/** Expand file references, preserving native $skill invocations. */
export async function resolveMentions(doc: any, _rootPath: string): Promise<string> {
  const parts: string[] = [];
  async function walk(node: any) {
    if (node.type === "text") {
      parts.push(node.text);
    } else if (node.type === "promptImage") {
      throw new Error("image.owner"); // Image documents are resolved and pinned by the backend.
    } else if (node.type === "mention") {
      const kind = node.attrs.kind;
      const ref = node.attrs.ref;
      const label = node.attrs.label;
      try {
        if (kind === "file") {
          const content = await api.readFileContent(ref);
          parts.push(`\n--- @文件: ${label} (${ref}) ---\n\`\`\`\n${content}\n\`\`\`\n`);
        } else {
          parts.push(`$${label}`);
        }
      } catch (e) {
        parts.push(`\n[引用失败: ${label} — ${String(e)}]\n`);
      }
    } else if (node.content) {
      for (const child of node.content) await walk(child);
    }
    if (node.type === "paragraph" || node.type === "heading") {
      parts.push("\n");
    }
  }
  await walk(doc);
  return parts.join("").trim();
}
