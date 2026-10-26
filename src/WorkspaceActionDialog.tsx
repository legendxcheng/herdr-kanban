import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { tr, useLocale } from "./i18n";

export function WorkspaceActionDialog({ action, name, machine, opener, onCancel, onConfirm }: {
  action: "hide" | "delete";
  name: string;
  machine: string;
  opener: HTMLElement;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  useLocale();
  const id = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const root = document.getElementById("root");
    const wasInert = root?.inert ?? false;
    if (root) root.inert = true;
    cancel.current?.focus();
    return () => {
      if (root) root.inert = wasInert;
      if (opener.isConnected) opener.focus();
    };
  }, [opener]);

  return createPortal(<div className="dialog-overlay" onMouseDown={event => {
    if (event.target === event.currentTarget) onCancel();
  }}>
    <section className="dialog workspace-action-dialog" role="dialog" aria-modal="true"
      aria-labelledby={`${id}-title`} aria-describedby={`${id}-target ${id}-message`}
      onKeyDown={event => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onCancel(); }
        if (event.key === "Tab") {
          const buttons = event.currentTarget.querySelectorAll<HTMLButtonElement>("button");
          const first = buttons[0], last = buttons[buttons.length - 1];
          if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
          else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
        }
      }}>
      <h3 id={`${id}-title`}>{tr(`workspace.${action}_title`)}</h3>
      <p id={`${id}-target`} className="workspace-action-target">{machine} · {name}</p>
      <p id={`${id}-message`}>{tr(`workspace.${action}_message`)}</p>
      <div className="dialog-actions">
        <button ref={cancel} className="btn-secondary" onClick={onCancel}>{tr("common.cancel")}</button>
        <button className={action === "delete" ? "btn-primary workspace-action-delete" : "btn-primary"}
          onClick={onConfirm}>{tr(`workspace.${action}_confirm`)}</button>
      </div>
    </section>
  </div>, document.body);
}
