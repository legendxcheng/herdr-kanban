import { useEditor, EditorContent, EditorContext } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { MentionNode } from "./MentionNode";
import { MentionPopover } from "./MentionPopover";
import { useEffect } from "react";
import { tr, useLocale } from "./i18n";
import { PromptImageNode } from "./PromptImageNode";
import { ImagePaste } from "./imagePaste";
import "./promptImages.css";

interface PromptEditorProps {
  content: string; // Tiptap JSON string
  rootPath: string;
  onChange: (json: string) => void;
  labelledBy?: string;
  disabled?: boolean;
  autoFocus?: boolean;
  getDraftId?: () => Promise<string>;
  onImageBusy?: (busy: boolean) => void;
  onImageError?: (reason: unknown) => void;
}

export function PromptEditor({ content, rootPath, onChange, labelledBy, disabled = false, autoFocus = false, getDraftId = async () => { throw new Error("image.draft_closed"); }, onImageBusy = () => {}, onImageError = () => {} }: PromptEditorProps) {
  const locale = useLocale();
  const editor = useEditor({
    extensions: [
      StarterKit,
      Placeholder.configure({ placeholder: () => tr("task.prompt_placeholder") }),
      MentionNode,
      PromptImageNode.configure({ getDraftId }),
      ImagePaste.configure({ getDraftId, onBusy: onImageBusy, onError: onImageError }),
    ],
    content: (() => {
      try { const value = JSON.parse(content); return value?.type === "doc" ? value : null; } catch { return null; }
    })(),
    editable: !disabled,
    editorProps: { attributes: { role: "textbox", "aria-multiline": "true", ...(labelledBy ? { "aria-labelledby": labelledBy } : { "aria-label": tr("task.prompt") }) } },
    onUpdate({ editor }) {
      onChange(JSON.stringify(editor.getJSON()));
    },
  });

  // Refresh decorations and accessibility text without replacing the editor,
  // its document, selection, image draft, or undo history.
  useEffect(() => {
    if (!editor || editor.isDestroyed) return;
    editor.setOptions({ editorProps: { attributes: { role: "textbox", "aria-multiline": "true",
      ...(labelledBy ? { "aria-labelledby": labelledBy } : { "aria-label": tr("task.prompt") }) } } });
    editor.view.dispatch(editor.state.tr.setMeta("addToHistory", false));
  }, [editor, locale, labelledBy]);
  useEffect(() => { editor?.setEditable(!disabled); }, [editor, disabled]);
  useEffect(() => { if (autoFocus) editor?.commands.focus("end"); }, [editor, autoFocus]);

  return (
    <EditorContext.Provider value={{ editor }}>
      <div className="prompt-editor-wrap">
        <EditorContent editor={editor} className="prompt-editor" />
        <p className="image-paste-hint">{tr("image.hint")}</p>
        {!disabled && <MentionPopover rootPath={rootPath} />}
      </div>
    </EditorContext.Provider>
  );
}
