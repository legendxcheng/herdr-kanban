import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import * as api from "./api";
import { tr } from "./i18n";

const uploads = new PluginKey<DecorationSet>("prompt-image-uploads");
const encode = (file: File) => new Promise<string>((resolve, reject) => {
  const reader = new FileReader();
  reader.onload = () => resolve(String(reader.result).split(",")[1]);
  reader.onerror = () => reject(new Error("image.storage"));
  reader.readAsDataURL(file);
});

interface Options { getDraftId: () => Promise<string>; onBusy: (busy: boolean) => void; onError: (reason: unknown) => void }
export const ImagePaste = Extension.create<Options>({
  name: "imagePaste",
  addProseMirrorPlugins() {
    const { editor, options } = this;
    let pending = 0;
    let pendingBytes = 0;
    let sequence = 0;
    let queue = Promise.resolve();
    const slot = (position: number, id: string, order: number) => Decoration.widget(position, () => {
      const span = document.createElement("span"); span.className = "image-importing"; span.textContent = tr("image.importing"); return span;
    }, { id, order, side: -1 });
    return [new Plugin({ key: uploads,
      state: {
        init: () => DecorationSet.empty,
        apply(transaction, previous) {
          let next = previous.map(transaction.mapping, transaction.doc);
          const action = transaction.getMeta(uploads);
          if (action?.add) next = next.add(transaction.doc, action.add);
          if (action?.remove) next = next.remove(next.find().filter(item => item.spec.id === action.remove));
          if (action?.complete) {
            const completed = action.complete;
            const following = previous.find().filter(item => item.from === completed.position && item.spec.order > completed.order);
            next = next.remove(next.find().filter(item => item.spec.id === completed.id || following.some(old => old.spec.id === item.spec.id)));
            next = next.add(transaction.doc, following.map(item => slot(completed.after, item.spec.id, item.spec.order)));
          }
          return next;
        },
      },
      props: {
        decorations: state => uploads.getState(state),
        handlePaste(view, event) {
          if (!editor.isEditable) return false;
          const data = event.clipboardData;
          if (!data) return false;
          const items = Array.from(data.items ?? []).filter(item => item.kind === "file" && item.type.startsWith("image/"));
          let files = items.length ? items.map(item => item.getAsFile()).filter((file): file is File => !!file)
            : Array.from(data.files ?? []).filter(file => file.type.startsWith("image/"));
          // A ClipboardItem can expose several media representations of one image.
          // A file collection (types: Files) still keeps its distinct images.
          const mediaTypes = Array.from(data.types ?? []).filter(type => type.startsWith("image/"));
          const htmlImages = data.getData("text/html").match(/<img\b/gi)?.length ?? 0;
          if (mediaTypes.length > 1 && htmlImages <= 1 && new Set(files.map(file => file.type)).size === files.length && files.every(file => mediaTypes.includes(file.type))) {
            const preferred = ["image/png", "image/webp", "image/jpeg"].map(type => files.find(file => file.type === type)).find(Boolean);
            if (preferred) files = [preferred];
          }
          if (htmlImages <= 1) {
            const sources = new Map<string, File[]>();
            for (const file of files) {
              const source = file.name.replace(/\.(png|jpe?g|webp|bmp)$/i, "").toLowerCase();
              sources.set(source, [...(sources.get(source) ?? []), file]);
            }
            const selected = new Set(files);
            for (const group of sources.values()) {
              if (group.length < 2 || new Set(group.map(file => file.type)).size !== group.length) continue;
              // File MIME representations share their logical source name; independent
              // files retain distinct names, even when their MIME types differ.
              const preferred = ["image/png", "image/webp", "image/jpeg"].map(type => group.find(file => file.type === type)).find(Boolean);
              if (preferred) for (const file of group) if (file !== preferred) selected.delete(file);
            }
            files = files.filter(file => selected.has(file));
          }
          if (!files.length) return false;
          event.preventDefault();
          const text = data.getData("text/plain");
          if (text) editor.commands.insertContent(text.split(/\r?\n/).map(line => ({ type: "paragraph", content: line ? [{ type: "text", text: line }] : [] })));
          let count = 0; let bytes = 0;
          view.state.doc.descendants(node => { if (node.type.name === "promptImage") { count++; bytes += Number(node.attrs.bytes) || 0; } });
          const incomingBytes = files.reduce((sum, file) => sum + file.size, 0);
          if (count + pending + files.length > 20 || bytes + pendingBytes + incomingBytes > 50 * 1024 * 1024) {
            options.onError(new Error("image.limit")); return true;
          }
          const position = view.state.selection.from;
          const jobs = files.map(file => ({ file, id: crypto.randomUUID(), order: ++sequence }));
          const decorations = jobs.map(job => slot(position, job.id, job.order));
          view.dispatch(view.state.tr.setMeta(uploads, { add: decorations }));
          pending += jobs.length; pendingBytes += incomingBytes; options.onBusy(true);
          for (const job of jobs) queue = queue.then(async () => {
            try {
              if (editor.isDestroyed) return;
              if (job.file.size > 10 * 1024 * 1024) throw new Error("image.limit");
              const draftId = await options.getDraftId();
              const image = await api.importPromptImage(draftId, job.file.name || tr("image.default_name"), job.file.type, await encode(job.file));
              if (editor.isDestroyed) return;
              const placeholder = uploads.getState(view.state)?.find().find(item => item.spec.id === job.id);
              if (!placeholder) return;
              editor.chain().insertContentAt(placeholder.from, { type: "promptImage", attrs: { imageId: image.id, name: image.name, bytes: image.bytes } }, { updateSelection: false })
                .command(({ tr }) => {
                  let after = placeholder.from;
                  tr.doc.descendants((node, position) => { if (node.type.name === "promptImage" && node.attrs.imageId === image.id) after = position + node.nodeSize; });
                  tr.setMeta(uploads, { complete: { id: job.id, order: job.order, position: placeholder.from, after } });
                  return true;
                }).run();
            } catch (reason) { if (!editor.isDestroyed) options.onError(reason); }
            finally {
              if (!editor.isDestroyed) {
                view.dispatch(view.state.tr.setMeta(uploads, { remove: job.id }));
                pending--; pendingBytes -= job.file.size; options.onBusy(pending > 0);
              }
            }
          });
          return true;
        },
      },
    })];
  },
});
