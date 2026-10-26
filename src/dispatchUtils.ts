export function hasPromptImages(doc: any): boolean {
  return doc?.type === "promptImage" || (Array.isArray(doc?.content) && doc.content.some(hasPromptImages));
}

/** Lexical only: references belong to the execution machine, not this WebView. */
export function workspaceRelativePath(file: string, root: string): string {
  const normalize = (value: string) => {
    let path = value.replace(/\\/g, "/");
    if (path.startsWith("//?/UNC/")) path = "//" + path.slice(8);
    else if (path.startsWith("//?/")) path = path.slice(4);
    const prefix = path.startsWith("//") ? "//" : path.startsWith("/") ? "/" : "";
    const parts: string[] = [];
    for (const part of path.slice(prefix.length).split("/")) {
      if (!part || part === ".") continue;
      if (part === ".." && parts.length && parts[parts.length - 1] !== ".." && !parts[parts.length - 1].endsWith(":")) parts.pop();
      else parts.push(part);
    }
    return prefix + parts.join("/");
  };
  const path = normalize(file);
  const base = normalize(root).replace(/\/$/, "");
  const windows = /^[a-z]:\//i.test(path) || path.startsWith("//");
  const comparable = (value: string) => windows ? value.replace(/[A-Z]/g, letter => letter.toLowerCase()) : value;
  // Preserve out-of-workspace / legacy references rather than inventing a path.
  if (root && comparable(path).startsWith(comparable(base) + "/")) return path.slice(base.length + 1);
  return path;
}

export function fileReference(ref: string, rootPath: string): string {
  const path = workspaceRelativePath(ref, rootPath);
  // Codex CLI insert_selected_path: quote whitespace unless a quote is already present.
  // Unicode White_Space matches Rust char::is_whitespace (unlike JavaScript \s).
  return /\p{White_Space}/u.test(path) && !path.includes('"') ? `"${path}"` : path;
}

/** Serialize references without reading file or skill bodies. */
export function resolveMentions(doc: any, rootPath: string): string {
  const parts: string[] = [];
  function walk(node: any) {
    if (node.type === "text") {
      parts.push(node.text);
    } else if (node.type === "promptImage") {
      throw new Error("image.owner"); // Image documents are resolved and pinned by the backend.
    } else if (node.type === "mention") {
      const kind = node.attrs.kind;
      const ref = node.attrs.ref;
      const label = node.attrs.label;
      parts.push(kind === "file" ? fileReference(ref, rootPath) : `$${label}`);
    } else if (node.content) {
      for (const child of node.content) walk(child);
    }
    if (node.type === "paragraph" || node.type === "heading") {
      parts.push("\n");
    }
  }
  walk(doc);
  return parts.join("").trim();
}
