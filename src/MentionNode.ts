import { Node, mergeAttributes } from "@tiptap/core";

/**
 * Custom mention node for @ (file) and $ (skill) references.
 * Stored as an inline node with attrs: kind, label, ref.
 * Renders as a pill chip in the editor.
 */
export const MentionNode = Node.create({
  name: "mention",
  group: "inline",
  inline: true,
  atom: true,

  addAttributes() {
    return {
      kind:  { default: "file" },
      label: { default: "" },
      ref:   { default: "" },
    };
  },

  parseHTML() {
    return [{ tag: 'span[data-mention]' }];
  },

  renderHTML({ HTMLAttributes }) {
    const { kind, label } = HTMLAttributes;
    return ["span", mergeAttributes(HTMLAttributes, {
      "data-mention": kind,
      class: `mention-chip mention-${kind}`,
      contenteditable: "false",
    }), `${kind === "file" ? "@" : "$"}${label}`];
  },

  renderText({ node }) {
    // Plain text used when extracting prompt for dispatch preview
    return `${node.attrs.kind === "file" ? "@" : "$"}${node.attrs.label}`;
  },
});
