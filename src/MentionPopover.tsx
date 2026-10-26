import { useCurrentEditor } from "@tiptap/react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import * as api from "./api";
import type { FileEntry, SkillEntry } from "./types";
import { errorText, tr, useLocale } from "./i18n";
import "./mentionSearch.css";

interface Suggestion {
  kind: "file" | "skill";
  label: string;
  ref: string;
  description?: string;
}

const mentionTrigger = /([@$])([\p{L}\p{N}_./\\-]*)$/u;

/**
 * Mention popover controller. Renders a floating suggestion list when the
 * editor has an active @ or $ trigger, and inserts a mention node on select.
 */
export function MentionPopover({ rootPath, projectId }: { rootPath: string; projectId?: string }) {
  const locale = useLocale();
  const { editor } = useCurrentEditor();
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<Suggestion[]>([]);
  const [index, setIndex] = useState(0);
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<"file" | "skill">("file");
  const boxRef = useRef<HTMLDivElement>(null);
  const revealSelection = useRef(false);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  // Serialize remote searches per project in this editor. Superseded queued
  // queries are skipped; a stalled computer cannot hold up another project.
  const remoteSearches = useRef(new Map<string, Promise<void>>());

  useLayoutEffect(() => {
    if (boxRef.current) boxRef.current.scrollTop = 0;
  }, [items]);

  useLayoutEffect(() => {
    if (!open) { revealSelection.current = false; return; }
    if (!revealSelection.current) return;
    revealSelection.current = false;
    const box = boxRef.current;
    const selected = box?.children[index] as HTMLElement | undefined;
    if (!box || !selected) return;
    // Scroll only the list, not the editor/form ancestors. Tall entries start at
    // their heading so the skill remains identifiable before reading its text.
    const viewportTop = box.getBoundingClientRect().top + box.clientTop;
    const entry = selected.getBoundingClientRect();
    if (entry.top < viewportTop || entry.height > box.clientHeight) {
      box.scrollTop += entry.top - viewportTop;
    } else if (entry.bottom > viewportTop + box.clientHeight) {
      box.scrollTop += entry.bottom - viewportTop - box.clientHeight;
    }
  }, [index, open, items]);

  useLayoutEffect(() => {
    if (!editor || !open) return;
    const positionAtCursor = () => {
      const box = boxRef.current;
      if (!box || editor.isDestroyed) return;
      const cursor = editor.view.coordsAtPos(editor.state.selection.from);
      const size = box.getBoundingClientRect();
      const left = Math.max(8, Math.min(cursor.left, window.innerWidth - size.width - 8));
      const below = cursor.bottom + 4;
      const top = Math.max(8, Math.min(
        below + size.height <= window.innerHeight - 8 ? below : cursor.top - size.height - 4,
        window.innerHeight - size.height - 8,
      ));
      setPosition({ left, top });
    };
    positionAtCursor();
    editor.on("selectionUpdate", positionAtCursor);
    editor.on("update", positionAtCursor);
    window.addEventListener("resize", positionAtCursor);
    window.addEventListener("scroll", positionAtCursor, true);
    return () => {
      editor.off("selectionUpdate", positionAtCursor);
      editor.off("update", positionAtCursor);
      window.removeEventListener("resize", positionAtCursor);
      window.removeEventListener("scroll", positionAtCursor, true);
    };
  }, [editor, open, items, status, error, locale]);

  // Watch editor selection for trigger chars
  useEffect(() => {
    if (!editor) return;
    const handler = () => {
      const sel = editor.state.selection;
      const $pos = sel.$from;
      const textBefore = $pos.parent.textContent.slice(0, $pos.parentOffset);
      // find last @ or $ not followed by space
      const m = textBefore.match(mentionTrigger);
      if (m) {
        const ch = m[1];
        const q = m[2];
        setKind(ch === "@" ? "file" : "skill");
        setQuery(q);
        setOpen(true);
      } else {
        setOpen(false);
      }
    };
    editor.on("selectionUpdate", handler);
    editor.on("update", handler);
    return () => {
      editor.off("selectionUpdate", handler);
      editor.off("update", handler);
    };
  }, [editor]);

  // Fetch suggestions when query/kind changes
  useEffect(() => {
    setItems([]);
    setIndex(0);
    if (!open) return;
    setStatus("loading");
    setError("");
    let cancelled = false;
    const search = async () => {
      if (cancelled) return;
      try {
        let suggestions: Suggestion[] = [];
        if (kind === "file") {
          if (!rootPath.trim()) throw new Error("mention.directory_required");
          const files = await api.searchFiles(rootPath, query, projectId);
          suggestions = files.map((f: FileEntry) => ({
            kind: "file" as const,
            label: f.rel_path,
            ref: f.abs_path,
          }));
        } else {
          const skills = await api.searchSkills(rootPath, query, projectId);
          suggestions = skills.map((s: SkillEntry) => ({
            kind: "skill" as const,
            label: s.name,
            ref: s.skill_md_path,
            description: s.description,
          }));
        }
        if (!cancelled) {
          setItems(suggestions);
          setIndex(0);
          setStatus("ready");
        }
      } catch (e) {
        if (!cancelled) {
          setItems([]);
          setError(String(e));
          setStatus("error");
        }
      }
    };
    const schedule = () => {
      if (!projectId) { void search(); return; }
      const previous = remoteSearches.current.get(projectId) ?? Promise.resolve();
      const next = previous.then(search);
      remoteSearches.current.set(projectId, next);
      void next.finally(() => {
        if (remoteSearches.current.get(projectId) === next) remoteSearches.current.delete(projectId);
      });
    };
    // Local skill lookup stays immediate. Remote @ and $ both combine typing.
    const timer = kind === "file" || projectId ? window.setTimeout(schedule, 120) : undefined;
    if (kind === "skill" && !projectId) schedule();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [open, query, kind, rootPath, projectId, attempt]);

  // Keyboard handling inside editor
  useEffect(() => {
    if (!editor || !open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
        return;
      }
      if (items.length === 0) return;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        e.stopPropagation();
        revealSelection.current = true;
        setIndex((i) => (i + 1) % items.length);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        revealSelection.current = true;
        setIndex((i) => (i - 1 + items.length) % items.length);
      } else if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        e.stopPropagation();
        insertMention(items[index]);
      }
    };
    // Consume suggestion keys before ProseMirror turns Enter into a new paragraph.
    editor.view.dom.addEventListener("keydown", handler, true);
    return () => editor.view.dom.removeEventListener("keydown", handler, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, open, items, index]);

  function insertMention(s: Suggestion) {
    if (!editor) return;
    const { state, dispatch } = editor.view;
    const sel = state.selection;
    const $pos = sel.$from;
    const textBefore = $pos.parent.textContent.slice(0, $pos.parentOffset);
    const m = textBefore.match(mentionTrigger);
    if (!m) return;
    const start = sel.from - (m[0].length);
    // Replace the trigger+query with a mention node
    const mentionNode = state.schema.nodes.mention.create({
      kind: s.kind,
      label: s.label,
      ref: s.ref,
    });
    const tr = state.tr.deleteRange(start, sel.to).insert(start, mentionNode).insert(start + mentionNode.nodeSize, state.schema.text(" "));
    dispatch(tr.scrollIntoView());
    setOpen(false);
    editor.view.focus();
  }

  if (!open) return null;

  return createPortal(
    <div ref={boxRef} className="mention-popover" style={position}>
      {status === "loading" && <div className="mention-feedback" role="status">{tr(projectId ? "mention.searching_remote" : "mention.searching")}</div>}
      {status === "ready" && items.length === 0 && <div className="mention-feedback" role="status">{tr(kind === "file" ? "mention.no_files" : "mention.no_skills")}</div>}
      {status === "error" && <div className="mention-feedback">
        <div role="alert"><strong>{tr("mention.failed")}</strong><div className="mention-error">{errorText(error)}</div></div>
        <button type="button" className="btn-secondary" onMouseDown={e => e.preventDefault()} onClick={() => { setAttempt(value => value + 1); editor?.view.focus(); }}>{tr("mention.retry")}</button>
      </div>}
      {items.map((s, i) => (
        <div
          key={s.ref}
          className={"mention-item" + (i === index ? " active" : "")}
          onMouseDown={(e) => { e.preventDefault(); insertMention(s); }}
          // Scrolling can move rows under a stationary pointer. Only actual
          // pointer movement should override the keyboard-selected candidate.
          onMouseMove={() => { revealSelection.current = false; setIndex(i); }}
        >
          <span className="mention-kind">{s.kind === "file" ? "@" : "$"}</span>
          <span className="mention-content">
            <span className="mention-label">{s.label}</span>
            {s.description && <span className="mention-desc">{s.description}</span>}
          </span>
        </div>
      ))}
    </div>, document.body,
  );
}
