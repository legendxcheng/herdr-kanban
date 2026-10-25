import { Node, mergeAttributes } from "@tiptap/core";
import { NodeViewWrapper, ReactNodeViewRenderer, type NodeViewProps } from "@tiptap/react";
import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import { tr, useLocalizedMessage } from "./i18n";
import { closeHistory } from "@tiptap/pm/history";
import { ImagePreview } from "./ImagePreview";

function ImageView({ node, extension, editor, deleteNode, selected }: NodeViewProps) {
  const [source, setSource] = useState("");
  const [error, setError] = useLocalizedMessage();
  const [expanded, setExpanded] = useState(false);
  const thumbnail = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const draftId = await extension.options.getDraftId();
        const preview = await api.previewPromptImage(draftId, node.attrs.imageId);
        if (active) setSource(preview);
      } catch (reason) { if (active) setError(String(reason)); }
    })();
    return () => { active = false; };
  }, [extension, node.attrs.imageId]);
  return <NodeViewWrapper as="figure" className={`prompt-image${selected ? " selected" : ""}`} contentEditable={false}>
    <button ref={thumbnail} type="button" className="image-thumbnail" disabled={!source}
      aria-label={`${tr("image.enlarge")}: ${node.attrs.name}`} title={node.attrs.name} onClick={() => setExpanded(true)}>
      {source ? <img src={source} alt={node.attrs.name} draggable={false} /> : <span role={error ? "alert" : "status"}>{error || tr("image.loading")}</span>}
    </button>
    {editor.isEditable && <button type="button" className="prompt-image-remove" title={tr("image.remove")}
      aria-label={`${tr("image.remove")}: ${node.attrs.name}`} onClick={() => {
        if (!editor.isEditable) return;
        editor.view.dispatch(closeHistory(editor.state.tr)); deleteNode(); editor.view.focus();
      }}>×</button>}
    {expanded && source && <ImagePreview source={source} name={node.attrs.name} opener={thumbnail.current} onClose={() => setExpanded(false)} />}
  </NodeViewWrapper>;
}

export const PromptImageNode = Node.create<{ getDraftId: () => Promise<string> }>({
  name: "promptImage", group: "block", atom: true,
  addOptions: () => ({ getDraftId: async () => { throw new Error("image.draft_closed"); } }),
  addAttributes: () => ({ imageId: { default: "" }, name: { default: "" }, bytes: { default: 0 } }),
  parseHTML: () => [{ tag: "figure[data-prompt-image]" }],
  renderHTML: ({ HTMLAttributes }) => ["figure", mergeAttributes(HTMLAttributes, { "data-prompt-image": "true" })],
  renderText: ({ node }) => `[${node.attrs.name}]`,
  addNodeView: () => ReactNodeViewRenderer(ImageView),
});
